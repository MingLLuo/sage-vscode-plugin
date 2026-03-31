from __future__ import annotations

import fnmatch
from dataclasses import dataclass
from pathlib import Path
import re
from typing import Optional
from urllib.parse import unquote, urlparse

from .model import ImportBinding, ModuleRecord, SourceRange, SymbolRecord, document_symbol_kind
from .parser import parse_module


@dataclass
class DocumentationResult:
    name: str
    kind: str
    module_name: str
    uri: str
    detail: str
    summary: Optional[str]
    docstring: Optional[str]
    markers: tuple[str, ...] = ()
    sections: tuple[dict[str, str], ...] = ()

    def to_payload(self) -> dict[str, Optional[str]]:
        return {
            "name": self.name,
            "kind": self.kind,
            "moduleName": self.module_name,
            "uri": self.uri,
            "detail": self.detail,
            "summary": self.summary,
            "docstring": self.docstring,
            "markers": list(self.markers),
            "sections": list(self.sections),
        }


class WorkspaceIndex:
    def __init__(self, source_roots: list[Path], excluded_globs: tuple[str, ...], enable_pyx: bool) -> None:
        self._source_roots = source_roots
        self._excluded_globs = excluded_globs
        self._enable_pyx = enable_pyx
        self._modules: dict[str, ModuleRecord] = {}
        self._module_paths: dict[Path, str] = {}

    @property
    def modules(self) -> dict[str, ModuleRecord]:
        return self._modules

    def build(self) -> None:
        self._modules.clear()
        self._module_paths.clear()
        for root in self._source_roots:
            if not root.exists():
                continue
            for path in root.rglob("*"):
                if not path.is_file():
                    continue
                if path.suffix not in {".py", ".pyx", ".pxd"}:
                    continue
                if path.suffix in {".pyx", ".pxd"} and not self._enable_pyx:
                    continue
                if self._is_excluded(root, path):
                    continue
                module_name = module_name_from_path(root, path)
                if not module_name:
                    continue
                source = path.read_text(encoding="utf-8")
                record = parse_module(module_name, path, source)
                existing = self._modules.get(module_name)
                self._modules[module_name] = merge_module_records(existing, record) if existing else record
                self._module_paths[path.resolve()] = module_name

    def module_for_path(self, path: Path) -> Optional[ModuleRecord]:
        module_name = self._module_paths.get(path.resolve())
        if module_name is None:
            return None
        return self._modules.get(module_name)

    def parse_document(
        self,
        uri: str,
        source: str,
        language_id: str,
    ) -> ModuleRecord:
        path = path_from_uri(uri)
        module_name = self._module_paths.get(path.resolve(), f"document::{path.stem}")
        record = parse_module(module_name, path, source)
        if language_id == "sagemath":
            for candidate in ("sage.all_cmdline", "sage.all"):
                if candidate in self._modules and candidate not in record.star_imports:
                    record.star_imports.append(candidate)
        return record

    def resolve_symbol(self, record: ModuleRecord, name: str) -> Optional[SymbolRecord]:
        return self._resolve_symbol(record, name, visited=set())

    def exported_symbols(self, module_name: str) -> dict[str, SymbolRecord]:
        record = self._modules.get(module_name)
        if record is None:
            return {}
        results: dict[str, SymbolRecord] = {}
        for name in self._visible_names(record):
            symbol = self._resolve_symbol(record, name, visited=set())
            if symbol is not None:
                results[name] = symbol
        return results

    def completion_items(self, record: ModuleRecord, prefix: str) -> list[dict[str, object]]:
        seen: dict[str, SymbolRecord] = {}
        for name in sorted(self._visible_names(record)):
            if prefix and not name.startswith(prefix):
                continue
            symbol = self._resolve_symbol(record, name, visited=set())
            if symbol is not None:
                seen.setdefault(name, symbol)
        return [symbol.completion_item() for _, symbol in sorted(seen.items())][:100]

    def member_completion_items(
        self,
        record: ModuleRecord,
        expression: str,
        prefix: str,
    ) -> list[dict[str, object]]:
        base_symbol = self.resolve_symbol(record, expression)
        if base_symbol is None:
            return []
        owner_record = self._record_for_symbol(record, base_symbol)
        if owner_record is None:
            return []

        seen: dict[str, SymbolRecord] = {}
        for name, symbol in sorted(self._resolved_members(owner_record, base_symbol).items()):
            if prefix and not name.startswith(prefix):
                continue
            seen.setdefault(name, symbol)
        return [symbol.completion_item() for _, symbol in sorted(seen.items())][:100]

    def document_symbols(self, record: ModuleRecord) -> list[dict[str, object]]:
        items: list[dict[str, object]] = []
        for name in sorted(self._visible_names(record)):
            symbol = self._resolve_symbol(record, name, visited=set())
            if symbol is None:
                continue
            items.append(
                {
                    "name": name,
                    "kind": document_symbol_kind(symbol.kind),
                    "range": symbol.source_range.to_lsp(),
                    "selectionRange": symbol.source_range.to_lsp(),
                    "detail": symbol.detail,
                }
            )
        return items

    def workspace_symbols(self, query: str) -> list[dict[str, object]]:
        needle = query.casefold().strip()
        items: list[dict[str, object]] = []
        seen: set[tuple[Path, int, int, str]] = set()
        for module_name in sorted(self._modules):
            record = self._modules[module_name]
            for name, symbol in sorted(record.symbols.items()):
                if name.startswith("_"):
                    continue
                haystack = f"{name} {module_name}".casefold()
                if needle and needle not in haystack:
                    continue
                identity = symbol_identity(symbol)
                if identity in seen:
                    continue
                seen.add(identity)
                items.append(
                    {
                        "name": name,
                        "kind": document_symbol_kind(symbol.kind),
                        "location": {
                            "uri": symbol.file_path.as_uri(),
                            "range": symbol.source_range.to_lsp(),
                        },
                        "containerName": module_name,
                    }
                )
            for owner_name in sorted(set(record.member_symbols) | set(record.member_bindings)):
                for name, symbol in sorted(self._resolved_member_symbols(record, owner_name).items()):
                    if name.startswith("_"):
                        continue
                    haystack = f"{name} {owner_name} {module_name}".casefold()
                    if needle and needle not in haystack:
                        continue
                    identity = symbol_identity(symbol)
                    if identity in seen:
                        continue
                    seen.add(identity)
                    items.append(
                        {
                            "name": name,
                            "kind": document_symbol_kind(symbol.kind),
                            "location": {
                                "uri": symbol.file_path.as_uri(),
                                "range": symbol.source_range.to_lsp(),
                            },
                            "containerName": f"{module_name}.{owner_name}",
                        }
                    )
        return items[:200]

    def reference_locations(
        self,
        record: ModuleRecord,
        name: str,
        include_declaration: bool = True,
    ) -> list[dict[str, object]]:
        binding = record.bindings.get(name)
        if (
            binding is not None
            and name not in record.symbols
            and (binding.target_name is None or binding.target_name != name)
        ):
            return self._local_name_locations(record, name)

        target = self.resolve_symbol(record, name)
        if target is None:
            return self._local_name_locations(record, name)

        target_identity = symbol_identity(target)
        records = self._records_for_search(record)
        locations: list[dict[str, object]] = []
        seen: set[tuple[str, int, int, int, int]] = set()

        for candidate in records.values():
            resolved = self.resolve_symbol(candidate, name)
            if resolved is None or symbol_identity(resolved) != target_identity:
                continue
            for source_range in iter_identifier_ranges(candidate.source, name):
                if not include_declaration and _same_location(candidate.file_path, source_range, target.file_path, target.source_range):
                    continue
                location = {
                    "uri": candidate.file_path.as_uri(),
                    "range": source_range.to_lsp(),
                }
                location_key = location_identity(location)
                if location_key in seen:
                    continue
                seen.add(location_key)
                locations.append(location)

        return locations

    def rename_edits(
        self,
        record: ModuleRecord,
        name: str,
        new_name: str,
    ) -> dict[str, list[dict[str, object]]]:
        locations = self.reference_locations(record, name, include_declaration=True)
        changes: dict[str, list[dict[str, object]]] = {}
        for location in locations:
            uri = location["uri"]
            changes.setdefault(uri, []).append(
                {
                    "range": location["range"],
                    "newText": new_name,
                }
            )
        return changes

    def diagnostics_for_record(self, record: ModuleRecord) -> list[dict[str, object]]:
        diagnostics: list[dict[str, object]] = []
        seen: set[tuple[int, int, str]] = set()
        for binding in record.bindings.values():
            target_record = self._modules.get(binding.module_name)
            message: Optional[str] = None
            if target_record is None:
                message = f"Unresolved import module '{binding.module_name}'"
            elif binding.target_name is not None and self._resolve_symbol(target_record, binding.target_name, visited=set()) is None:
                message = (
                    f"Unresolved import name '{binding.target_name}' from '{binding.module_name}'"
                )
            if message is None:
                continue
            diagnostic_key = (
                binding.source_range.start.line,
                binding.source_range.start.character,
                message,
            )
            if diagnostic_key in seen:
                continue
            seen.add(diagnostic_key)
            diagnostics.append(
                {
                    "range": binding.source_range.to_lsp(),
                    "severity": 2,
                    "source": "sage-lsp",
                    "message": message,
                }
            )
        return diagnostics

    def documentation_for_symbol(self, record: ModuleRecord, name: str) -> Optional[DocumentationResult]:
        symbol = self.resolve_symbol(record, name)
        if symbol is None:
            return None
        summary, sections = split_docstring(symbol.docstring)
        display_name = symbol.name if "." in name else name
        return DocumentationResult(
            name=display_name,
            kind=symbol.kind,
            module_name=symbol.module_name,
            uri=symbol.file_path.as_uri(),
            detail=symbol.detail,
            summary=summary,
            docstring=symbol.docstring,
            markers=(
                f"kind:{symbol.kind}",
                f"module:{symbol.module_name}",
                f"source:{symbol.file_path.suffix.lstrip('.') or 'py'}",
            ),
            sections=sections,
        )

    def _visible_names(self, record: ModuleRecord) -> set[str]:
        names = set(record.symbols)
        names.update(record.bindings)
        for star_import in record.star_imports:
            names.update(self.exported_symbols(star_import))
        return {name for name in names if not name.startswith("_")}

    def _resolve_symbol(
        self,
        record: ModuleRecord,
        name: str,
        visited: set[tuple[str, str]],
    ) -> Optional[SymbolRecord]:
        visit_key = (record.module_name, name)
        if visit_key in visited:
            return None
        visited.add(visit_key)

        if "." in name:
            return self._resolve_dotted_symbol(record, name, visited)

        if name in record.symbols:
            return record.symbols[name]

        binding = record.bindings.get(name)
        if binding is not None:
            resolved = self._resolve_binding(binding, visited)
            if resolved is not None:
                return resolved
            return SymbolRecord(
                name=name,
                kind="module" if binding.target_name is None else "variable",
                module_name=binding.module_name,
                file_path=self._modules.get(binding.module_name, record).file_path,
                source_range=binding.source_range,
                detail=f"import from {binding.module_name}",
            )

        for star_import in record.star_imports:
            imported_record = self._modules.get(star_import)
            if imported_record is None:
                continue
            resolved = self._resolve_symbol(imported_record, name, visited)
            if resolved is not None:
                return resolved

        return None

    def _resolve_dotted_symbol(
        self,
        record: ModuleRecord,
        name: str,
        visited: set[tuple[str, str]],
    ) -> Optional[SymbolRecord]:
        parts = [part for part in name.split(".") if part]
        if not parts:
            return None

        current_record = record
        current_symbol = self._resolve_symbol(current_record, parts[0], visited)
        if current_symbol is None:
            return None

        for attribute in parts[1:]:
            owner_record = self._record_for_symbol(current_record, current_symbol)
            if owner_record is None:
                return None
            current_symbol = self._resolve_member_symbol(owner_record, current_symbol, attribute, visited)
            if current_symbol is None:
                return None
            current_record = owner_record

        return current_symbol

    def _resolve_binding(
        self,
        binding: ImportBinding,
        visited: set[tuple[str, str]],
    ) -> Optional[SymbolRecord]:
        target_record = self._modules.get(binding.module_name)
        if target_record is None:
            return None
        if binding.target_name is None:
            return self._symbol_from_module_binding(binding, target_record)
        return self._resolve_symbol(target_record, binding.target_name, visited)

    def _resolve_member_symbol(
        self,
        record: ModuleRecord,
        symbol: SymbolRecord,
        attribute: str,
        visited: set[tuple[str, str]],
    ) -> Optional[SymbolRecord]:
        if symbol.kind == "module":
            module_record = self._modules.get(symbol.module_name)
            if module_record is None:
                return None
            return self._resolve_symbol(module_record, attribute, visited)

        owner_name = None
        if symbol.kind == "class":
            owner_name = symbol.name
        else:
            owner_name = record.instance_types.get(symbol.name)

        if owner_name is None:
            return None
        return self._resolve_member_from_owner(record, owner_name, attribute, visited)

    def _resolve_member_from_owner(
        self,
        record: ModuleRecord,
        owner_name: str,
        attribute: str,
        visited: set[tuple[str, str]],
    ) -> Optional[SymbolRecord]:
        direct_symbol = record.member_symbols.get(owner_name, {}).get(attribute)
        if direct_symbol is not None:
            return direct_symbol

        binding = record.member_bindings.get(owner_name, {}).get(attribute)
        if binding is None:
            return None

        resolved = self._resolve_binding(binding, visited)
        if resolved is not None:
            return resolved
        target_record = self._modules.get(binding.module_name)
        if target_record is None:
            return None
        return self._symbol_from_module_binding(binding, target_record)

    def _resolved_member_symbols(self, record: ModuleRecord, owner_name: str) -> dict[str, SymbolRecord]:
        members: dict[str, SymbolRecord] = dict(record.member_symbols.get(owner_name, {}))
        for name, binding in record.member_bindings.get(owner_name, {}).items():
            resolved = self._resolve_binding(binding, visited=set())
            if resolved is not None:
                members[name] = resolved
                continue
            target_record = self._modules.get(binding.module_name)
            if target_record is not None:
                members[name] = self._symbol_from_module_binding(binding, target_record)
        return members

    def _resolved_members(self, record: ModuleRecord, symbol: SymbolRecord) -> dict[str, SymbolRecord]:
        if symbol.kind == "module":
            module_record = self._modules.get(symbol.module_name)
            if module_record is None:
                return {}
            members: dict[str, SymbolRecord] = {}
            for name in self._visible_names(module_record):
                resolved = self._resolve_symbol(module_record, name, visited=set())
                if resolved is not None:
                    members[name] = resolved
            return members

        owner_name = symbol.name if symbol.kind == "class" else record.instance_types.get(symbol.name)
        if owner_name is None:
            return {}
        return self._resolved_member_symbols(record, owner_name)

    def _record_for_symbol(
        self,
        current_record: ModuleRecord,
        symbol: SymbolRecord,
    ) -> Optional[ModuleRecord]:
        if symbol.module_name == current_record.module_name:
            return current_record
        return self._modules.get(symbol.module_name)

    def _symbol_from_module_binding(
        self,
        binding: ImportBinding,
        target_record: ModuleRecord,
    ) -> SymbolRecord:
        fallback = SymbolRecord(
            name=binding.alias,
            kind="module",
            module_name=target_record.module_name,
            file_path=target_record.file_path,
            source_range=binding.source_range,
        )
        return SymbolRecord(
            name=binding.alias,
            kind="module",
            module_name=target_record.module_name,
            file_path=target_record.file_path,
            source_range=target_record.symbols.get(binding.alias, fallback).source_range,
            detail=f"module {target_record.module_name}",
            docstring=target_record.docstring,
        )

    def _is_excluded(self, root: Path, path: Path) -> bool:
        relative = path.relative_to(root).as_posix()
        return any(
            fnmatch.fnmatch(relative, pattern) or fnmatch.fnmatch(path.name, pattern)
            for pattern in self._excluded_globs
        )

    def _records_for_search(self, current_record: ModuleRecord) -> dict[str, ModuleRecord]:
        records = dict(self._modules)
        records[current_record.module_name] = current_record
        return records

    def _local_name_locations(self, record: ModuleRecord, name: str) -> list[dict[str, object]]:
        return [
            {
                "uri": record.file_path.as_uri(),
                "range": source_range.to_lsp(),
            }
            for source_range in iter_identifier_ranges(record.source, name)
        ]


