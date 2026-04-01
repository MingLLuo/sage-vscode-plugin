from __future__ import annotations

from concurrent.futures import ThreadPoolExecutor
import fnmatch
from dataclasses import dataclass
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
from typing import Optional
from urllib.parse import unquote, urlparse

from .model import ImportBinding, ModuleRecord, SourceRange, SymbolRecord, document_symbol_kind
from .parser import parse_module


CACHE_SCHEMA_VERSION = 2
SUMMARY_CACHE_SCHEMA_VERSION = 1
QUERY_SCAN_PARALLEL_THRESHOLD = 128
QUERY_SCAN_MAX_WORKERS = 8
RIPGREP_TIMEOUT_SECONDS = 10


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


@dataclass(frozen=True)
class ModuleSymbolSummary:
    name: str
    kind: str
    module_name: str
    file_path: Path
    source_range: SourceRange
    container_name: str = ""

    def workspace_symbol_item(self) -> dict[str, object]:
        return {
            "name": self.name,
            "kind": document_symbol_kind(self.kind),
            "location": {
                "uri": self.file_path.as_uri(),
                "range": self.source_range.to_lsp(),
            },
            "containerName": self.container_name or self.module_name,
        }


@dataclass
class ModuleSummary:
    module_name: str
    file_path: Path
    exports: frozenset[str]
    symbols: tuple[ModuleSymbolSummary, ...]


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
        self._module_summaries: dict[str, ModuleSummary] = {}
        self._module_paths: dict[Path, str] = {}
        self._summary_paths: dict[Path, str] = {}
        self._module_component_paths: dict[str, set[Path]] = {}
        self._module_records_by_path: dict[Path, ModuleRecord] = {}
        self._exact_export_index: dict[str, set[str]] = {}
        self._resolved_symbol_cache: dict[tuple[str, str], Optional[SymbolRecord]] = {}
        self._resolved_member_cache: dict[tuple[str, str, str], Optional[SymbolRecord]] = {}
        self._document_records: dict[str, tuple[str, str, ModuleRecord]] = {}
        self._query_summary_cache: dict[str, dict[str, ModuleSummary]] = {}
        self._query_import_candidate_cache: dict[str, list[str]] = {}
        self._cache_entries: dict[str, dict[str, object]] = {}
        self._summary_cache_entries: dict[str, dict[str, object]] = {}
        self._summary_cache_loaded = False
        self._summary_cache_complete = False
        self._summary_cache_dirty = False
        self._summary_cache_persisted_complete: Optional[bool] = None
        self._cache_snapshot_complete = False
        self._loaded_roots: set[Path] = set()
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
        self._summary_cache_complete = True
        self._persist_summary_cache()
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
        self._summary_cache_complete = restored_any
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
            self._summary_cache_complete = True
        self._clear_resolution_caches()
        if persist_snapshot:
            self._persist_summary_cache()
            self._persist_cache_snapshot()

    def ensure_full_index(self) -> None:
        if self._fully_indexed:
            return
        missing_roots = [root for root in self._source_roots if root.resolve() not in self._loaded_roots]
        if not missing_roots:
            self._fully_indexed = True
            self._cache_snapshot_complete = True
            self._persist_cache_snapshot()
            return
        self.load_roots(missing_roots, mark_fully_indexed=True, persist_snapshot=True)

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
            self._query_summary_cache.clear()
            self._query_import_candidate_cache.clear()
            self._persist_summary_cache()
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
            self._query_summary_cache.clear()
            self._query_import_candidate_cache.clear()
            self._persist_summary_cache()
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
            self._query_summary_cache.clear()
            self._query_import_candidate_cache.clear()
            self._persist_summary_cache()
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
        existed = cache_key in self._cache_entries or indexed_path in self._module_paths or indexed_path in self._summary_paths
        self._cache_entries.pop(cache_key, None)
        self._remove_summary_cache_entry(indexed_path)

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
                    self._drop_module_summary(module_name)
        if module_name is None:
            summary_module_name = self._summary_paths.pop(indexed_path, None)
            if summary_module_name is not None and summary_module_name not in self._modules:
                self._drop_module_summary(summary_module_name)
        return existed

    def resolve_symbol(self, record: ModuleRecord, name: str) -> Optional[SymbolRecord]:
        cache_key = self._symbol_cache_key(record, name)
        if cache_key is not None and cache_key in self._resolved_symbol_cache:
            return self._resolved_symbol_cache[cache_key]

        resolved = self._resolve_symbol(record, name, visited=set())
        if cache_key is not None:
            self._resolved_symbol_cache[cache_key] = resolved
        return resolved

    def exported_symbols(
        self,
        module_name: str,
        visited: Optional[set[str]] = None,
    ) -> dict[str, SymbolRecord]:
        record = self._modules.get(module_name) or self._ensure_module_loaded(module_name)
        if record is None:
            return {}
        results: dict[str, SymbolRecord] = {}
        for name in self._visible_names(record, visited):
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
        if needle:
            fast_summaries = self._query_summaries_for_query(needle)
            if fast_summaries:
                return _workspace_symbol_items_from_summaries(
                    _merge_query_summaries(self._module_summaries, fast_summaries),
                    needle,
                )
            self._load_modules_matching_query(needle)
        else:
            self.ensure_full_index()
        return _workspace_symbol_items_from_summaries(self._module_summaries, needle)

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
        stripped_name = name.strip()
        if not stripped_name:
            return []
        fast_summaries = self._query_summaries_for_query(stripped_name.casefold())
        if fast_summaries:
            return _import_candidates_from_summaries(
                stripped_name,
                _merge_query_summaries(self._module_summaries, fast_summaries),
                exclude_module=exclude_module,
                loaded_modules=self._modules,
                fully_indexed=self._fully_indexed,
            )
        if not self._fully_indexed and not self._summary_cache_complete:
            fast_modules = self._query_import_candidate_cache.get(stripped_name)
            if fast_modules is None:
                fast_modules = self._ripgrep_import_candidate_modules(stripped_name, self._deferred_roots())
                if fast_modules is not None:
                    self._query_import_candidate_cache[stripped_name] = fast_modules
            if fast_modules:
                return _rank_candidate_modules(
                    fast_modules,
                    stripped_name,
                    exclude_module=exclude_module,
                    loaded_modules=self._modules,
                )
        self._load_modules_matching_query(stripped_name.casefold())
        return _import_candidates_from_summaries(
            stripped_name,
            self._module_summaries,
            exclude_module=exclude_module,
            loaded_modules=self._modules,
            fully_indexed=self._fully_indexed,
            exact_export_index=self._exact_export_index,
            visible_name_resolver=self._visible_names,
        )

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

    def _visible_names(
        self,
        record: ModuleRecord,
        visited: Optional[set[str]] = None,
    ) -> set[str]:
        names = set(record.symbols)
        names.update(record.bindings)
        next_visited = set(visited or ())
        next_visited.add(record.module_name)
        for star_import in record.star_imports:
            if star_import in next_visited:
                continue
            names.update(self.exported_symbols(star_import, next_visited))
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
        self._module_summaries.clear()
        self._module_paths.clear()
        self._summary_paths.clear()
        self._module_component_paths.clear()
        self._module_records_by_path.clear()
        self._exact_export_index.clear()
        self._resolved_symbol_cache.clear()
        self._resolved_member_cache.clear()
        self._document_records.clear()
        self._query_summary_cache.clear()
        self._query_import_candidate_cache.clear()
        self._cache_entries = {}
        self._summary_cache_entries = {}
        self._summary_cache_loaded = False
        self._summary_cache_complete = False
        self._summary_cache_dirty = False
        self._summary_cache_persisted_complete = None
        self._cache_snapshot_complete = False
        self._loaded_roots.clear()
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
        loaded_roots = set() if reset_cache_entries else set(self._loaded_roots)
        for root in roots:
            loaded_roots.add(root.resolve())
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
        self._loaded_roots = loaded_roots

    def _load_modules_matching_query(self, needle: str) -> None:
        if self._fully_indexed or not needle:
            return

        matched_from_cache = self._load_summary_cache_matches(needle)
        if self._summary_cache_complete:
            return
        if matched_from_cache:
            self._persist_summary_cache()
            return

        ripgrep_matches = self._ripgrep_candidate_paths_for_query(needle, self._deferred_roots())
        if ripgrep_matches:
            candidates = self._query_candidates_from_paths(ripgrep_matches)
        else:
            candidates = []
            for root, path, module_name in self._iter_indexable_modules_for_roots(self._deferred_roots()):
                resolved_path = path.resolve()
                if resolved_path in self._module_paths or resolved_path in self._summary_paths:
                    continue
                if str(resolved_path) in self._summary_cache_entries:
                    continue
                candidates.append((module_name, path, resolved_path))

        loaded_any = False
        for module_name, resolved_path, fingerprint, summary in self._query_summaries_for_candidates(needle, candidates):
            self._store_module_summary(
                module_name,
                resolved_path,
                summary,
                fingerprint=fingerprint,
            )
            loaded_any = True
        if loaded_any:
            self._persist_summary_cache()

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
            self._drop_module_summary(module_name)
            return

        ordered_records = sorted(
            (self._module_records_by_path[path] for path in component_paths if path in self._module_records_by_path),
            key=lambda record: (module_record_precedence(record), str(record.file_path)),
        )
        if not ordered_records:
            self._modules.pop(module_name, None)
            self._drop_module_summary(module_name)
            return

        merged = ordered_records[0]
        for record in ordered_records[1:]:
            merged = merge_module_records(merged, record)
        self._modules[module_name] = merged
        merged_path = merged.file_path.resolve()
        cache_entry = self._cache_entries.get(str(merged_path))
        self._store_module_summary(
            module_name,
            merged_path,
            module_summary_from_record(merged),
            fingerprint=_cache_entry_fingerprint(cache_entry) or file_fingerprint(merged_path),
        )

    def _deferred_roots(self) -> list[Path]:
        return [root for root in self._source_roots if root.resolve() not in self._loaded_roots]

    def _store_module_summary(
        self,
        module_name: str,
        path: Path,
        summary: ModuleSummary,
        *,
        fingerprint: Optional[dict[str, int]] = None,
    ) -> None:
        existing = self._module_summaries.get(module_name)
        if existing is not None:
            for export_name in existing.exports:
                modules = self._exact_export_index.get(export_name)
                if modules is None:
                    continue
                modules.discard(module_name)
                if not modules:
                    self._exact_export_index.pop(export_name, None)
        self._module_summaries[module_name] = summary
        self._summary_paths[path] = module_name
        for export_name in summary.exports:
            self._exact_export_index.setdefault(export_name, set()).add(module_name)
        self._upsert_summary_cache_entry(path, module_name, summary, fingerprint)

    def _drop_module_summary(self, module_name: str) -> None:
        summary = self._module_summaries.pop(module_name, None)
        if summary is None:
            return
        stale_paths = [path for path, name in self._summary_paths.items() if name == module_name]
        for stale_path in stale_paths:
            self._summary_paths.pop(stale_path, None)
        for export_name in summary.exports:
            modules = self._exact_export_index.get(export_name)
            if modules is None:
                continue
            modules.discard(module_name)
            if not modules:
                self._exact_export_index.pop(export_name, None)

    def _load_summary_cache_matches(self, needle: str) -> bool:
        self._ensure_summary_cache_loaded()
        matched_any = False
        for cache_key, entry in list(self._summary_cache_entries.items()):
            path = Path(cache_key)
            if path in self._module_paths or path in self._summary_paths:
                continue
            root = self._source_root_for_path(path)
            if root is None or root.resolve() in self._loaded_roots:
                continue
            if not self._summary_entry_matches(entry, needle, path):
                continue
            module_name = entry.get("moduleName")
            if not isinstance(module_name, str):
                self._remove_summary_cache_entry(path)
                continue
            fingerprint = _cache_entry_fingerprint(entry)
            if fingerprint is None:
                self._remove_summary_cache_entry(path)
                continue
            if not path.exists() or not self._is_indexable_path(root, path):
                self._remove_summary_cache_entry(path)
                continue
            current_fingerprint = file_fingerprint(path)
            if current_fingerprint != fingerprint:
                source = self._read_module_source(path)
                rebuilt = summarize_module_source(module_name, path, source)
                self._store_module_summary(
                    module_name,
                    path,
                    rebuilt,
                    fingerprint=current_fingerprint,
                )
                matched_any = matched_any or needle in module_name.casefold() or _module_summary_matches(rebuilt, needle)
                continue
            summary = deserialize_module_summary(entry.get("summary"), module_name, path)
            self._store_module_summary(
                module_name,
                path,
                summary,
                fingerprint=fingerprint,
            )
            matched_any = True
        return matched_any

    def _summary_entry_matches(
        self,
        entry: dict[str, object],
        needle: str,
        path: Path,
    ) -> bool:
        module_name = entry.get("moduleName")
        if isinstance(module_name, str) and needle in module_name.casefold():
            return True
        if not isinstance(module_name, str):
            return False
        summary = deserialize_module_summary(entry.get("summary"), module_name, path)
        return _module_summary_matches(summary, needle)

    def _upsert_summary_cache_entry(
        self,
        path: Path,
        module_name: str,
        summary: ModuleSummary,
        fingerprint: Optional[dict[str, int]],
    ) -> None:
        if fingerprint is None:
            return
        self._ensure_summary_cache_loaded()
        cache_key = str(path.resolve())
        next_entry = {
            "moduleName": module_name,
            "fingerprint": fingerprint,
            "summary": serialize_module_summary(summary),
        }
        if self._summary_cache_entries.get(cache_key) == next_entry:
            return
        self._summary_cache_entries[cache_key] = next_entry
        self._summary_cache_dirty = True

    def _remove_summary_cache_entry(self, path: Path) -> None:
        self._ensure_summary_cache_loaded()
        if self._summary_cache_entries.pop(str(path.resolve()), None) is not None:
            self._summary_cache_dirty = True

    def _query_summaries_for_candidates(
        self,
        needle: str,
        candidates: list[tuple[str, Path, Path]],
    ) -> list[tuple[str, Path, dict[str, int], ModuleSummary]]:
        if not candidates:
            return []
        if len(candidates) < QUERY_SCAN_PARALLEL_THRESHOLD:
            return [
                match
                for candidate in candidates
                if (match := self._candidate_summary_for_query(needle, candidate)) is not None
            ]

        max_workers = min(QUERY_SCAN_MAX_WORKERS, max(1, os.cpu_count() or 1), len(candidates))
        with ThreadPoolExecutor(max_workers=max_workers) as executor:
            return [
                match
                for match in executor.map(
                    lambda candidate: self._candidate_summary_for_query(needle, candidate),
                    candidates,
                )
                if match is not None
            ]

    def _candidate_summary_for_query(
        self,
        needle: str,
        candidate: tuple[str, Path, Path],
    ) -> Optional[tuple[str, Path, dict[str, int], ModuleSummary]]:
        module_name, path, resolved_path = candidate
        module_haystack = module_name.casefold()
        cache_key = str(resolved_path)
        fingerprint = file_fingerprint(path)
        cached_entry = self._cache_entries.get(cache_key)
        source = self._source_for_module_path(path, cached_entry, fingerprint)
        if needle not in module_haystack and needle not in source.casefold():
            return None
        return (
            module_name,
            resolved_path,
            fingerprint,
            summarize_module_source(module_name, resolved_path, source),
        )

    def _query_specific_summaries_for_candidates(
        self,
        needle: str,
        candidates: list[tuple[str, Path, Path]],
    ) -> list[tuple[str, Path, dict[str, int], ModuleSummary]]:
        if not candidates:
            return []
        if len(candidates) < QUERY_SCAN_PARALLEL_THRESHOLD:
            return [
                match
                for candidate in candidates
                if (match := self._candidate_query_summary(needle, candidate)) is not None
            ]

        max_workers = min(QUERY_SCAN_MAX_WORKERS, max(1, os.cpu_count() or 1), len(candidates))
        with ThreadPoolExecutor(max_workers=max_workers) as executor:
            return [
                match
                for match in executor.map(
                    lambda candidate: self._candidate_query_summary(needle, candidate),
                    candidates,
                )
                if match is not None
            ]

    def _candidate_query_summary(
        self,
        needle: str,
        candidate: tuple[str, Path, Path],
    ) -> Optional[tuple[str, Path, dict[str, int], ModuleSummary]]:
        module_name, path, resolved_path = candidate
        cache_key = str(resolved_path)
        fingerprint = file_fingerprint(path)
        cached_entry = self._cache_entries.get(cache_key)
        source = self._source_for_module_path(path, cached_entry, fingerprint)
        summary = summarize_module_source_for_query(module_name, resolved_path, source, needle)
        if not summary.exports and not summary.symbols:
            return None
        return (module_name, resolved_path, fingerprint, summary)

    def _query_candidates_from_paths(
        self,
        paths: set[Path],
    ) -> list[tuple[str, Path, Path]]:
        candidates: list[tuple[str, Path, Path]] = []
        for resolved_path in sorted(path.resolve() for path in paths):
            if resolved_path in self._module_paths or resolved_path in self._summary_paths:
                continue
            if str(resolved_path) in self._summary_cache_entries:
                continue
            root = self._source_root_for_path(resolved_path)
            if root is None or not self._is_indexable_path(root, resolved_path):
                continue
            module_name = module_name_from_path(root, resolved_path)
            if not module_name:
                continue
            candidates.append((module_name, resolved_path, resolved_path))
        return candidates

    def _ripgrep_candidate_paths_for_query(
        self,
        needle: str,
        roots: list[Path],
    ) -> Optional[set[Path]]:
        rg_path = shutil.which("rg")
        if rg_path is None or not needle or not roots:
            return None
        command = [
            rg_path,
            "-l",
            "-i",
            "--regexp",
            _query_definition_pattern(needle),
            "--glob",
            "*.py",
            "--glob",
            "*.sage",
            "--glob",
            "*.pyx",
            "--glob",
            "*.pxd",
            "--glob",
            "*.pxi",
            *[str(root) for root in roots],
        ]
        try:
            completed = subprocess.run(
                command,
                capture_output=True,
                check=False,
                text=True,
                timeout=RIPGREP_TIMEOUT_SECONDS,
            )
        except (OSError, subprocess.SubprocessError):
            return None
        if completed.returncode not in {0, 1}:
            return None
        matches: set[Path] = set()
        for line in completed.stdout.splitlines():
            if not line.strip():
                continue
            path = Path(line.strip()).resolve()
            root = self._source_root_for_path(path)
            if root is None or not self._is_indexable_path(root, path):
                continue
            matches.add(path)
        return matches

    def _query_summaries_for_query(self, needle: str) -> Optional[dict[str, ModuleSummary]]:
        if not needle or self._fully_indexed:
            return None
        self._ensure_summary_cache_loaded()
        if self._summary_cache_complete:
            return None
        cached = self._query_summary_cache.get(needle)
        if cached is not None:
            return cached
        paths = self._ripgrep_candidate_paths_for_query(needle, self._deferred_roots())
        if not paths:
            return None
        summaries = self._query_summaries_from_paths(needle, paths)
        if summaries:
            self._query_summary_cache[needle] = summaries
            return summaries
        return None

    def _query_summaries_from_paths(
        self,
        needle: str,
        paths: set[Path],
    ) -> Optional[dict[str, ModuleSummary]]:
        candidates = self._query_candidates_from_paths(paths)
        if not candidates:
            return None
        summaries = self._query_specific_summaries_for_candidates(needle, candidates)
        if not summaries:
            return None
        return {
            module_name: summary
            for module_name, _, _, summary in summaries
        }

    def _ripgrep_import_candidate_modules(
        self,
        name: str,
        roots: list[Path],
    ) -> Optional[list[str]]:
        rg_path = shutil.which("rg")
        if rg_path is None or not name or not roots:
            return None
        command = [
            rg_path,
            "-l",
            "-i",
            "--fixed-strings",
            "--glob",
            "*.py",
            "--glob",
            "*.sage",
            "--glob",
            "*.pyx",
            "--glob",
            "*.pxd",
            "--glob",
            "*.pxi",
            name,
            *[str(root) for root in roots],
        ]
        try:
            completed = subprocess.run(
                command,
                capture_output=True,
                check=False,
                text=True,
                timeout=RIPGREP_TIMEOUT_SECONDS,
            )
        except (OSError, subprocess.SubprocessError):
            return None
        if completed.returncode not in {0, 1}:
            return None

        candidates: set[str] = set()
        name_folded = name.casefold()
        for path, line_text, line_number in self._iter_ripgrep_match_lines(completed.stdout):
            root = self._source_root_for_path(path)
            if root is None or not self._is_indexable_path(root, path):
                continue
            module_name = module_name_from_path(root, path)
            if not module_name:
                continue
            exports, _ = query_symbols_from_line(module_name, path, line_text, line_number, name_folded)
            if name in exports:
                candidates.add(module_name)
        return sorted(candidates)

    def _iter_ripgrep_match_lines(self, stdout: str) -> list[tuple[Path, str, int]]:
        matches: list[tuple[Path, str, int]] = []
        for raw_line in stdout.splitlines():
            if not raw_line:
                continue
            match = RIPGREP_VIMGREP_RE.match(raw_line)
            if match is None:
                continue
            try:
                line_number = int(match.group("line"))
            except ValueError:
                continue
            matches.append(
                (
                    Path(match.group("path")).resolve(),
                    match.group("text"),
                    line_number,
                )
            )
        return matches

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
        if restored_any:
            self._loaded_roots = {root.resolve() for root in self._source_roots}
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

    def _cache_digest(self) -> str:
        return hashlib.sha256(
            "\0".join(
                [
                    *[str(path.resolve()) for path in self._source_roots],
                    *self._excluded_globs,
                    f"enable_pyx={self._enable_pyx}",
                    f"schema={CACHE_SCHEMA_VERSION}",
                ]
            ).encode("utf-8")
        ).hexdigest()[:16]

    def _cache_file_path(self) -> Path:
        resolved_cache_dir = _resolve_cache_dir(self._cache_dir)
        if resolved_cache_dir is None:
            raise OSError("No writable cache directory available")
        return resolved_cache_dir / f"workspace-index-{self._cache_digest()}.json"

    def _summary_cache_file_path(self) -> Path:
        resolved_cache_dir = _resolve_cache_dir(self._cache_dir)
        if resolved_cache_dir is None:
            raise OSError("No writable cache directory available")
        return resolved_cache_dir / f"workspace-summary-{self._cache_digest()}.json"

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

    def _ensure_summary_cache_loaded(self) -> None:
        if self._summary_cache_loaded:
            return
        complete, entries = self._load_cached_summary_entries()
        self._summary_cache_entries = entries
        self._summary_cache_complete = self._summary_cache_complete or complete
        self._summary_cache_loaded = True
        self._summary_cache_dirty = False
        self._summary_cache_persisted_complete = complete

    def _load_cached_summary_entries(self) -> tuple[bool, dict[str, dict[str, object]]]:
        try:
            cache_file = self._summary_cache_file_path()
        except OSError:
            return False, {}
        if not cache_file.exists():
            return False, {}
        try:
            payload = json.loads(cache_file.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError):
            return False, {}
        if not isinstance(payload, dict) or payload.get("schemaVersion") != SUMMARY_CACHE_SCHEMA_VERSION:
            return False, {}
        entries = payload.get("entries")
        if not isinstance(entries, dict):
            return False, {}
        return (
            bool(payload.get("complete", False)),
            {key: value for key, value in entries.items() if isinstance(key, str) and isinstance(value, dict)},
        )

    def _write_cached_summary_entries(
        self,
        complete: bool,
        entries: dict[str, dict[str, object]],
    ) -> None:
        try:
            cache_file = self._summary_cache_file_path()
        except OSError:
            return
        payload = {
            "schemaVersion": SUMMARY_CACHE_SCHEMA_VERSION,
            "complete": complete,
            "entries": entries,
        }
        try:
            cache_file.write_text(json.dumps(payload, separators=(",", ":"), sort_keys=True), encoding="utf-8")
        except OSError:
            return

    def _persist_summary_cache(self) -> None:
        self._ensure_summary_cache_loaded()
        if self._fully_indexed:
            self._summary_cache_complete = True
        if (
            not self._summary_cache_dirty
            and self._summary_cache_persisted_complete == self._summary_cache_complete
        ):
            return
        self._write_cached_summary_entries(self._summary_cache_complete, self._summary_cache_entries)
        self._summary_cache_dirty = False
        self._summary_cache_persisted_complete = self._summary_cache_complete


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


