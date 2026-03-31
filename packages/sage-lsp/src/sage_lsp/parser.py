from __future__ import annotations

import ast
import re
from pathlib import Path
from typing import Optional

from .model import ImportBinding, ModuleRecord, SourceRange, SymbolRecord
from .source_map import preprocess_sage_source


LAZY_IMPORT_NAMES = {"lazy_import", "_lazy_import"}
TRIPLE_QUOTE_RE = re.compile(r'^\s*(?P<quote>"""|\'\'\')(?P<body>.*?)(?P=quote)', re.DOTALL)
PYX_ASSIGN_RE = re.compile(r"^(?P<indent>\s*)(?P<name>[A-Z][A-Za-z0-9_]*)\s*=", re.MULTILINE)
CYTHON_CLASS_RE = re.compile(r"^(?:cdef\s+class|cpdef\s+class|class)\s+(?P<name>[A-Za-z_][A-Za-z0-9_]*)\b")
CYTHON_FROM_IMPORT_RE = re.compile(r"^from\s+(?P<module>[A-Za-z0-9_\.]+)\s+(?P<kind>cimport|import)\s+(?P<names>.+)$")
CYTHON_CIMPORT_RE = re.compile(r"^cimport\s+(?P<module>[A-Za-z0-9_\.]+)(?:\s+as\s+(?P<alias>[A-Za-z_][A-Za-z0-9_]*))?$")
LOOSE_DEF_RE = re.compile(r"^(?:async\s+def|def)\s+(?P<name>[A-Za-z_][A-Za-z0-9_]*)\s*\(")
LOOSE_CLASS_RE = re.compile(r"^class\s+(?P<name>[A-Za-z_][A-Za-z0-9_]*)\b")
LOOSE_IMPORT_RE = re.compile(r"^import\s+(?P<module>[A-Za-z0-9_\.]+)(?:\s+as\s+(?P<alias>[A-Za-z_][A-Za-z0-9_]*))?")
LOOSE_FROM_IMPORT_RE = re.compile(r"^from\s+(?P<module>[A-Za-z0-9_\.]+)\s+import\s+(?P<names>.+)$")
LOOSE_ASSIGN_RE = re.compile(r"^(?P<name>[A-Za-z_][A-Za-z0-9_]*)\s*=")
LOOSE_PREPARSE_ASSIGN_RE = re.compile(
    r"^(?P<parent>[A-Za-z_][A-Za-z0-9_]*)\.<(?P<symbols>[A-Za-z_][A-Za-z0-9_]*(?:\s*,\s*[A-Za-z_][A-Za-z0-9_]*)*)>\s*="
)
LOOSE_PREPARSE_VALIDATE_RE = re.compile(
    r"^(?P<indent>\s*)(?P<parent>[A-Za-z_][A-Za-z0-9_]*)\.<(?P<symbols>[^>]+)>(?P<spacing>\s*=)"
)


def parse_module(module_name: str, file_path: Path, source: str) -> ModuleRecord:
    if file_path.suffix in {".pyx", ".pxd", ".pxi"}:
        return parse_pyx_module(module_name, file_path, source)
    if file_path.suffix == ".sage":
        record = parse_loose_module(module_name, file_path, source)
        record.diagnostics.extend(syntax_diagnostics_for_source(file_path, source))
        return record
    return parse_python_module(module_name, file_path, source)


def parse_python_module(module_name: str, file_path: Path, source: str) -> ModuleRecord:
    record = ModuleRecord(module_name=module_name, file_path=file_path, language="python", source=source)
    try:
        tree = ast.parse(source, filename=str(file_path))
    except SyntaxError as error:
        record = parse_loose_module(module_name, file_path, source)
        record.diagnostics.extend(syntax_diagnostics_for_source(file_path, source, syntax_error=error))
        return record

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
            _collect_class_members(record, module_name, file_path, node)
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
                    instance_type = class_instance_target(record, node.value)
                    if instance_type is not None:
                        record.instance_types[target.id] = instance_type
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
            instance_type = class_instance_target(record, node.value)
            if instance_type is not None:
                record.instance_types[target.id] = instance_type
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