def module_name_from_path(root: Path, path: Path) -> Optional[str]:
    relative = path.relative_to(root)
    if not relative.parts:
        return None
    parts = list(relative.parts)
    parts[-1] = path.stem
    if parts[-1] == "__init__":
        parts = parts[:-1]
    return ".".join(part for part in parts if part)


def merge_module_records(existing: ModuleRecord, candidate: ModuleRecord) -> ModuleRecord:
    ordered_records = sorted((existing, candidate), key=module_record_precedence)
    preferred = ordered_records[-1]
    merged = ModuleRecord(
        module_name=preferred.module_name,
        file_path=preferred.file_path,
        language=preferred.language,
        source=preferred.source,
        docstring=preferred.docstring or ordered_records[0].docstring,
    )
    for record in ordered_records:
        merged.symbols.update(record.symbols)
        merged.bindings.update(record.bindings)
        merged.instance_types.update(record.instance_types)
        for owner_name, symbols in record.member_symbols.items():
            merged.member_symbols.setdefault(owner_name, {}).update(symbols)
        for owner_name, bindings in record.member_bindings.items():
            merged.member_bindings.setdefault(owner_name, {}).update(bindings)
        for star_import in record.star_imports:
            if star_import not in merged.star_imports:
                merged.star_imports.append(star_import)
    return merged