def serialize_module_summary(summary: ModuleSummary) -> dict[str, object]:
    return {
        "exports": sorted(summary.exports),
        "symbols": [serialize_module_symbol_summary(symbol) for symbol in summary.symbols],
    }


SUMMARY_CLASS_RE = re.compile(
    r"^(?P<indent>\s*)(?:class|cdef\s+class|cpdef\s+class)\s+(?P<name>[A-Za-z_][A-Za-z0-9_]*)"
)
SUMMARY_FUNCTION_RE = re.compile(
    r"^(?P<indent>\s*)(?:async\s+def|def|cpdef|cdef)(?:\s+(?:inline|api|public|readonly|nogil|gil|except|const|unsigned|signed|long|short|char|int|float|double|void|object|bint|size_t|Py_ssize_t|[A-Za-z_][A-Za-z0-9_\.\*\[\]]*))*\s+(?P<name>[A-Za-z_][A-Za-z0-9_]*)\s*\("
)
SUMMARY_ASSIGN_RE = re.compile(r"^(?P<name>[A-Za-z_][A-Za-z0-9_]*)\s*=")
SUMMARY_FROM_IMPORT_RE = re.compile(r"^from\s+(?P<module>[A-Za-z_][A-Za-z0-9_\.]*)\s+import\s+(?P<targets>.+)$")
SUMMARY_IMPORT_RE = re.compile(r"^import\s+(?P<targets>.+)$")
SUMMARY_LAZY_IMPORT_RE = re.compile(
    r"""lazy_import\(\s*["'][^"']+["']\s*,\s*["'](?P<target>[A-Za-z_][A-Za-z0-9_]*)["'](?:\s*,\s*["'](?P<alias>[A-Za-z_][A-Za-z0-9_]*)["'])?"""
)
RIPGREP_VIMGREP_RE = re.compile(r"^(?P<path>.*):(?P<line>\d+):(?P<column>\d+):(?P<text>.*)$")


