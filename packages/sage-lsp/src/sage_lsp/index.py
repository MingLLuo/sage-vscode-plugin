from __future__ import annotations

import fnmatch
from dataclasses import dataclass
import hashlib
import json
from pathlib import Path
import re
import tempfile
from typing import Optional
from urllib.parse import unquote, urlparse

from .model import ImportBinding, ModuleRecord, SourceRange, SymbolRecord, document_symbol_kind
from .parser import parse_module


CACHE_SCHEMA_VERSION = 2


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
    def __init__(
        self,
        source_roots: list[Path],
        excluded_globs: tuple[str, ...],
        enable_pyx: bool,
        cache_dir: Optional[Path] = None,
    ) -> None:
        self._source_roots = source_roots
        self._excluded_globs = excluded_globs
        self._enable_pyx = enable_pyx
        self._cache_dir = cache_dir or default_index_cache_dir()
        self._modules: dict[str, ModuleRecord] = {}
        self._module_paths: dict[Path, str] = {}
        self._module_component_paths: dict[str, set[Path]] = {}
        self._module_records_by_path: dict[Path, ModuleRecord] = {}
        self._resolved_symbol_cache: dict[tuple[str, str], Optional[SymbolRecord]] = {}
        self._resolved_member_cache: dict[tuple[str, str, str], Optional[SymbolRecord]] = {}
        self._document_records: dict[str, tuple[str, str, ModuleRecord]] = {}
        self._cache_entries: dict[str, dict[str, object]] = {}
        self._cache_snapshot_complete = False
        self._loading_modules: set[str] = set()
        self._fully_indexed = False

    @property
    def modules(self) -> dict[str, ModuleRecord]:
        return self._modules

    def build(self) -> None:
        self._reset_runtime_state()
        self._cache_entries = self._load_cached_entries()
        self._load_modules_for_roots(self._source_roots, reset_cache_entries=True)
        self._clear_resolution_caches()
        self._fully_indexed = True
        self._cache_snapshot_complete = True
        self._persist_cache_snapshot()

    def hydrate_from_cache(self) -> bool:
        self._reset_runtime_state()
        self._cache_entries = self._load_cached_entries()
        if not self._cache_entries:
            return False

        restored_any = self._restore_cached_entries(self._cache_entries)
        self._clear_resolution_caches()
        self._fully_indexed = restored_any
        self._cache_snapshot_complete = restored_any
        return restored_any

    def load_bootstrap_modules(self, module_names: tuple[str, ...]) -> bool:
        loaded_any = False
        for module_name in module_names:
            loaded_any = self._ensure_module_loaded(module_name) is not None or loaded_any
        return loaded_any

    def load_roots(
        self,
        roots: list[Path],
        *,
        mark_fully_indexed: bool = False,
        persist_snapshot: bool = False,
    ) -> None:
        self._load_modules_for_roots(roots, reset_cache_entries=False)
        if mark_fully_indexed:
            self._fully_indexed = True
            self._cache_snapshot_complete = True
        self._clear_resolution_caches()
        if persist_snapshot:
            self._persist_cache_snapshot()

    def ensure_full_index(self) -> None:
        if self._fully_indexed:
            return
        self.build()

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
        cached_document = self._cached_document_record(uri, source, language_id)
        if cached_document is not None:
            return cached_document

        path = path_from_uri(uri)
        module_name = self._module_paths.get(path.resolve(), f"document::{path.stem}")
        record = parse_module(module_name, path, source)
        if language_id == "sagemath":
            self._inject_default_sage_imports(record)
        self._remember_document_record(uri, source, language_id, record)
        return record

    def drop_document(self, uri: str) -> None:
        self._document_records.pop(uri, None)

    def refresh_path(self, path: Path) -> Optional[ModuleRecord]:
        self.refresh_paths([path])
        indexed_path = self._indexed_path(path)
        if indexed_path is None:
            return None
        module_name = self._module_paths.get(indexed_path)
        if module_name is None:
            return None
        return self._modules.get(module_name)

    def refresh_paths(self, paths: list[Path]) -> dict[Path, Optional[ModuleRecord]]:
        results: dict[Path, Optional[ModuleRecord]] = {}
        changed = False
        for path in paths:
            indexed_path = self._indexed_path(path)
            if indexed_path is None:
                continue
            before_module_name = self._module_paths.get(indexed_path)
            changed = self._refresh_path_in_memory(indexed_path) or changed
            after_module_name = self._module_paths.get(indexed_path, before_module_name)
            results[indexed_path] = self._modules.get(after_module_name) if after_module_name else None
        if changed:
            self._clear_resolution_caches()
            self._persist_cache_snapshot()
        return results

    def remove_paths(self, paths: list[Path]) -> None:
        changed = False
        for path in paths:
            indexed_path = self._indexed_path(path)
            if indexed_path is None:
                continue
            changed = self._remove_path_in_memory(indexed_path) or changed
        if changed:
            self._clear_resolution_caches()
            self._persist_cache_snapshot()

    def refresh_or_remove_paths(
        self,
        changes: list[tuple[Path, bool]],
    ) -> dict[Path, Optional[ModuleRecord]]:
        results: dict[Path, Optional[ModuleRecord]] = {}
        changed = False
        for path, deleted in changes:
            indexed_path = self._indexed_path(path)
            if indexed_path is None:
                continue
            if deleted:
                changed = self._remove_path_in_memory(indexed_path) or changed
                results[indexed_path] = None
                continue
            before_module_name = self._module_paths.get(indexed_path)
            changed = self._refresh_path_in_memory(indexed_path) or changed
            after_module_name = self._module_paths.get(indexed_path, before_module_name)
            results[indexed_path] = self._modules.get(after_module_name) if after_module_name else None
        if changed:
            self._clear_resolution_caches()
            self._persist_cache_snapshot()
        return results

    def remove_path(self, path: Path) -> None:
        self.remove_paths([path])

    def _refresh_path_in_memory(self, indexed_path: Path) -> bool:
        if not indexed_path.exists():
            return self._remove_path_in_memory(indexed_path)

        root = self._source_root_for_path(indexed_path)
        if root is None or not self._is_indexable_path(root, indexed_path):
            return self._remove_path_in_memory(indexed_path)

        module_name = module_name_from_path(root, indexed_path)
        if not module_name:
            return False

        fingerprint = file_fingerprint(indexed_path)
        cache_key = str(indexed_path)
        cached_entry = self._cache_entries.get(cache_key)
        source = self._source_for_module_path(indexed_path, cached_entry, fingerprint)
        record = self._load_or_parse_module_record(
            module_name,
            indexed_path,
            source,
            cached_entry,
            fingerprint,
        )
        self._store_module_record(module_name, indexed_path, record)
        self._cache_entries[cache_key] = {
            "moduleName": module_name,
            "fingerprint": fingerprint,
            "source": source,
            "record": serialize_module_record(record),
        }
        return True

    def _remove_path_in_memory(self, indexed_path: Path) -> bool:
        cache_key = str(indexed_path)
        existed = cache_key in self._cache_entries or indexed_path in self._module_paths
        self._cache_entries.pop(cache_key, None)

        module_name = self._module_paths.pop(indexed_path, None)
        self._module_records_by_path.pop(indexed_path, None)

        if module_name is not None:
            component_paths = self._module_component_paths.get(module_name)
            if component_paths is not None:
                component_paths.discard(indexed_path)
                if component_paths:
                    self._rebuild_module_record(module_name)
                else:
                    self._module_component_paths.pop(module_name, None)
                    self._modules.pop(module_name, None)
        return existed

    def resolve_symbol(self, record: ModuleRecord, name: str) -> Optional[SymbolRecord]:
        cache_key = self._symbol_cache_key(record, name)
        if cache_key is not None and cache_key in self._resolved_symbol_cache:
            return self._resolved_symbol_cache[cache_key]

        resolved = self._resolve_symbol(record, name, visited=set())
        if cache_key is not None:
            self._resolved_symbol_cache[cache_key] = resolved
        return resolved

    def exported_symbols(self, module_name: str) -> dict[str, SymbolRecord]:
        record = self._modules.get(module_name) or self._ensure_module_loaded(module_name)
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
        self.ensure_full_index()
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

    def import_candidates(
        self,
        name: str,
        *,
        exclude_module: Optional[str] = None,
    ) -> list[str]:
        self.ensure_full_index()
        candidates: list[tuple[int, str]] = []
        for module_name in sorted(self._modules):
            if module_name == exclude_module:
                continue
            record = self._modules[module_name]
            if name not in self._visible_names(record):
                continue
            symbol = self._resolve_symbol(record, name, visited=set())
            if symbol is None:
                continue
            score = 2
            if module_name in {"sage.all", "sage.all_cmdline"}:
                score = 1
            elif symbol.module_name == module_name:
                score = 0
            candidates.append((score, module_name))
        return [module_name for _, module_name in sorted(dict.fromkeys(candidates))]

    def diagnostics_for_record(self, record: ModuleRecord) -> list[dict[str, object]]:
        diagnostics: list[dict[str, object]] = list(record.diagnostics)
        seen: set[tuple[int, int, str]] = {
            (
                int(entry["range"]["start"]["line"]),
                int(entry["range"]["start"]["character"]),
                str(entry["message"]),
            )
            for entry in diagnostics
        }
        for binding in record.bindings.values():
            target_record = self._modules.get(binding.module_name) or self._ensure_module_loaded(binding.module_name)
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
                    "code": "unresolved-import-module" if target_record is None else "unresolved-import-name",
                    "message": message,
                    "data": {
                        "alias": binding.alias,
                        "moduleName": binding.module_name,
                        "targetName": binding.target_name,
                    },
                }
            )
        return diagnostics

    def documentation_for_symbol(self, record: ModuleRecord, name: str) -> Optional[DocumentationResult]:
        symbol = self.resolve_symbol(record, name)
        if symbol is None:
            return None
        docstring = symbol.docstring
        if docstring is None:
            documentation_proxy = self._documentation_proxy_symbol(record, symbol)
            if documentation_proxy is not None:
                docstring = documentation_proxy.docstring
        summary, sections = split_docstring(docstring)
        display_name = symbol.name if "." in name else name
        return DocumentationResult(
            name=display_name,
            kind=symbol.kind,
            module_name=symbol.module_name,
            uri=symbol.file_path.as_uri(),
            detail=symbol.detail,
            summary=summary,
            docstring=docstring,
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
            imported_record = self._modules.get(star_import) or self._ensure_module_loaded(star_import)
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
        target_record = self._modules.get(binding.module_name) or self._ensure_module_loaded(binding.module_name)
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
        cache_key = self._member_cache_key(record, owner_name, attribute)
        if cache_key is not None and cache_key in self._resolved_member_cache:
            return self._resolved_member_cache[cache_key]

        direct_symbol = record.member_symbols.get(owner_name, {}).get(attribute)
        if direct_symbol is not None:
            if cache_key is not None:
                self._resolved_member_cache[cache_key] = direct_symbol
            return direct_symbol

        binding = record.member_bindings.get(owner_name, {}).get(attribute)
        if binding is None:
            if cache_key is not None:
                self._resolved_member_cache[cache_key] = None
            return None

        resolved = self._resolve_binding(binding, visited)
        if resolved is not None:
            if cache_key is not None:
                self._resolved_member_cache[cache_key] = resolved
            return resolved
        target_record = self._modules.get(binding.module_name)
        if target_record is None:
            if cache_key is not None:
                self._resolved_member_cache[cache_key] = None
            return None
        result = self._symbol_from_module_binding(binding, target_record)
        if cache_key is not None:
            self._resolved_member_cache[cache_key] = result
        return result

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

    def _documentation_proxy_symbol(
        self,
        current_record: ModuleRecord,
        symbol: SymbolRecord,
    ) -> Optional[SymbolRecord]:
        if symbol.kind != "variable":
            return None
        owner_record = self._record_for_symbol(current_record, symbol)
        if owner_record is None:
            return None
        instance_type = owner_record.instance_types.get(symbol.name)
        if instance_type is None:
            return None
        proxy_symbol = owner_record.symbols.get(instance_type)
        if proxy_symbol is None or not proxy_symbol.docstring:
            return None
        return proxy_symbol

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

    def _symbol_cache_key(self, record: ModuleRecord, name: str) -> Optional[tuple[str, str]]:
        if not self._is_persisted_module_record(record):
            return None
        return (record.module_name, name)

    def _member_cache_key(
        self,
        record: ModuleRecord,
        owner_name: str,
        attribute: str,
    ) -> Optional[tuple[str, str, str]]:
        if not self._is_persisted_module_record(record):
            return None
        return (record.module_name, owner_name, attribute)

    def _reset_runtime_state(self) -> None:
        self._modules.clear()
        self._module_paths.clear()
        self._module_component_paths.clear()
        self._module_records_by_path.clear()
        self._resolved_symbol_cache.clear()
        self._resolved_member_cache.clear()
        self._document_records.clear()
        self._cache_entries = {}
        self._cache_snapshot_complete = False
        self._loading_modules.clear()
        self._fully_indexed = False

    def _iter_indexable_modules(self) -> list[tuple[Path, Path, str]]:
        return self._iter_indexable_modules_for_roots(self._source_roots)

    def _iter_indexable_modules_for_roots(
        self,
        roots: list[Path],
    ) -> list[tuple[Path, Path, str]]:
        results: list[tuple[Path, Path, str]] = []
        for root in roots:
            if not root.exists():
                continue
            for path in root.rglob("*"):
                if not self._is_indexable_path(root, path):
                    continue
                module_name = module_name_from_path(root, path)
                if module_name:
                    results.append((root, path, module_name))
        return results

    def _load_modules_for_roots(
        self,
        roots: list[Path],
        *,
        reset_cache_entries: bool,
    ) -> None:
        next_cache_entries = {} if reset_cache_entries else dict(self._cache_entries)
        for root, path, module_name in self._iter_indexable_modules_for_roots(roots):
            cache_key = str(path.resolve())
            fingerprint = file_fingerprint(path)
            cached_entry = self._cache_entries.get(cache_key)
            source = self._source_for_module_path(path, cached_entry, fingerprint)
            record = self._load_or_parse_module_record(
                module_name,
                path,
                source,
                cached_entry,
                fingerprint,
            )
            self._store_module_record(module_name, path, record, clear_caches=False)
            next_cache_entries[cache_key] = {
                "moduleName": module_name,
                "fingerprint": fingerprint,
                "source": source,
                "record": serialize_module_record(record),
            }
        self._cache_entries = next_cache_entries

    def _is_indexable_path(self, root: Path, path: Path) -> bool:
        if not path.is_file():
            return False
        if path.suffix not in {".py", ".sage", ".pyx", ".pxd", ".pxi"}:
            return False
        if path.suffix in {".pyx", ".pxd", ".pxi"} and not self._enable_pyx:
            return False
        return not self._is_excluded(root, path)

    def _load_or_parse_module_record(
        self,
        module_name: str,
        path: Path,
        source: str,
        cached_entry: Optional[dict[str, object]],
        fingerprint: dict[str, int],
    ) -> ModuleRecord:
        if _cache_entry_matches(cached_entry, module_name, fingerprint):
            return deserialize_module_record(cached_entry["record"], module_name, path, source)
        return parse_module(module_name, path, source)

    def _source_for_module_path(
        self,
        path: Path,
        cached_entry: Optional[dict[str, object]],
        fingerprint: dict[str, int],
    ) -> str:
        cached_source = self._cached_source_for_entry(cached_entry, fingerprint)
        if cached_source is not None:
            return cached_source
        return self._read_module_source(path)

    def _store_module_record(
        self,
        module_name: str,
        path: Path,
        record: ModuleRecord,
        *,
        clear_caches: bool = True,
    ) -> None:
        resolved_path = path.resolve()
        previous_module_name = self._module_paths.get(resolved_path)
        if previous_module_name is not None and previous_module_name != module_name:
            previous_component_paths = self._module_component_paths.get(previous_module_name)
            if previous_component_paths is not None:
                previous_component_paths.discard(resolved_path)
                if previous_component_paths:
                    self._rebuild_module_record(previous_module_name)
                else:
                    self._module_component_paths.pop(previous_module_name, None)
                    self._modules.pop(previous_module_name, None)

        self._module_paths[resolved_path] = module_name
        self._module_records_by_path[resolved_path] = record
        self._module_component_paths.setdefault(module_name, set()).add(resolved_path)
        self._rebuild_module_record(module_name)
        if clear_caches:
            self._clear_resolution_caches()

    def _cached_document_record(
        self,
        uri: str,
        source: str,
        language_id: str,
    ) -> Optional[ModuleRecord]:
        cached_document = self._document_records.get(uri)
        if cached_document is None:
            return None
        cached_language_id, cached_source, cached_record = cached_document
        if cached_language_id != language_id or cached_source != source:
            return None
        return cached_record

    def _remember_document_record(
        self,
        uri: str,
        source: str,
        language_id: str,
        record: ModuleRecord,
    ) -> None:
        self._document_records[uri] = (language_id, source, record)

    def _inject_default_sage_imports(self, record: ModuleRecord) -> None:
        for candidate in ("sage.all_cmdline", "sage.all"):
            if (candidate in self._modules or self._ensure_module_loaded(candidate) is not None) and candidate not in record.star_imports:
                record.star_imports.append(candidate)

    def _cached_source_for_entry(
        self,
        cached_entry: Optional[dict[str, object]],
        fingerprint: dict[str, int],
    ) -> Optional[str]:
        if not isinstance(cached_entry, dict):
            return None
        if cached_entry.get("fingerprint") != fingerprint:
            return None
        source = cached_entry.get("source")
        return source if isinstance(source, str) else None

    def _read_module_source(self, path: Path) -> str:
        return path.read_text(encoding="utf-8")

    def _is_persisted_module_record(self, record: ModuleRecord) -> bool:
        cached_record = self._modules.get(record.module_name)
        if cached_record is None:
            return False
        return (
            cached_record.source == record.source
            and cached_record.file_path.resolve() == record.file_path.resolve()
        )

    def _indexed_path(self, path: Path) -> Optional[Path]:
        try:
            return path.resolve()
        except OSError:
            return None

    def _source_root_for_path(self, path: Path) -> Optional[Path]:
        for root in self._source_roots:
            try:
                path.relative_to(root.resolve())
                return root
            except ValueError:
                continue
        return None

    def _rebuild_module_record(self, module_name: str) -> None:
        component_paths = self._module_component_paths.get(module_name)
        if not component_paths:
            self._modules.pop(module_name, None)
            return

        ordered_records = sorted(
            (self._module_records_by_path[path] for path in component_paths if path in self._module_records_by_path),
            key=lambda record: (module_record_precedence(record), str(record.file_path)),
        )
        if not ordered_records:
            self._modules.pop(module_name, None)
            return

        merged = ordered_records[0]
        for record in ordered_records[1:]:
            merged = merge_module_records(merged, record)
        self._modules[module_name] = merged

    def _clear_resolution_caches(self) -> None:
        self._resolved_symbol_cache.clear()
        self._resolved_member_cache.clear()

    def _restore_cached_entries(self, entries: dict[str, dict[str, object]]) -> bool:
        restored_any = False
        for cache_key, entry in entries.items():
            module_name = entry.get("moduleName")
            source = entry.get("source")
            payload = entry.get("record")
            if not isinstance(module_name, str) or not isinstance(source, str) or not isinstance(payload, dict):
                continue
            path = Path(cache_key)
            record = deserialize_module_record(payload, module_name, path, source)
            self._store_module_record(module_name, path, record, clear_caches=False)
            restored_any = True
        return restored_any

    def _ensure_module_loaded(self, module_name: str) -> Optional[ModuleRecord]:
        existing = self._modules.get(module_name)
        if existing is not None:
            return existing
        if module_name in self._loading_modules:
            return None

        self._loading_modules.add(module_name)
        try:
            loaded_any = False
            for root in self._source_roots:
                for candidate_path in self._candidate_paths_for_module(root, module_name):
                    if not candidate_path.exists() or not self._is_indexable_path(root, candidate_path):
                        continue
                    fingerprint = file_fingerprint(candidate_path)
                    cache_key = str(candidate_path.resolve())
                    cached_entry = self._cache_entries.get(cache_key)
                    source = self._source_for_module_path(candidate_path, cached_entry, fingerprint)
                    record = self._load_or_parse_module_record(
                        module_name,
                        candidate_path,
                        source,
                        cached_entry,
                        fingerprint,
                    )
                    self._store_module_record(module_name, candidate_path, record, clear_caches=False)
                    self._cache_entries[cache_key] = {
                        "moduleName": module_name,
                        "fingerprint": fingerprint,
                        "source": source,
                        "record": serialize_module_record(record),
                    }
                    loaded_any = True
            if loaded_any:
                self._clear_resolution_caches()
            return self._modules.get(module_name)
        finally:
            self._loading_modules.discard(module_name)

    def _candidate_paths_for_module(self, root: Path, module_name: str) -> list[Path]:
        relative_parts = module_name.split(".")
        if not relative_parts:
            return []

        base_path = root.joinpath(*relative_parts)
        candidates = [
            base_path / "__init__.py",
            base_path / "__init__.sage",
            *(base_path.with_suffix(suffix) for suffix in (".py", ".sage", ".pyx", ".pxd", ".pxi")),
        ]

        deduped: list[Path] = []
        seen: set[Path] = set()
        for candidate in candidates:
            resolved = candidate.resolve()
            if resolved in seen:
                continue
            seen.add(resolved)
            deduped.append(resolved)
        return deduped

    def _cache_file_path(self) -> Path:
        resolved_cache_dir = _resolve_cache_dir(self._cache_dir)
        if resolved_cache_dir is None:
            raise OSError("No writable cache directory available")
        digest = hashlib.sha256(
            "\0".join(
                [
                    *[str(path.resolve()) for path in self._source_roots],
                    *self._excluded_globs,
                    f"enable_pyx={self._enable_pyx}",
                    f"schema={CACHE_SCHEMA_VERSION}",
                ]
            ).encode("utf-8")
        ).hexdigest()[:16]
        return resolved_cache_dir / f"workspace-index-{digest}.json"

    def _load_cached_entries(self) -> dict[str, dict[str, object]]:
        try:
            cache_file = self._cache_file_path()
        except OSError:
            return {}
        if not cache_file.exists():
            return {}
        try:
            payload = json.loads(cache_file.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError):
            return {}
        if not isinstance(payload, dict) or payload.get("schemaVersion") != CACHE_SCHEMA_VERSION:
            return {}
        entries = payload.get("entries")
        if not isinstance(entries, dict):
            return {}
        return {key: value for key, value in entries.items() if isinstance(key, str) and isinstance(value, dict)}

    def _write_cached_entries(self, entries: dict[str, dict[str, object]]) -> None:
        try:
            cache_file = self._cache_file_path()
        except OSError:
            return
        payload = {
            "schemaVersion": CACHE_SCHEMA_VERSION,
            "entries": entries,
        }
        try:
            cache_file.write_text(json.dumps(payload, separators=(",", ":"), sort_keys=True), encoding="utf-8")
        except OSError:
            return

    def _persist_cache_snapshot(self) -> None:
        if not self._cache_snapshot_complete:
            return
        self._write_cached_entries(self._cache_entries)


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


def default_index_cache_dir() -> Path:
    return Path.home() / ".cache" / "sage-vscode-plugin" / "lsp-index-v1"


def _resolve_cache_dir(preferred: Path) -> Optional[Path]:
    for candidate in (
        preferred,
        Path(tempfile.gettempdir()) / "sage-vscode-plugin" / "lsp-index-v1",
    ):
        try:
            candidate.mkdir(parents=True, exist_ok=True)
            return candidate
        except OSError:
            continue
    return None


def file_fingerprint(path: Path) -> dict[str, int]:
    stat = path.stat()
    return {
        "mtimeNs": stat.st_mtime_ns,
        "size": stat.st_size,
    }


def _cache_entry_matches(
    entry: Optional[dict[str, object]],
    module_name: str,
    fingerprint: dict[str, int],
) -> bool:
    if not isinstance(entry, dict):
        return False
    if entry.get("moduleName") != module_name:
        return False
    cached_fingerprint = entry.get("fingerprint")
    return cached_fingerprint == fingerprint and isinstance(entry.get("record"), dict)


def serialize_module_record(record: ModuleRecord) -> dict[str, object]:
    return {
        "language": record.language,
        "docstring": record.docstring,
        "symbols": {name: serialize_symbol_record(symbol) for name, symbol in record.symbols.items()},
        "bindings": {name: serialize_import_binding(binding) for name, binding in record.bindings.items()},
        "starImports": list(record.star_imports),
        "memberSymbols": {
            owner_name: {name: serialize_symbol_record(symbol) for name, symbol in symbols.items()}
            for owner_name, symbols in record.member_symbols.items()
        },
        "memberBindings": {
            owner_name: {name: serialize_import_binding(binding) for name, binding in bindings.items()}
            for owner_name, bindings in record.member_bindings.items()
        },
        "instanceTypes": dict(record.instance_types),
        "diagnostics": list(record.diagnostics),
    }


def deserialize_module_record(
    payload: object,
    module_name: str,
    file_path: Path,
    source: str,
) -> ModuleRecord:
    if not isinstance(payload, dict):
        return parse_module(module_name, file_path, source)

    return ModuleRecord(
        module_name=module_name,
        file_path=file_path,
        language=str(payload.get("language", "python")),
        source=source,
        docstring=payload.get("docstring") if isinstance(payload.get("docstring"), str) else None,
        symbols={
            name: deserialize_symbol_record(symbol_payload, module_name, file_path)
            for name, symbol_payload in (payload.get("symbols", {}) or {}).items()
            if isinstance(name, str)
        },
        bindings={
            name: deserialize_import_binding(binding_payload)
            for name, binding_payload in (payload.get("bindings", {}) or {}).items()
            if isinstance(name, str)
        },
        star_imports=[
            value for value in (payload.get("starImports", []) or []) if isinstance(value, str)
        ],
        member_symbols={
            owner_name: {
                name: deserialize_symbol_record(symbol_payload, module_name, file_path)
                for name, symbol_payload in symbols_payload.items()
                if isinstance(name, str)
            }
            for owner_name, symbols_payload in (payload.get("memberSymbols", {}) or {}).items()
            if isinstance(owner_name, str) and isinstance(symbols_payload, dict)
        },
        member_bindings={
            owner_name: {
                name: deserialize_import_binding(binding_payload)
                for name, binding_payload in bindings_payload.items()
                if isinstance(name, str)
            }
            for owner_name, bindings_payload in (payload.get("memberBindings", {}) or {}).items()
            if isinstance(owner_name, str) and isinstance(bindings_payload, dict)
        },
        instance_types={
            name: value
            for name, value in (payload.get("instanceTypes", {}) or {}).items()
            if isinstance(name, str) and isinstance(value, str)
        },
        diagnostics=[
            entry for entry in (payload.get("diagnostics", []) or []) if isinstance(entry, dict)
        ],
    )


def serialize_symbol_record(symbol: SymbolRecord) -> dict[str, object]:
    return {
        "name": symbol.name,
        "kind": symbol.kind,
        "moduleName": symbol.module_name,
        "sourceRange": serialize_source_range(symbol.source_range),
        "detail": symbol.detail,
        "docstring": symbol.docstring,
    }


def deserialize_symbol_record(
    payload: object,
    module_name: str,
    file_path: Path,
) -> SymbolRecord:
    if not isinstance(payload, dict):
        return SymbolRecord(
            name="",
            kind="variable",
            module_name=module_name,
            file_path=file_path,
            source_range=SourceRange.from_offsets(1, 0, 1, 0),
        )
    return SymbolRecord(
        name=str(payload.get("name", "")),
        kind=str(payload.get("kind", "variable")),
        module_name=str(payload.get("moduleName", module_name)),
        file_path=file_path,
        source_range=deserialize_source_range(payload.get("sourceRange")),
        detail=str(payload.get("detail", "")),
        docstring=payload.get("docstring") if isinstance(payload.get("docstring"), str) else None,
    )


def serialize_import_binding(binding: ImportBinding) -> dict[str, object]:
    return {
        "alias": binding.alias,
        "moduleName": binding.module_name,
        "targetName": binding.target_name,
        "sourceRange": serialize_source_range(binding.source_range),
        "isLazy": binding.is_lazy,
    }


def deserialize_import_binding(payload: object) -> ImportBinding:
    if not isinstance(payload, dict):
        return ImportBinding(
            alias="",
            module_name="",
            target_name=None,
            source_range=SourceRange.from_offsets(1, 0, 1, 0),
        )
    return ImportBinding(
        alias=str(payload.get("alias", "")),
        module_name=str(payload.get("moduleName", "")),
        target_name=payload.get("targetName") if isinstance(payload.get("targetName"), str) else None,
        source_range=deserialize_source_range(payload.get("sourceRange")),
        is_lazy=bool(payload.get("isLazy", False)),
    )


def serialize_source_range(source_range: SourceRange) -> dict[str, int]:
    return {
        "startLine": source_range.start.line,
        "startCharacter": source_range.start.character,
        "endLine": source_range.end.line,
        "endCharacter": source_range.end.character,
    }


def deserialize_source_range(payload: object) -> SourceRange:
    if not isinstance(payload, dict):
        return SourceRange.from_offsets(1, 0, 1, 0)
    return SourceRange.from_offsets(
        int(payload.get("startLine", 0)) + 1,
        int(payload.get("startCharacter", 0)),
        int(payload.get("endLine", 0)) + 1,
        int(payload.get("endCharacter", 0)),
    )


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
