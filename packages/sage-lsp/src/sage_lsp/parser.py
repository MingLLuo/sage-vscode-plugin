from __future__ import annotations

import ast
import re
from pathlib import Path

from .model import ImportBinding, ModuleRecord, SourceRange, SymbolRecord


LAZY_IMPORT_NAMES = {"lazy_import", "_lazy_import"}
TRIPLE_QUOTE_RE = re.compile(r'^\s*(?P<quote>"""|\'\'\')(?P<body>.*?)(?P=quote)', re.DOTALL)
PYX_SYMBOL_RE = re.compile(
    r"^(?P<indent>\s*)(?:(?:cdef|cpdef)\s+)?(?:class|cdef\s+class|def)\s+(?P<name>[A-Za-z_][A-Za-z0-9_]*)",
    re.MULTILINE,
)
PYX_ASSIGN_RE = re.compile(r"^(?P<indent>\s*)(?P<name>[A-Z][A-Za-z0-9_]*)\s*=", re.MULTILINE)
LOOSE_DEF_RE = re.compile(r"^(?:async\s+def|def)\s+(?P<name>[A-Za-z_][A-Za-z0-9_]*)\s*\(")
LOOSE_CLASS_RE = re.compile(r"^class\s+(?P<name>[A-Za-z_][A-Za-z0-9_]*)\b")
LOOSE_IMPORT_RE = re.compile(r"^import\s+(?P<module>[A-Za-z0-9_\.]+)(?:\s+as\s+(?P<alias>[A-Za-z_][A-Za-z0-9_]*))?")
LOOSE_FROM_IMPORT_RE = re.compile(r"^from\s+(?P<module>[A-Za-z0-9_\.]+)\s+import\s+(?P<names>.+)$")
LOOSE_ASSIGN_RE = re.compile(r"^(?P<name>[A-Za-z_][A-Za-z0-9_]*)\s*=")
LOOSE_PREPARSE_ASSIGN_RE = re.compile(
    r"^(?P<parent>[A-Za-z_][A-Za-z0-9_]*)\.<(?P<symbol>[A-Za-z_][A-Za-z0-9_]*)>\s*="
)


def parse_module(module_name: str, file_path: Path, source: str) -> ModuleRecord:
    if file_path.suffix == ".pyx":
        return parse_pyx_module(module_name, file_path, source)
    if file_path.suffix == ".sage":
        return parse_loose_module(module_name, file_path, source)
    return parse_python_module(module_name, file_path, source)


def parse_python_module(module_name: str, file_path: Path, source: str) -> ModuleRecord:
    record = ModuleRecord(module_name=module_name, file_path=file_path, language="python", source=source)
    try:
        tree = ast.parse(source, filename=str(file_path))
    except SyntaxError:
        return parse_loose_module(module_name, file_path, source)

    record.docstring = ast.get_docstring(tree)

    for node in tree.body:
        if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)):
            record.symbols[node.name] = SymbolRecord(
                name=node.name,
                kind="function",
                module_name=module_name,
                file_path=file_path,
                source_range=node_range(node),
                detail=f"function {node.name}",
                docstring=ast.get_docstring(node),
            )
            continue

        if isinstance(node, ast.ClassDef):
            record.symbols[node.name] = SymbolRecord(
                name=node.name,
                kind="class",
                module_name=module_name,
                file_path=file_path,
                source_range=node_range(node),
                detail=f"class {node.name}",
                docstring=ast.get_docstring(node),
            )
            continue

        if isinstance(node, ast.Assign):
            for target in node.targets:
                if isinstance(target, ast.Name):
                    record.symbols[target.id] = SymbolRecord(
                        name=target.id,
                        kind="constant" if target.id.isupper() else "variable",
                        module_name=module_name,
                        file_path=file_path,
                        source_range=node_range(target),
                        detail=f"variable {target.id}",
                    )
            continue

        if isinstance(node, ast.AnnAssign) and isinstance(node.target, ast.Name):
            target = node.target
            record.symbols[target.id] = SymbolRecord(
                name=target.id,
                kind="constant" if target.id.isupper() else "variable",
                module_name=module_name,
                file_path=file_path,
                source_range=node_range(target),
                detail=f"variable {target.id}",
            )
            continue

        if isinstance(node, ast.ImportFrom):
            imported_module = resolve_imported_module(module_name, node.module, node.level)
            if imported_module is None:
                continue
            for alias in node.names:
                if alias.name == "*":
                    record.star_imports.append(imported_module)
                    continue
                imported_name = alias.asname or alias.name
                record.bindings[imported_name] = ImportBinding(
                    alias=imported_name,
                    module_name=imported_module,
                    target_name=alias.name,
                    source_range=node_range(node),
                )
            continue

        if isinstance(node, ast.Import):
            for alias in node.names:
                imported_name = alias.asname or alias.name.split(".")[-1]
                record.bindings[imported_name] = ImportBinding(
                    alias=imported_name,
                    module_name=alias.name,
                    target_name=None,
                    source_range=node_range(node),
                )
            continue

        binding = parse_lazy_import_statement(module_name, node)
        if binding:
            for item in binding:
                record.bindings[item.alias] = item

    return record