def module_summary_from_record(record: ModuleRecord) -> ModuleSummary:
    symbols: list[ModuleSymbolSummary] = []
    export_names: set[str] = set()

    for name, symbol in sorted(record.symbols.items()):
        if name.startswith("_"):
            continue
        export_names.add(name)
        symbols.append(
            ModuleSymbolSummary(
                name=name,
                kind=symbol.kind,
                module_name=record.module_name,
                file_path=symbol.file_path,
                source_range=symbol.source_range,
            )
        )

    for name, binding in sorted(record.bindings.items()):
        if name.startswith("_"):
            continue
        export_names.add(name)
        symbols.append(
            ModuleSymbolSummary(
                name=name,
                kind="module" if binding.target_name is None else "variable",
                module_name=record.module_name,
                file_path=record.file_path,
                source_range=binding.source_range,
            )
        )

    for owner_name, member_symbols in sorted(record.member_symbols.items()):
        for name, symbol in sorted(member_symbols.items()):
            if name.startswith("_"):
                continue
            symbols.append(
                ModuleSymbolSummary(
                    name=name,
                    kind=symbol.kind,
                    module_name=record.module_name,
                    file_path=symbol.file_path,
                    source_range=symbol.source_range,
                    container_name=f"{record.module_name}.{owner_name}",
                )
            )

    for owner_name, member_bindings in sorted(record.member_bindings.items()):
        for name, binding in sorted(member_bindings.items()):
            if name.startswith("_"):
                continue
            symbols.append(
                ModuleSymbolSummary(
                    name=name,
                    kind="module" if binding.target_name is None else "variable",
                    module_name=record.module_name,
                    file_path=record.file_path,
                    source_range=binding.source_range,
                    container_name=f"{record.module_name}.{owner_name}",
                )
            )

    return ModuleSummary(
        module_name=record.module_name,
        file_path=record.file_path,
        exports=frozenset(export_names),
        symbols=tuple(symbols),
    )