def _collect_class_members(
    record: ModuleRecord,
    module_name: str,
    file_path: Path,
    node: ast.ClassDef,
) -> None:
    owner_name = node.name
    member_symbols = record.member_symbols.setdefault(owner_name, {})
    member_bindings = record.member_bindings.setdefault(owner_name, {})
    class_bindings: dict[str, ImportBinding] = {}

    for item in node.body:
        if isinstance(item, ast.ImportFrom):
            imported_module = resolve_imported_module(module_name, item.module, item.level)
            if imported_module is None:
                continue
            for alias in item.names:
                if alias.name == "*":
                    continue
                imported_name = alias.asname or alias.name
                class_bindings[imported_name] = ImportBinding(
                    alias=imported_name,
                    module_name=imported_module,
                    target_name=alias.name,
                    source_range=node_range(item),
                )
            continue

        if isinstance(item, ast.Import):
            for alias in item.names:
                imported_name = alias.asname or alias.name.split(".")[-1]
                class_bindings[imported_name] = ImportBinding(
                    alias=imported_name,
                    module_name=alias.name,
                    target_name=None,
                    source_range=node_range(item),
                )
            continue

        if isinstance(item, (ast.FunctionDef, ast.AsyncFunctionDef)):
            member_symbols[item.name] = SymbolRecord(
                name=item.name,
                kind="function",
                module_name=module_name,
                file_path=file_path,
                source_range=node_range(item),
                detail=f"function {owner_name}.{item.name}",
                docstring=ast.get_docstring(item),
            )
            continue

        if isinstance(item, ast.Assign):
            for target in item.targets:
                if not isinstance(target, ast.Name):
                    continue
                binding = member_binding_from_expression(
                    record,
                    module_name,
                    target.id,
                    item.value,
                    node_range(target),
                    class_bindings,
                )
                if binding is not None:
                    member_bindings[target.id] = binding
                    continue
                member_symbols.setdefault(
                    target.id,
                    SymbolRecord(
                        name=target.id,
                        kind="constant" if target.id.isupper() else "variable",
                        module_name=module_name,
                        file_path=file_path,
                        source_range=node_range(target),
                        detail=f"attribute {owner_name}.{target.id}",
                    ),
                )
            continue

        if isinstance(item, ast.AnnAssign) and isinstance(item.target, ast.Name):
            target = item.target
            binding = member_binding_from_expression(
                record,
                module_name,
                target.id,
                item.value,
                node_range(target),
                class_bindings,
            )
            if binding is not None:
                member_bindings[target.id] = binding
                continue
            member_symbols.setdefault(
                target.id,
                SymbolRecord(
                    name=target.id,
                    kind="constant" if target.id.isupper() else "variable",
                    module_name=module_name,
                    file_path=file_path,
                    source_range=node_range(target),
                    detail=f"attribute {owner_name}.{target.id}",
                ),
            )


def member_binding_from_expression(
    record: ModuleRecord,
    module_name: str,
    alias: str,
    value: Optional[ast.AST],
    source_range: SourceRange,
    scope_bindings: Optional[dict[str, ImportBinding]] = None,
) -> Optional[ImportBinding]:
    dotted_reference = dotted_reference_from_expression(value)
    if dotted_reference is None:
        return None

    parts = dotted_reference.split(".")
    head = parts[0]
    binding = (scope_bindings or {}).get(head) or record.bindings.get(head)
    if binding is None:
        return None

    target_module_name = binding.module_name
    if binding.target_name is None:
        target_name = ".".join(parts[1:]) or None
    elif parts[1:] and binding.target_name[:1].islower():
        target_module_name = f"{binding.module_name}.{binding.target_name}"
        target_name = ".".join(parts[1:]) or None
    else:
        target_name = ".".join([binding.target_name, *parts[1:]]) if parts[1:] else binding.target_name

    return ImportBinding(
        alias=alias,
        module_name=target_module_name,
        target_name=target_name,
        source_range=source_range,
        is_lazy=binding.is_lazy,
    )