def parse_loose_module(module_name: str, file_path: Path, source: str) -> ModuleRecord:
    record = ModuleRecord(module_name=module_name, file_path=file_path, language="python", source=source)
    doc_match = TRIPLE_QUOTE_RE.search(source)
    if doc_match:
        record.docstring = doc_match.group("body").strip()

    for line_number, raw_line in enumerate(source.splitlines(), start=1):
        line = raw_line.strip()
        if not line or line.startswith("#"):
            continue

        if binding := parse_lazy_import_line(line, line_number):
            for item in binding:
                record.bindings[item.alias] = item
            continue

        if match := LOOSE_FROM_IMPORT_RE.match(line):
            imported_module = match.group("module")
            names = match.group("names").strip()
            if names == "*":
                record.star_imports.append(imported_module)
                continue
            for item in names.split(","):
                entry = item.strip()
                if not entry:
                    continue
                alias_name = entry
                target_name = entry
                if " as " in entry:
                    target_name, alias_name = [part.strip() for part in entry.split(" as ", maxsplit=1)]
                record.bindings[alias_name] = ImportBinding(
                    alias=alias_name,
                    module_name=imported_module,
                    target_name=target_name,
                    source_range=SourceRange.from_offsets(line_number, raw_line.find(alias_name), line_number, raw_line.find(alias_name) + len(alias_name)),
                )
            continue

        if match := LOOSE_IMPORT_RE.match(line):
            imported_module = match.group("module")
            alias = match.group("alias") or imported_module.split(".")[-1]
            record.bindings[alias] = ImportBinding(
                alias=alias,
                module_name=imported_module,
                target_name=None,
                source_range=SourceRange.from_offsets(line_number, raw_line.find(alias), line_number, raw_line.find(alias) + len(alias)),
            )
            continue

        if match := LOOSE_DEF_RE.match(line):
            name = match.group("name")
            record.symbols[name] = SymbolRecord(
                name=name,
                kind="function",
                module_name=module_name,
                file_path=file_path,
                source_range=SourceRange.from_offsets(line_number, raw_line.find(name), line_number, raw_line.find(name) + len(name)),
                detail=f"function {name}",
            )
            continue

        if match := LOOSE_CLASS_RE.match(line):
            name = match.group("name")
            record.symbols[name] = SymbolRecord(
                name=name,
                kind="class",
                module_name=module_name,
                file_path=file_path,
                source_range=SourceRange.from_offsets(line_number, raw_line.find(name), line_number, raw_line.find(name) + len(name)),
                detail=f"class {name}",
            )
            continue

        if match := LOOSE_PREPARSE_ASSIGN_RE.match(line):
            parent = match.group("parent")
            symbol = match.group("symbol")
            for name in (parent, symbol):
                record.symbols[name] = SymbolRecord(
                    name=name,
                    kind="variable",
                    module_name=module_name,
                    file_path=file_path,
                    source_range=SourceRange.from_offsets(line_number, raw_line.find(name), line_number, raw_line.find(name) + len(name)),
                    detail=f"variable {name}",
                )
            continue

        if match := LOOSE_ASSIGN_RE.match(line):
            name = match.group("name")
            record.symbols[name] = SymbolRecord(
                name=name,
                kind="constant" if name.isupper() else "variable",
                module_name=module_name,
                file_path=file_path,
                source_range=SourceRange.from_offsets(line_number, raw_line.find(name), line_number, raw_line.find(name) + len(name)),
                detail=f"variable {name}",
            )

    return record