def summarize_module_source(module_name: str, file_path: Path, source: str) -> ModuleSummary:
    exports: set[str] = set()
    symbols: list[ModuleSymbolSummary] = []
    class_stack: list[tuple[int, str]] = []

    for line_number, line in enumerate(source.splitlines(), start=1):
        stripped = line.lstrip()
        if not stripped or stripped.startswith("#"):
            continue
        indent = len(line) - len(stripped)
        while class_stack and indent <= class_stack[-1][0]:
            class_stack.pop()

        class_match = SUMMARY_CLASS_RE.match(line)
        if class_match is not None:
            name = class_match.group("name")
            if indent == 0 and not name.startswith("_"):
                exports.add(name)
                symbols.append(_summary_symbol(name, "class", module_name, file_path, line_number))
                class_stack.append((indent, name))
            continue

        function_match = SUMMARY_FUNCTION_RE.match(line)
        if function_match is not None:
            name = function_match.group("name")
            if name.startswith("_"):
                continue
            if class_stack:
                symbols.append(
                    _summary_symbol(
                        name,
                        "function",
                        module_name,
                        file_path,
                        line_number,
                        container_name=f"{module_name}.{class_stack[-1][1]}",
                    )
                )
            elif indent == 0:
                exports.add(name)
                symbols.append(_summary_symbol(name, "function", module_name, file_path, line_number))
            continue

        if indent != 0:
            continue

        from_import_match = SUMMARY_FROM_IMPORT_RE.match(stripped)
        if from_import_match is not None:
            for target in [item.strip() for item in from_import_match.group("targets").split(",")]:
                if not target or target == "*":
                    continue
                alias = target.split(" as ")[-1].strip()
                if alias.startswith("_"):
                    continue
                exports.add(alias)
                symbols.append(_summary_symbol(alias, "variable", module_name, file_path, line_number))
            continue

        import_match = SUMMARY_IMPORT_RE.match(stripped)
        if import_match is not None:
            for target in [item.strip() for item in import_match.group("targets").split(",")]:
                if not target:
                    continue
                alias = target.split(" as ")[-1].strip().split(".")[0]
                if alias.startswith("_"):
                    continue
                exports.add(alias)
                symbols.append(_summary_symbol(alias, "module", module_name, file_path, line_number))
            continue

        lazy_import_match = SUMMARY_LAZY_IMPORT_RE.search(stripped)
        if lazy_import_match is not None:
            alias = lazy_import_match.group("alias") or lazy_import_match.group("target")
            if not alias.startswith("_"):
                exports.add(alias)
                symbols.append(_summary_symbol(alias, "variable", module_name, file_path, line_number))
            continue

        assign_match = SUMMARY_ASSIGN_RE.match(stripped)
        if assign_match is not None:
            name = assign_match.group("name")
            if name.startswith("_"):
                continue
            exports.add(name)
            kind = "constant" if name.isupper() else "variable"
            symbols.append(_summary_symbol(name, kind, module_name, file_path, line_number))

    return ModuleSummary(
        module_name=module_name,
        file_path=file_path,
        exports=frozenset(exports),
        symbols=tuple(symbols),
    )


