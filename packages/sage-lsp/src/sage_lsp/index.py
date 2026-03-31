from __future__ import annotations

import fnmatch
from dataclasses import dataclass
from pathlib import Path
from urllib.parse import unquote, urlparse

from .model import ImportBinding, ModuleRecord, SymbolRecord, document_symbol_kind
from .parser import parse_module


@dataclass(slots=True)
class DocumentationResult:
    name: str
    kind: str
    module_name: str
    uri: str
    detail: str
    summary: str | None
    docstring: str | None
    markers: tuple[str, ...] = ()
    sections: tuple[dict[str, str], ...] = ()

    def to_payload(self) -> dict[str, str | None]:
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
                if path.suffix not in {".py", ".pyx"}:
                    continue
                if path.suffix == ".pyx" and not self._enable_pyx:
                    continue
                if self._is_excluded(root, path):
                    continue
                module_name = module_name_from_path(root, path)
                if not module_name:
                    continue
                source = path.read_text(encoding="utf-8")
                record = parse_module(module_name, path, source)
                self._modules[module_name] = record
                self._module_paths[path.resolve()] = module_name

    def module_for_path(self, path: Path) -> ModuleRecord | None:
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

    def resolve_symbol(self, record: ModuleRecord, name: str) -> SymbolRecord | None:
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

    def documentation_for_symbol(self, record: ModuleRecord, name: str) -> DocumentationResult | None:
        symbol = self.resolve_symbol(record, name)
        if symbol is None:
            return None
        summary, sections = split_docstring(symbol.docstring)
        return DocumentationResult(
            name=name,
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
    ) -> SymbolRecord | None:
        visit_key = (record.module_name, name)
        if visit_key in visited:
            return None
        visited.add(visit_key)

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

    def _resolve_binding(
        self,
        binding: ImportBinding,
        visited: set[tuple[str, str]],
    ) -> SymbolRecord | None:
        target_record = self._modules.get(binding.module_name)
        if target_record is None:
            return None
        if binding.target_name is None:
            return SymbolRecord(
                name=binding.alias,
                kind="module",
                module_name=target_record.module_name,
                file_path=target_record.file_path,
                source_range=target_record.symbols.get(binding.alias, SymbolRecord(
                    name=binding.alias,
                    kind="module",
                    module_name=target_record.module_name,
                    file_path=target_record.file_path,
                    source_range=binding.source_range,
                )).source_range,
                detail=f"module {target_record.module_name}",
                docstring=target_record.docstring,
            )
        return self._resolve_symbol(target_record, binding.target_name, visited)

    def _is_excluded(self, root: Path, path: Path) -> bool:
        relative = path.relative_to(root).as_posix()
        return any(
            fnmatch.fnmatch(relative, pattern) or fnmatch.fnmatch(path.name, pattern)
            for pattern in self._excluded_globs
        )


def module_name_from_path(root: Path, path: Path) -> str | None:
    relative = path.relative_to(root)
    if not relative.parts:
        return None
    parts = list(relative.parts)
    parts[-1] = path.stem
    if parts[-1] == "__init__":
        parts = parts[:-1]
    return ".".join(part for part in parts if part)


def path_from_uri(uri: str) -> Path:
    parsed = urlparse(uri)
    return Path(unquote(parsed.path))


def first_paragraph(docstring: str | None) -> str | None:
    if not docstring:
        return None
    stripped = docstring.strip()
    if not stripped:
        return None
    return stripped.split("\n\n", maxsplit=1)[0].strip()


def split_docstring(docstring: str | None) -> tuple[str | None, tuple[dict[str, str], ...]]:
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
