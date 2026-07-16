use super::*;

impl WorkspaceIndex {
    pub fn file_for_path(&self, path: &Path) -> Option<IndexedFile> {
        let path = normalize_path(path.to_path_buf());
        if !path_is_under_roots(&path, &self.options.roots) {
            return None;
        }
        if let Ok(cache) = self.file_lookup_cache.lock() {
            if let Some(file) = cache.get(&path) {
                return Some(file.clone());
            }
        }
        let mut file = if let Some(file) = self.files.get(&path).cloned() {
            file
        } else if self.cached_file_count > 0 {
            load_file_from_db(&self.db_path, &path).ok()?
        } else {
            return None;
        };
        if file.symbols.is_empty() && self.cached_symbol_count > 0 {
            if let Ok(symbols) = load_symbols_for_path_from_db(&self.db_path, &path) {
                file.symbols = symbols;
            }
        }
        if let Ok(mut cache) = self.file_lookup_cache.lock() {
            cache.insert(path, file.clone());
        }
        Some(file)
    }

    pub fn fresh_file_for_query(&self, path: &Path) -> Option<IndexedFile> {
        let path = normalize_path(path.to_path_buf());
        if !path_is_under_roots(&path, &self.options.roots) {
            return None;
        }
        load_fresh_file_for_query_from_db(&self.db_path, &path)
            .ok()
            .flatten()
    }

    pub fn source_path_for_module(&self, module: &str) -> Option<PathBuf> {
        module_source_path_from_roots(module, &self.options.roots, self.options.enable_pyx)
    }

    pub fn diagnostics_for_source(&self, path: &Path, source: &str) -> Vec<DiagnosticRecord> {
        diagnostics_for_source(path, source)
    }

    pub fn references(&self, name: &str) -> Vec<ReferenceRecord> {
        self.references_matching(name, |_| true)
    }

    pub fn editable_references(&self, name: &str) -> Vec<ReferenceRecord> {
        if self.options.editable_roots.is_empty() {
            return self.references(name);
        }
        let pending_paths: BTreeSet<_> = self
            .pending_refresh_path_snapshot()
            .into_iter()
            .filter(|path| self.is_editable_path(path))
            .collect();
        if pending_paths.is_empty() {
            if let Ok(cache) = self.reference_lookup_cache.lock() {
                if let Some(references) = cache.get(name) {
                    return references.clone();
                }
            }
        }
        let mut results = Vec::new();
        let mut loaded_from_db = false;
        if self.cached_file_count > 0 || self.db_path.exists() {
            if let Ok(cached) =
                load_reference_spans_from_db(&self.db_path, name, &self.options.editable_roots)
            {
                loaded_from_db = true;
                results.extend(
                    cached
                        .into_iter()
                        .filter(|reference| !pending_paths.contains(&reference.path)),
                );
            }
        }
        if !loaded_from_db {
            for file in self.files.values() {
                if !self.is_editable_path(&file.path) {
                    continue;
                }
                if let Ok(source) = fs::read_to_string(&file.path) {
                    results.extend(references_in_source(&file.path, &source, name));
                }
            }
        }
        for path in &pending_paths {
            if let Ok(source) = fs::read_to_string(path) {
                results.extend(references_in_source(path, &source, name));
            }
        }
        let results = dedupe_reference_records(results);
        if pending_paths.is_empty() {
            if let Ok(mut cache) = self.reference_lookup_cache.lock() {
                cache.insert(name.to_string(), results.clone());
            }
        }
        results
    }

    fn references_matching<F>(&self, name: &str, include_path: F) -> Vec<ReferenceRecord>
    where
        F: Fn(&Path) -> bool + Sync,
    {
        let mut paths = BTreeSet::new();
        let mut persisted_paths = BTreeSet::new();
        if self.cached_file_count > 0 {
            if let Ok(cached_paths) = load_file_paths_from_db(&self.db_path, &self.options.roots) {
                persisted_paths.extend(cached_paths);
            }
            match load_filtered_file_paths_from_db(&self.db_path, &self.options.roots, name) {
                Ok(Some(filtered_paths)) => paths.extend(filtered_paths),
                _ => paths.extend(persisted_paths.iter().cloned()),
            }
        }
        // A filesystem event is marked before the replacement index is prepared. Keep those few
        // paths in the live scan until their refreshed filters have been persisted and installed.
        paths.extend(self.pending_refresh_path_snapshot());
        paths.extend(
            self.files
                .values()
                .filter(|file| !persisted_paths.contains(&file.path))
                .map(|file| file.path.clone()),
        );
        let results = paths
            .into_iter()
            .collect::<Vec<_>>()
            .par_iter()
            .filter(|path| include_path(path))
            .filter_map(|path| {
                let source = fs::read_to_string(path).ok()?;
                source
                    .contains(name)
                    .then(|| references_in_source(path, &source, name))
            })
            .flatten()
            .collect();
        dedupe_reference_records(results)
    }

    pub(super) fn effective_editable_roots(&self) -> Vec<PathBuf> {
        if self.options.editable_roots.is_empty() {
            self.options.roots.clone()
        } else {
            self.options.editable_roots.clone()
        }
    }

    pub fn is_editable_path(&self, path: &Path) -> bool {
        self.effective_editable_roots()
            .iter()
            .any(|root| path.starts_with(root))
    }

    pub(super) fn should_persist_reference_spans(&self, path: &Path) -> bool {
        !self.options.editable_roots.is_empty() && self.is_editable_path(path)
    }