def summarize_module_source_for_query(
    module_name: str,
    file_path: Path,
    source: str,
    needle: str,
) -> ModuleSummary:
    exports: set[str] = set()
    symbols: dict[tuple[str, int, int, int, int], ModuleSymbolSummary] = {}
    needle_folded = needle.casefold()

    for line_number, line in enumerate(source.splitlines(), start=1):
        if needle_folded not in line.casefold():
            continue
        line_exports, line_symbols = query_symbols_from_line(
            module_name,
            file_path,
            line,
            line_number,
            needle_folded,
        )
        exports.update(line_exports)
        for symbol in line_symbols:
            key = (
                symbol.name,
                symbol.source_range.start.line,
                symbol.source_range.start.character,
                symbol.source_range.end.line,
                symbol.source_range.end.character,
            )
            symbols[key] = symbol

    return ModuleSummary(
        module_name=module_name,
        file_path=file_path,
        exports=frozenset(exports),
        symbols=tuple(symbols.values()),
    )


def _summary_symbol(
    name: str,
    kind: str,
    module_name: str,
    file_path: Path,
    line_number: int,
    *,
    container_name: str = "",
) -> ModuleSymbolSummary:
    return ModuleSymbolSummary(
        name=name,
        kind=kind,
        module_name=module_name,
        file_path=file_path,
        source_range=SourceRange.from_offsets(line_number, 0, line_number, max(len(name), 1)),
        container_name=container_name,
    )