def parse_pyx_module(module_name: str, file_path: Path, source: str) -> ModuleRecord:
    record = ModuleRecord(module_name=module_name, file_path=file_path, language="pyx", source=source)
    doc_match = TRIPLE_QUOTE_RE.search(source)
    if doc_match:
        record.docstring = doc_match.group("body").strip()

    for match in PYX_SYMBOL_RE.finditer(source):
        line = source.count("\n", 0, match.start()) + 1
        line_start = source.rfind("\n", 0, match.start()) + 1
        column = match.start("name") - line_start
        name = match.group("name")
        prefix = source[match.start() : match.start("name")]
        kind = "class" if "class" in prefix else "function"
        record.symbols[name] = SymbolRecord(
            name=name,
            kind=kind,
            module_name=module_name,
            file_path=file_path,
            source_range=SourceRange.from_offsets(line, column, line, column + len(name)),
            detail=f"{kind} {name}",
        )

    for match in PYX_ASSIGN_RE.finditer(source):
        line = source.count("\n", 0, match.start()) + 1
        line_start = source.rfind("\n", 0, match.start()) + 1
        column = match.start("name") - line_start
        name = match.group("name")
        record.symbols.setdefault(
            name,
            SymbolRecord(
                name=name,
                kind="constant",
                module_name=module_name,
                file_path=file_path,
                source_range=SourceRange.from_offsets(line, column, line, column + len(name)),
                detail=f"constant {name}",
            ),
        )

    return record


def parse_lazy_import_statement(module_name: str, node: ast.stmt) -> list[ImportBinding]:
    if not isinstance(node, ast.Expr) or not isinstance(node.value, ast.Call):
        return []
    call = node.value
    if not is_lazy_import_call(call):
        return []
    if not call.args or not isinstance(call.args[0], ast.Constant) or not isinstance(call.args[0].value, str):
        return []

    imported_module = str(call.args[0].value)
    names = parse_string_or_list(call.args[1] if len(call.args) > 1 else None)
    aliases = parse_aliases(call.args[2] if len(call.args) > 2 else None)

    for keyword in call.keywords:
        if keyword.arg in {"as_", "as_name"}:
            aliases = parse_aliases(keyword.value)

    if not names:
        return []

    if not aliases:
        aliases = names

    bindings: list[ImportBinding] = []
    for index, name in enumerate(names):
        alias = aliases[index] if index < len(aliases) else name
        bindings.append(
            ImportBinding(
                alias=alias,
                module_name=imported_module,
                target_name=name,
                source_range=node_range(node),
                is_lazy=True,
            )
        )
    return bindings


def parse_lazy_import_line(line: str, line_number: int) -> list[ImportBinding]:
    if "lazy_import(" not in line:
        return []
    module_match = re.search(r"""lazy_import\(\s*["']([^"']+)["']""", line)
    names_match = re.findall(r"""["']([^"']+)["']""", line)
    if module_match is None or len(names_match) < 2:
        return []
    imported_module = module_match.group(1)
    raw_names = names_match[1:]
    if len(raw_names) >= 4 and len(raw_names) % 2 == 1:
        midpoint = len(raw_names) // 2
        names = raw_names[: midpoint + 1]
        aliases = raw_names[midpoint + 1 :]
    else:
        names = raw_names
        aliases = raw_names
    bindings: list[ImportBinding] = []
    for index, name in enumerate(names):
        alias = aliases[index] if index < len(aliases) else name
        bindings.append(
            ImportBinding(
                alias=alias,
                module_name=imported_module,
                target_name=name,
                source_range=SourceRange.from_offsets(line_number, max(0, line.find(alias)), line_number, max(0, line.find(alias)) + len(alias)),
                is_lazy=True,
            )
        )
    return bindings


def parse_string_or_list(value: ast.AST | None) -> list[str]:
    if isinstance(value, ast.Constant) and isinstance(value.value, str):
        return [value.value]
    if isinstance(value, (ast.List, ast.Tuple)):
        items: list[str] = []
        for item in value.elts:
            if isinstance(item, ast.Constant) and isinstance(item.value, str):
                items.append(item.value)
        return items
    return []


def parse_aliases(value: ast.AST | None) -> list[str]:
    return parse_string_or_list(value)


def is_lazy_import_call(call: ast.Call) -> bool:
    if isinstance(call.func, ast.Name):
        return call.func.id in LAZY_IMPORT_NAMES
    if isinstance(call.func, ast.Attribute):
        return call.func.attr in LAZY_IMPORT_NAMES
    return False


def resolve_imported_module(module_name: str, raw_module: str | None, level: int) -> str | None:
    if level == 0:
        return raw_module

    pieces = module_name.split(".")
    if pieces:
        pieces = pieces[:-1]
    if level > 1:
        pieces = pieces[: -(level - 1)] if level - 1 <= len(pieces) else []
    if raw_module:
        pieces.extend(raw_module.split("."))
    return ".".join(piece for piece in pieces if piece) or None


def node_range(node: ast.AST) -> SourceRange:
    line = getattr(node, "lineno", 1)
    end_line = getattr(node, "end_lineno", line)
    column = getattr(node, "col_offset", 0)
    end_column = getattr(node, "end_col_offset", column)
    return SourceRange.from_offsets(line, column, end_line, end_column)