def module_record_precedence(record: ModuleRecord) -> int:
    return {
        ".py": 4,
        ".sage": 4,
        ".pyx": 3,
        ".pxd": 2,
        ".pxi": 1,
    }.get(record.file_path.suffix, 0)


def iter_identifier_ranges(text: str, name: str) -> list[SourceRange]:
    if not name:
        return []

    line_number = 1
    column = 0
    index = 0
    state = "code"
    ranges: list[SourceRange] = []

    while index < len(text):
        char = text[index]

        if char == "\n":
            line_number += 1
            column = 0
            index += 1
            continue

        if state == "code":
            if text.startswith("'''", index):
                state = "triple_single"
                index += 3
                column += 3
                continue
            if text.startswith('"""', index):
                state = "triple_double"
                index += 3
                column += 3
                continue
            if char == "#":
                while index < len(text) and text[index] != "\n":
                    index += 1
                    column += 1
                continue
            if char == "'":
                state = "single"
                index += 1
                column += 1
                continue
            if char == '"':
                state = "double"
                index += 1
                column += 1
                continue
            if char.isalpha() or char == "_":
                start_line = line_number
                start_column = column
                end_index = index + 1
                end_column = column + 1
                while end_index < len(text) and (text[end_index].isalnum() or text[end_index] == "_"):
                    end_index += 1
                    end_column += 1
                if text[index:end_index] == name:
                    ranges.append(
                        SourceRange.from_offsets(
                            start_line,
                            start_column,
                            start_line,
                            end_column,
                        )
                    )
                index = end_index
                column = end_column
                continue

            index += 1
            column += 1
            continue

        if state in {"single", "double"}:
            if char == "\\" and index + 1 < len(text):
                index += 2
                column += 2
                continue
            if (state == "single" and char == "'") or (state == "double" and char == '"'):
                state = "code"
            index += 1
            column += 1
            continue

        if state == "triple_single":
            if text.startswith("'''", index):
                state = "code"
                index += 3
                column += 3
            else:
                index += 1
                column += 1
            continue

        if text.startswith('"""', index):
            state = "code"
            index += 3
            column += 3
        else:
            index += 1
            column += 1

    return ranges