def serialize_module_symbol_summary(symbol: ModuleSymbolSummary) -> dict[str, object]:
    return {
        "name": symbol.name,
        "kind": symbol.kind,
        "containerName": symbol.container_name,
        "sourceRange": serialize_source_range(symbol.source_range),
    }


def deserialize_module_summary(
    payload: object,
    module_name: str,
    file_path: Path,
) -> ModuleSummary:
    if not isinstance(payload, dict):
        return ModuleSummary(module_name=module_name, file_path=file_path, exports=frozenset(), symbols=())
    exports_payload = payload.get("exports")
    symbols_payload = payload.get("symbols")
    exports = frozenset(
        export_name
        for export_name in (exports_payload or [])
        if isinstance(export_name, str) and not export_name.startswith("_")
    )
    symbols = tuple(
        deserialize_module_symbol_summary(symbol_payload, module_name, file_path)
        for symbol_payload in (symbols_payload or [])
        if isinstance(symbol_payload, dict)
    )
    return ModuleSummary(
        module_name=module_name,
        file_path=file_path,
        exports=exports,
        symbols=symbols,
    )


def deserialize_module_symbol_summary(
    payload: object,
    module_name: str,
    file_path: Path,
) -> ModuleSymbolSummary:
    if not isinstance(payload, dict):
        return ModuleSymbolSummary(
            name="",
            kind="variable",
            module_name=module_name,
            file_path=file_path,
            source_range=SourceRange.from_offsets(1, 0, 1, 0),
        )
    return ModuleSymbolSummary(
        name=str(payload.get("name", "")),
        kind=str(payload.get("kind", "variable")),
        module_name=module_name,
        file_path=file_path,
        source_range=deserialize_source_range(payload.get("sourceRange")),
        container_name=str(payload.get("containerName", "")),
    )


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