    pub(super) fn reset_operation_timings(&mut self, operation: &str) {
        self.last_operation = Some(operation.to_string());
        self.last_index_ms = 0;
        match operation {
            "hydrate" => {
                self.last_hydrate_ms = 0;
            }
            "reconcile" => {
                self.last_reconcile_ms = 0;
                self.last_persist_ms = 0;
                self.last_hot_cache_ms = 0;
            }
            "rebuild" | "refresh" => {
                self.last_hydrate_ms = 0;
                self.last_reconcile_ms = 0;
                self.last_persist_ms = 0;
                self.last_hot_cache_ms = 0;
            }
            _ => {}
        }
    }

    pub(super) fn clear_lookup_cache(&self) {
        if let Ok(mut cache) = self.symbol_lookup_cache.lock() {
            cache.clear();
        }
        if let Ok(mut cache) = self.file_lookup_cache.lock() {
            cache.clear();
        }
        if let Ok(mut cache) = self.sage_method_lookup_cache.lock() {
            cache.clear();
        }
        if let Ok(mut cache) = self.reference_lookup_cache.lock() {
            cache.clear();
        }
    }

    pub(super) fn clear_lookup_cache_entries(&self, names: &BTreeSet<String>) {
        if let Ok(mut cache) = self.symbol_lookup_cache.lock() {
            for name in names {
                cache.remove(&name.to_ascii_lowercase());
            }
        }
        if let Ok(mut cache) = self.file_lookup_cache.lock() {
            cache.clear();
        }
        if let Ok(mut cache) = self.sage_method_lookup_cache.lock() {
            cache.clear();
        }
        if let Ok(mut cache) = self.reference_lookup_cache.lock() {
            cache.clear();
        }
    }

    pub(super) fn lookup_cache_len(&self) -> usize {
        self.symbol_lookup_cache
            .lock()
            .map(|cache| cache.len())
            .unwrap_or_default()
    }

    pub(super) fn sage_method_cache_stats(&self) -> SageMethodCacheStats {
        if !self.db_path.exists() {
            return SageMethodCacheStats::default();
        }
        load_sage_method_cache_stats_from_db(&self.db_path).unwrap_or_default()
    }

    pub(super) fn insert_sage_method_lookup_cache(
        &self,
        owner_type: SageOwnerType,
        member: &str,
        record: Option<SymbolRecord>,
    ) {
        if let Ok(mut cache) = self.sage_method_lookup_cache.lock() {
            cache.insert(sage_method_cache_key(owner_type, member), record);
        }
    }

    pub(super) fn prewarm_hot_symbol_cache(&self, include_dynamic_exports: bool) {
        let names = hot_sage_symbol_names();
        let names: Vec<_> = names.into_iter().collect();
        let mut grouped = if self.cached_symbol_count > 0 || self.db_path.exists() {
            load_materialized_sage_export_groups_by_names_from_db(
                &self.db_path,
                "sage.all",
                &names,
                &self.options.roots,
            )
            .unwrap_or_default()
        } else {
            HashMap::new()
        };
        if include_dynamic_exports && (self.cached_symbol_count > 0 || self.db_path.exists()) {
            for (name, records) in
                load_hot_sage_export_groups_from_db(&self.db_path, "sage.all", &self.options.roots)
                    .unwrap_or_default()
            {
                grouped.entry(name).or_default().extend(records);
            }
        }
        if include_dynamic_exports {
            for name in self.hot_sage_export_names_from_memory() {
                if let Some(memory_symbols) = self.symbols_by_name.get(&name.to_ascii_lowercase()) {
                    grouped
                        .entry(name.to_ascii_lowercase())
                        .or_default()
                        .extend(memory_symbols.clone());
                }
            }
        }
        for name in &names {
            if let Some(memory_symbols) = self.symbols_by_name.get(&name.to_ascii_lowercase()) {
                grouped
                    .entry(name.to_ascii_lowercase())
                    .or_default()
                    .extend(memory_symbols.clone());
            }
        }
        if let Ok(mut cache) = self.symbol_lookup_cache.lock() {
            for name in names {
                let key = name.to_ascii_lowercase();
                let symbols = grouped.remove(&key).unwrap_or_default();
                cache.insert(key, dedupe_symbol_records(symbols));
            }
        }
        self.prewarm_hot_sage_method_cache();
    }

    fn prewarm_hot_sage_method_cache(&self) {
        if !(self.cached_symbol_count > 0 || self.db_path.exists()) {
            return;
        }
        let keys = hot_sage_method_keys();
        let methods =
            load_materialized_sage_methods_from_db(&self.db_path, &keys, &self.options.roots)
                .unwrap_or_default();
        if methods.is_empty() {
            return;
        }
        if let Ok(mut cache) = self.sage_method_lookup_cache.lock() {
            for (owner_type, member, record) in methods {
                cache.insert(sage_method_cache_key(owner_type, &member), Some(record));
            }
        }
    }

    fn hot_sage_export_names_from_memory(&self) -> BTreeSet<String> {
        let mut names = BTreeSet::new();
        for symbol in self.files.values().flat_map(|file| &file.symbols) {
            if symbol.kind == SymbolKind::Import && module_is_sage_all_export_module(&symbol.module)
            {
                insert_import_symbol_hot_names(&mut names, symbol);
                if names.len() >= MAX_DYNAMIC_HOT_EXPORT_NAMES {
                    break;
                }
            }
        }
        names
    }
}