def class_instance_target(record: ModuleRecord, value: Optional[ast.AST]) -> Optional[str]:
    if not isinstance(value, ast.Call):
        return None

    target = dotted_reference_from_expression(value.func)
    if target is None:
        return None

    if target in record.symbols and record.symbols[target].kind == "class":
        return target
    return None


def dotted_reference_from_expression(value: Optional[ast.AST]) -> Optional[str]:
    if isinstance(value, ast.Call):
        if isinstance(value.func, ast.Name) and value.func.id in {"staticmethod", "classmethod"} and len(value.args) == 1:
            return dotted_reference_from_expression(value.args[0])
        return dotted_reference_from_expression(value.func)

    if isinstance(value, ast.Name):
        return value.id

    if isinstance(value, ast.Attribute):
        parent = dotted_reference_from_expression(value.value)
        if parent is None:
            return None
        return f"{parent}.{value.attr}"

    return None


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
            symbols = [symbol.strip() for symbol in match.group("symbols").split(",")]
            for name in [parent, *symbols]:
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
    source_lines = source.splitlines()

    for line_number, raw_line in enumerate(source.splitlines(), start=1):
        line = raw_line.strip()
        if not line or line.startswith("#"):
            continue

        if match := CYTHON_FROM_IMPORT_RE.match(line):
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
                    source_range=SourceRange.from_offsets(
                        line_number,
                        raw_line.find(alias_name),
                        line_number,
                        raw_line.find(alias_name) + len(alias_name),
                    ),
                )
            continue

        if match := CYTHON_CIMPORT_RE.match(line):
            imported_module = match.group("module")
            alias = match.group("alias") or imported_module.split(".")[-1]
            record.bindings[alias] = ImportBinding(
                alias=alias,
                module_name=imported_module,
                target_name=None,
                source_range=SourceRange.from_offsets(
                    line_number,
                    raw_line.find(alias),
                    line_number,
                    raw_line.find(alias) + len(alias),
                ),
            )
            continue

        symbol_name, kind = parse_cython_symbol_line(line)
        if symbol_name and kind:
            record.symbols[symbol_name] = SymbolRecord(
                name=symbol_name,
                kind=kind,
                module_name=module_name,
                file_path=file_path,
                source_range=SourceRange.from_offsets(
                    line_number,
                    raw_line.find(symbol_name),
                    line_number,
                    raw_line.find(symbol_name) + len(symbol_name),
                ),
                detail=f"{kind} {symbol_name}",
                docstring=extract_block_docstring(source_lines, line_number),
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


def extract_block_docstring(source_lines: list[str], definition_line: int) -> Optional[str]:
    line_index = definition_line - 1
    if line_index < 0 or line_index >= len(source_lines):
        return None

    base_indent = leading_indent(source_lines[line_index])
    index = line_index + 1
    while index < len(source_lines):
        candidate = source_lines[index]
        stripped = candidate.strip()
        if not stripped:
            index += 1
            continue
        if leading_indent(candidate) <= base_indent:
            return None
        return parse_triple_quoted_block(source_lines, index)
    return None


def parse_triple_quoted_block(source_lines: list[str], start_index: int) -> Optional[str]:
    opener_line = source_lines[start_index].lstrip()
    if not opener_line.startswith(('"""', "'''")):
        return None

    quote = opener_line[:3]
    remainder = opener_line[3:]
    if quote in remainder:
        return remainder.split(quote, maxsplit=1)[0].strip()

    body_lines = [remainder]
    for line in source_lines[start_index + 1 :]:
        if quote in line:
            prefix, _ = line.split(quote, maxsplit=1)
            body_lines.append(prefix)
            return "\n".join(body_lines).strip()
        body_lines.append(line)
    return None


def leading_indent(line: str) -> int:
    return len(line) - len(line.lstrip())


def syntax_diagnostics_for_source(
    file_path: Path,
    source: str,
    syntax_error: Optional[SyntaxError] = None,
) -> list[dict[str, object]]:
    active_error = syntax_error
    if active_error is None:
        try:
            ast.parse(sanitized_source_for_validation(file_path, source), filename=str(file_path))
        except SyntaxError as error:
            active_error = error

    if active_error is None:
        return []

    line_number = active_error.lineno or 1
    character = max((active_error.offset or 1) - 1, 0)
    source_lines = source.splitlines()
    source_line = source_lines[line_number - 1] if 0 < line_number <= len(source_lines) else ""
    end_character = min(len(source_line), character + max(1, highlighted_span(active_error)))
    return [
        {
            "range": SourceRange.from_offsets(
                line_number,
                character,
                line_number,
                end_character,
            ).to_lsp(),
            "severity": 1,
            "source": "sage-lsp",
            "message": f"Syntax error: {active_error.msg}",
        }
    ]


def sanitized_source_for_validation(file_path: Path, source: str) -> str:
    if file_path.suffix != ".sage":
        return source

    generated_text = preprocess_sage_source(source).generated_text
    sanitized_lines = [
        LOOSE_PREPARSE_VALIDATE_RE.sub(r"\g<indent>\g<parent>\g<spacing>", line)
        for line in generated_text.splitlines()
    ]
    return "\n".join(sanitized_lines) + ("\n" if generated_text.endswith("\n") else "")


def highlighted_span(error: SyntaxError) -> int:
    if error.end_offset is not None and error.offset is not None and error.end_offset > error.offset:
        return error.end_offset - error.offset
    return 1


def parse_cython_symbol_line(line: str) -> tuple[Optional[str], Optional[str]]:
    if line.startswith(("cdef extern from", "ctypedef ", "include ")):
        return None, None

    class_match = CYTHON_CLASS_RE.match(line)
    if class_match is not None:
        return class_match.group("name"), "class"

    if "(" not in line:
        return None, None

    prefix = line.split("(", maxsplit=1)[0].strip()
    if not prefix:
        return None, None

    if prefix.startswith("async def ") or prefix.startswith("def "):
        return prefix.split()[-1], "function"

    if prefix.startswith("cpdef "):
        return prefix.split()[-1], "function"

    if prefix.startswith("cdef "):
        if prefix.startswith("cdef class "):
            return prefix.split()[-1], "class"
        return prefix.split()[-1], "function"

    return None, None


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

    try:
        parsed = ast.parse(line)
    except SyntaxError:
        return []

    if not parsed.body:
        return []

    bindings = parse_lazy_import_statement("document::line", parsed.body[0])
    return [relocate_binding_line(binding, line_number) for binding in bindings]


def parse_string_or_list(value: Optional[ast.AST]) -> list[str]:
    if isinstance(value, ast.Constant) and isinstance(value.value, str):
        return [value.value]
    if isinstance(value, (ast.List, ast.Tuple)):
        items: list[str] = []
        for item in value.elts:
            if isinstance(item, ast.Constant) and isinstance(item.value, str):
                items.append(item.value)
        return items
    return []


def parse_aliases(value: Optional[ast.AST]) -> list[str]:
    return parse_string_or_list(value)


def relocate_binding_line(binding: ImportBinding, line_number: int) -> ImportBinding:
    line_offset = line_number - (binding.source_range.start.line + 1)
    if line_offset == 0:
        return binding
    return ImportBinding(
        alias=binding.alias,
        module_name=binding.module_name,
        target_name=binding.target_name,
        source_range=SourceRange.from_offsets(
            binding.source_range.start.line + 1 + line_offset,
            binding.source_range.start.character,
            binding.source_range.end.line + 1 + line_offset,
            binding.source_range.end.character,
        ),
        is_lazy=binding.is_lazy,
    )


def is_lazy_import_call(call: ast.Call) -> bool:
    if isinstance(call.func, ast.Name):
        return call.func.id in LAZY_IMPORT_NAMES
    if isinstance(call.func, ast.Attribute):
        return call.func.attr in LAZY_IMPORT_NAMES
    return False


def resolve_imported_module(module_name: str, raw_module: Optional[str], level: int) -> Optional[str]:
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