def _cache_entry_fingerprint(entry: Optional[dict[str, object]]) -> Optional[dict[str, int]]:
    if not isinstance(entry, dict):
        return None
    fingerprint = entry.get("fingerprint")
    if not isinstance(fingerprint, dict):
        return None
    mtime_ns = fingerprint.get("mtimeNs")
    size = fingerprint.get("size")
    if not isinstance(mtime_ns, int) or not isinstance(size, int):
        return None
    return {"mtimeNs": mtime_ns, "size": size}


def _module_summary_matches(summary: ModuleSummary, needle: str) -> bool:
    for symbol in summary.symbols:
        haystack = f"{symbol.name} {symbol.container_name} {summary.module_name}".casefold()
        if needle in haystack:
            return True
    return False


def _workspace_symbol_items_from_summaries(
    summaries: dict[str, ModuleSummary],
    needle: str,
) -> list[dict[str, object]]:
    items: list[dict[str, object]] = []
    seen: set[tuple[str, int, int, int, int]] = set()
    for module_name in sorted(summaries):
        summary = summaries[module_name]
        for symbol in summary.symbols:
            haystack = f"{symbol.name} {symbol.container_name} {module_name}".casefold()
            if needle and needle not in haystack:
                continue
            location_key = (
                str(symbol.file_path),
                symbol.source_range.start.line,
                symbol.source_range.start.character,
                symbol.source_range.end.line,
                symbol.source_range.end.character,
            )
            if location_key in seen:
                continue
            seen.add(location_key)
            items.append(symbol.workspace_symbol_item())
    return items[:200]


def _import_candidates_from_summaries(
    name: str,
    summaries: dict[str, ModuleSummary],
    *,
    exclude_module: Optional[str],
    loaded_modules: dict[str, ModuleRecord],
    fully_indexed: bool,
    exact_export_index: Optional[dict[str, set[str]]] = None,
    visible_name_resolver=None,
) -> list[str]:
    candidate_modules: set[str]
    if exact_export_index is not None:
        candidate_modules = set(exact_export_index.get(name, set()))
    else:
        candidate_modules = set()
    for module_name, summary in summaries.items():
        if name in summary.exports:
            candidate_modules.add(module_name)
    if fully_indexed and visible_name_resolver is not None:
        for module_name, record in list(loaded_modules.items()):
            if module_name in candidate_modules or not record.star_imports:
                continue
            if name in visible_name_resolver(record):
                candidate_modules.add(module_name)
    candidates: list[tuple[int, str]] = []
    for module_name in sorted(candidate_modules):
        if module_name == exclude_module:
            continue
        score = 2
        if module_name in {"sage.all", "sage.all_cmdline"}:
            score = 1
        elif module_name in loaded_modules and name in loaded_modules[module_name].symbols:
            score = 0
        candidates.append((score, module_name))
    return [module_name for _, module_name in sorted(dict.fromkeys(candidates))]