def symbol_identity(symbol: SymbolRecord) -> tuple[Path, int, int, str]:
    return (
        symbol.file_path.resolve(),
        symbol.source_range.start.line,
        symbol.source_range.start.character,
        symbol.name,
    )


def location_identity(location: dict[str, object]) -> tuple[str, int, int, int, int]:
    raw_range = location["range"]
    return (
        str(location["uri"]),
        int(raw_range["start"]["line"]),
        int(raw_range["start"]["character"]),
        int(raw_range["end"]["line"]),
        int(raw_range["end"]["character"]),
    )


def _same_location(
    left_path: Path,
    left_range: SourceRange,
    right_path: Path,
    right_range: SourceRange,
) -> bool:
    return (
        left_path.resolve() == right_path.resolve()
        and left_range.start.line == right_range.start.line
        and left_range.start.character == right_range.start.character
        and left_range.end.line == right_range.end.line
        and left_range.end.character == right_range.end.character
    )


def path_from_uri(uri: str) -> Path:
    parsed = urlparse(uri)
    path_text = unquote(parsed.path)
    if parsed.netloc:
        return Path(f"//{parsed.netloc}{path_text}")
    if re.match(r"^/[A-Za-z]:", path_text):
        path_text = path_text[1:]
    return Path(path_text)


def first_paragraph(docstring: Optional[str]) -> Optional[str]:
    if not docstring:
        return None
    stripped = docstring.strip()
    if not stripped:
        return None
    return stripped.split("\n\n", maxsplit=1)[0].strip()


def split_docstring(docstring: Optional[str]) -> tuple[Optional[str], tuple[dict[str, str], ...]]:
    if not docstring:
      return None, ()
    stripped = docstring.strip()
    if not stripped:
      return None, ()
    paragraphs = [paragraph.strip() for paragraph in stripped.split("\n\n") if paragraph.strip()]
    summary = paragraphs[0] if paragraphs else None
    sections: list[dict[str, str]] = []
    if len(paragraphs) > 1:
      sections.append({"title": "Details", "body": "\n\n".join(paragraphs[1:])})
    return summary, tuple(sections)