def _rank_candidate_modules(
    module_names: list[str],
    name: str,
    *,
    exclude_module: Optional[str],
    loaded_modules: dict[str, ModuleRecord],
) -> list[str]:
    candidates: list[tuple[int, str]] = []
    for module_name in sorted(dict.fromkeys(module_names)):
        if module_name == exclude_module:
            continue
        score = 2
        if module_name in {"sage.all", "sage.all_cmdline"}:
            score = 1
        elif module_name in loaded_modules and name in loaded_modules[module_name].symbols:
            score = 0
        candidates.append((score, module_name))
    return [module_name for _, module_name in sorted(candidates)]


def _merge_query_summaries(
    persisted: dict[str, ModuleSummary],
    query_summaries: dict[str, ModuleSummary],
) -> dict[str, ModuleSummary]:
    merged = dict(persisted)
    for module_name, summary in query_summaries.items():
        if module_name not in merged:
            merged[module_name] = summary
            continue
        existing = merged[module_name]
        combined_exports = set(existing.exports)
        combined_exports.update(summary.exports)
        combined_symbols: dict[tuple[str, int, int, int, int], ModuleSymbolSummary] = {}
        for symbol in existing.symbols + summary.symbols:
            key = (
                symbol.name,
                symbol.source_range.start.line,
                symbol.source_range.start.character,
                symbol.source_range.end.line,
                symbol.source_range.end.character,
            )
            combined_symbols[key] = symbol
        merged[module_name] = ModuleSummary(
            module_name=existing.module_name,
            file_path=existing.file_path,
            exports=frozenset(combined_exports),
            symbols=tuple(combined_symbols.values()),
        )
    return merged


def _query_definition_pattern(needle: str) -> str:
    token = f"[A-Za-z0-9_]*{re.escape(needle)}[A-Za-z0-9_]*"
    return "|".join(
        (
            rf"^\s*(?:class|cdef\s+class|cpdef\s+class)\s+{token}\b",
            rf"^\s*(?:async\s+def|def|cpdef|cdef)\b.*\b{token}\s*\(",
            rf"^\s*from\s+[A-Za-z_][A-Za-z0-9_\.]*\s+import\s+.*\b{token}\b",
            rf"^\s*import\s+.*\b{token}\b",
            rf"lazy_import\(\s*['\"][^'\"]+['\"]\s*,\s*['\"]{token}['\"]",
            rf"lazy_import\(\s*['\"][^'\"]+['\"]\s*,\s*['\"][A-Za-z_][A-Za-z0-9_]*['\"]\s*,\s*['\"]{token}['\"]",
            rf"^\s*{token}\s*=",
        )
    )


def query_symbols_from_line(
    module_name: str,
    file_path: Path,
    line: str,
    line_number: int,
    needle: str,
) -> tuple[set[str], tuple[ModuleSymbolSummary, ...]]:
    stripped = line.rstrip("\n")
    exports: set[str] = set()
    symbols: list[ModuleSymbolSummary] = []
    needle_folded = needle.casefold()

    class_match = SUMMARY_CLASS_RE.match(stripped)
    if class_match is not None:
        name = class_match.group("name")
        indent = len(class_match.group("indent") or "")
        if needle_folded in name.casefold():
            symbols.append(_summary_symbol(name, "class", module_name, file_path, line_number))
            if indent == 0:
                exports.add(name)
        return exports, tuple(symbols)

    function_match = SUMMARY_FUNCTION_RE.match(stripped)
    if function_match is not None:
        name = function_match.group("name")
        indent = len(function_match.group("indent") or "")
        if needle_folded in name.casefold():
            symbols.append(_summary_symbol(name, "function", module_name, file_path, line_number))
            if indent == 0:
                exports.add(name)
        return exports, tuple(symbols)

    from_import_match = SUMMARY_FROM_IMPORT_RE.match(stripped.lstrip())
    if from_import_match is not None:
        for target in [item.strip() for item in from_import_match.group("targets").split(",")]:
            if not target or target == "*":
                continue
            alias = target.split(" as ")[-1].strip()
            if alias.startswith("_") or needle_folded not in alias.casefold():
                continue
            exports.add(alias)
            symbols.append(_summary_symbol(alias, "variable", module_name, file_path, line_number))
        return exports, tuple(symbols)

    import_match = SUMMARY_IMPORT_RE.match(stripped.lstrip())
    if import_match is not None:
        for target in [item.strip() for item in import_match.group("targets").split(",")]:
            if not target:
                continue
            alias = target.split(" as ")[-1].strip().split(".")[0]
            if alias.startswith("_") or needle_folded not in alias.casefold():
                continue
            exports.add(alias)
            symbols.append(_summary_symbol(alias, "module", module_name, file_path, line_number))
        return exports, tuple(symbols)

    lazy_import_match = SUMMARY_LAZY_IMPORT_RE.search(stripped)
    if lazy_import_match is not None:
        alias = lazy_import_match.group("alias") or lazy_import_match.group("target")
        if needle_folded in alias.casefold():
            exports.add(alias)
            symbols.append(_summary_symbol(alias, "variable", module_name, file_path, line_number))
        return exports, tuple(symbols)

    assign_match = SUMMARY_ASSIGN_RE.match(stripped.lstrip())
    if assign_match is not None:
        name = assign_match.group("name")
        if needle_folded in name.casefold():
            exports.add(name)
            kind = "constant" if name.isupper() else "variable"
            symbols.append(_summary_symbol(name, kind, module_name, file_path, line_number))
    return exports, tuple(symbols)


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
