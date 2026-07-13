use super::*;

impl WorkspaceIndex {
    pub fn symbols_with_prefix(&self, prefix: &str, limit: usize) -> Vec<SymbolRecord> {
        let needle = prefix.to_ascii_lowercase();
        let mut results = if self.cached_symbol_count > 0 {
            load_symbols_with_prefix_from_db(&self.db_path, prefix, limit, &self.options.roots)
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        results.extend(
            self.symbols_by_name
                .iter()
                .filter(|(name, _)| needle.is_empty() || name.starts_with(&needle))
                .filter_map(|(_, symbols)| best_symbol(symbols.clone())),
        );
        dedupe_best_symbols(results, limit)
    }

    pub fn completion_items_at_source(
        &self,
        source: &str,
        position: QueryPosition,
        limit: usize,
    ) -> Vec<QueryCompletion> {
        self.completion_items_at_source_with_fallback(source, position, limit, None)
    }

    pub(super) fn completion_items_at_source_with_fallback(
        &self,
        source: &str,
        position: QueryPosition,
        limit: usize,
        fallback_prefix: Option<&str>,
    ) -> Vec<QueryCompletion> {
        if limit == 0 || !is_code_completion_position(source, position) {
            return Vec::new();
        }
        if let Some(context) = member_completion_context(source, position) {
            if let Some(owner_type) =
                infer_completion_owner_type(source, &context.owner, position.line)
            {
                let completions =
                    self.known_sage_method_completions(owner_type, &context.prefix, limit);
                if !completions.is_empty() {
                    return completions;
                }
            }
        }
        let prefix = current_prefix(source, position.line, position.character).unwrap_or_default();
        let prefix = if prefix.is_empty() {
            fallback_prefix.unwrap_or("")
        } else {
            prefix.as_str()
        };
        let mut results = local_completion_items(source, position, prefix, limit);
        let mut seen: BTreeSet<String> = results
            .iter()
            .map(|completion| completion.label.to_ascii_lowercase())
            .collect();
        for record in self.symbols_with_prefix(prefix, limit) {
            if results.len() >= limit {
                break;
            }
            if seen.insert(record.name.to_ascii_lowercase()) {
                results.push(completion_from_symbol(record));
            }
        }
        results
    }

    pub fn workspace_symbols(&self, query: &str, limit: usize) -> Vec<SymbolRecord> {
        let needle = query.to_ascii_lowercase();
        if limit == 0 {
            return Vec::new();
        }
        if is_valid_identifier(query) {
            let mut exact = self.symbol_candidates_without_docs(query);
            if exact.is_empty() {
                if let Some(resolution) = self.resolve_sage_exported_symbol(query) {
                    exact.push(resolution.record);
                }
            }
            if !exact.is_empty() {
                exact = suppress_workspace_import_noise(exact);
                exact.sort_by(|left, right| {
                    workspace_symbol_sort_key(left, &needle)
                        .cmp(&workspace_symbol_sort_key(right, &needle))
                        .then(left.name.cmp(&right.name))
                        .then(left.module.cmp(&right.module))
                });
                exact.truncate(limit);
                return exact;
            }
        }
        let fetch_limit = limit.saturating_mul(12).max(limit).max(200);
        let mut results = if self.cached_symbol_count > 0 {
            load_workspace_symbols_from_db(&self.db_path, query, fetch_limit, &self.options.roots)
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        results.extend(
            self.symbols_by_name
                .iter()
                .filter(|(name, symbols)| {
                    needle.is_empty()
                        || name.contains(&needle)
                        || symbols.first().is_some_and(|symbol| {
                            symbol.module.to_ascii_lowercase().contains(&needle)
                        })
                })
                .flat_map(|(_, symbols)| symbols.clone()),
        );
        let mut results = dedupe_symbol_records(results);
        results = suppress_workspace_import_noise(results);
        results.sort_by(|left, right| {
            workspace_symbol_sort_key(left, &needle)
                .cmp(&workspace_symbol_sort_key(right, &needle))
                .then(left.name.cmp(&right.name))
                .then(left.module.cmp(&right.module))
        });
        results.truncate(limit);
        results
    }

    pub fn symbol(&self, name: &str) -> Option<SymbolRecord> {
        best_symbol(self.symbol_candidates(name))
    }

    pub fn resolve_symbol(&self, name: &str, module_hint: Option<&str>) -> Option<SymbolRecord> {
        let candidates = self.symbol_candidates(name);
        if candidates.is_empty() {
            return None;
        }
        let resolved = resolve_from_candidates(module_hint, candidates)?;
        if resolved.kind == SymbolKind::Import {
            self.resolve_import_record(&resolved).or(Some(resolved))
        } else {
            Some(resolved)
        }
    }

    pub(super) fn resolve_sage_exported_symbol(&self, name: &str) -> Option<SageExportResolution> {
        self.resolve_sage_exported_symbol_from("sage.all", name)
    }

    pub(super) fn resolve_sage_exported_symbol_from(
        &self,
        import_module: &str,
        name: &str,
    ) -> Option<SageExportResolution> {
        if let Some(resolution) = self.resolve_hot_sage_export(import_module, name) {
            return Some(resolution);
        }
        if self.cached_symbol_count > 0 || self.db_path.exists() {
            if let Ok(Some(resolution)) = load_materialized_sage_export_from_db(
                &self.db_path,
                import_module,
                name,
                &self.options.roots,
            ) {
                return Some(resolution);
            }
        }
        if let Some(import_symbol) = self
            .symbol_candidates(name)
            .into_iter()
            .filter(|candidate| candidate.kind == SymbolKind::Import)
            .find(|candidate| candidate.module == import_module)
        {
            if let Some(record) = self.resolve_import_record(&import_symbol) {
                return Some(SageExportResolution {
                    record,
                    reason: "indexed sage.all re-export chain",
                });
            }
            return Some(SageExportResolution {
                record: import_symbol,
                reason: "indexed sage.all import binding",
            });
        }
        if module_is_sage_all_export_module(import_module) {
            if let Some(record) =
                self.resolve_module_symbol_from_roots(import_module, name, 0, &mut BTreeSet::new())
            {
                return Some(SageExportResolution {
                    record,
                    reason: "source-derived sage.all export chain",
                });
            }
        }
        if let Some(target) = SAGE_EXPORT_MAP
            .iter()
            .find(|target| target.import_module == import_module && target.name == name)
        {
            if let Some(record) = self
                .symbol_candidates(target.source_name)
                .into_iter()
                .filter(|candidate| {
                    import_target_definition_matches(
                        candidate,
                        target.source_module,
                        target.source_name,
                    )
                })
                .min_by_key(symbol_choice_key)
                .or_else(|| {
                    self.resolve_module_symbol_from_roots(
                        target.source_module,
                        target.source_name,
                        0,
                        &mut BTreeSet::new(),
                    )
                })
            {
                return Some(SageExportResolution {
                    record,
                    reason: "built-in sage.all export fallback",
                });
            }
        }
        None
    }

    fn resolve_hot_sage_export(
        &self,
        import_module: &str,
        name: &str,
    ) -> Option<SageExportResolution> {
        if import_module != "sage.all" {
            return None;
        }
        let target = SAGE_EXPORT_MAP
            .iter()
            .find(|target| target.import_module == "sage.all" && target.name == name)?;
        let key = name.to_ascii_lowercase();
        let symbols = self
            .symbol_lookup_cache
            .lock()
            .ok()
            .and_then(|cache| cache.get(&key).cloned())?;
        let record = best_symbol(
            symbols
                .into_iter()
                .filter(|symbol| {
                    symbol.name == name
                        && symbol.kind != SymbolKind::Import
                        && !module_is_sage_all_export_module(&symbol.module)
                        && path_is_under_roots(&symbol.path, &self.options.roots)
                        && import_target_definition_matches(
                            symbol,
                            target.source_module,
                            target.source_name,
                        )
                })
                .collect(),
        )?;
        Some(SageExportResolution {
            record,
            reason: "materialized sage.all export cache (hot)",
        })
    }

    pub(super) fn resolve_import_record(&self, symbol: &SymbolRecord) -> Option<SymbolRecord> {
        let mut seen = BTreeSet::new();
        self.resolve_import_record_with_depth(symbol, 0, &mut seen)
    }

    fn resolve_import_record_with_depth(
        &self,
        symbol: &SymbolRecord,
        depth: usize,
        seen: &mut BTreeSet<String>,
    ) -> Option<SymbolRecord> {
        if symbol.kind != SymbolKind::Import || depth >= MAX_IMPORT_RESOLUTION_DEPTH {
            return None;
        }
        let import_from = symbol.import_from.as_ref()?;
        let (source_module, source_name) =
            import_target_in_context(import_from, &symbol.name, &symbol.module);
        if !seen.insert(format!("{source_module}::{source_name}")) {
            return None;
        }

        let candidates = self.symbol_candidates(&source_name);
        if let Some(definition) = candidates
            .iter()
            .filter(|candidate| {
                import_target_definition_matches(candidate, &source_module, &source_name)
            })
            .min_by_key(|candidate| symbol_choice_key(candidate))
            .cloned()
        {
            return Some(definition);
        }

        if let Some(definition) =
            self.resolve_module_symbol_from_roots(&source_module, &source_name, depth + 1, seen)
        {
            return Some(definition);
        }

        let next_import = candidates
            .iter()
            .filter(|candidate| {
                candidate.kind == SymbolKind::Import
                    && candidate.name == source_name
                    && module_matches_import(&candidate.module, &source_module)
            })
            .min_by_key(|candidate| symbol_choice_key(candidate))?;
        self.resolve_import_record_with_depth(next_import, depth + 1, seen)
            .or_else(|| Some(next_import.clone()))
    }

    pub(super) fn resolve_module_symbol_from_roots(
        &self,
        module: &str,
        name: &str,
        depth: usize,
        seen: &mut BTreeSet<String>,
    ) -> Option<SymbolRecord> {
        self.resolve_module_symbol_from_roots_with_exports(module, name, depth, seen, false)
    }

    fn resolve_module_symbol_from_roots_with_exports(
        &self,
        module: &str,
        name: &str,
        depth: usize,
        seen: &mut BTreeSet<String>,
        require_exported: bool,
    ) -> Option<SymbolRecord> {
        if depth >= MAX_IMPORT_RESOLUTION_DEPTH {
            return None;
        }
        let path =
            module_source_path_from_roots(module, &self.options.roots, self.options.enable_pyx)?;
        let file = parse_file_for_roots(&path, &self.options.roots).ok()?;
        let symbols = file.symbols;
        if require_exported {
            if let Some(exported_names) = explicit_all_names_from_symbols(symbols.iter()) {
                if !exported_names.contains(name) {
                    return None;
                }
            } else if name.starts_with('_') {
                return None;
            }
        }
        let candidates: Vec<_> = symbols
            .iter()
            .filter(|symbol| symbol.name == name)
            .cloned()
            .collect();
        if let Some(definition) = candidates
            .iter()
            .filter(|candidate| import_target_definition_matches(candidate, module, name))
            .min_by_key(|candidate| symbol_choice_key(candidate))
            .cloned()
        {
            return Some(definition);
        }
        for star_import in symbols
            .iter()
            .filter(|symbol| is_star_import_symbol(symbol))
        {
            let import_from = star_import.import_from.as_deref()?;
            let star_module = import_from.strip_suffix("::*").unwrap_or(import_from);
            let star_module = resolve_relative_module(star_module, module);
            if !seen.insert(format!("{star_module}::{name}")) {
                continue;
            }
            if let Some(definition) = self.resolve_module_symbol_from_roots_with_exports(
                &star_module,
                name,
                depth + 1,
                seen,
                true,
            ) {
                return Some(definition);
            }
        }
        let next_import = candidates
            .iter()
            .filter(|candidate| candidate.kind == SymbolKind::Import)
            .min_by_key(|candidate| symbol_choice_key(candidate))?
            .clone();
        self.resolve_import_record_with_depth(&next_import, depth + 1, seen)
            .or(Some(next_import))
    }

    pub(super) fn symbol_candidates(&self, name: &str) -> Vec<SymbolRecord> {
        let key = name.to_ascii_lowercase();
        if let Ok(cache) = self.symbol_lookup_cache.lock() {
            if let Some(cached) = cache.get(&key) {
                let mut symbols = cached.clone();
                if let Some(memory_symbols) = self.symbols_by_name.get(&key) {
                    symbols.extend(memory_symbols.clone());
                }
                return dedupe_symbol_records(symbols);
            }
        }
        let mut symbols = if self.cached_symbol_count > 0 {
            load_symbols_by_name_from_db(&self.db_path, name, &self.options.roots)
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let persistent_symbols = symbols.clone();
        if let Ok(mut cache) = self.symbol_lookup_cache.lock() {
            cache.insert(key.clone(), persistent_symbols);
        }
        if let Some(memory_symbols) = self.symbols_by_name.get(&key) {
            symbols.extend(memory_symbols.clone());
        }
        dedupe_symbol_records(symbols)
    }

    fn symbol_candidates_without_docs(&self, name: &str) -> Vec<SymbolRecord> {
        let key = name.to_ascii_lowercase();
        if let Ok(cache) = self.symbol_lookup_cache.lock() {
            if let Some(cached) = cache.get(&key) {
                let mut symbols = cached.clone();
                if let Some(memory_symbols) = self.symbols_by_name.get(&key) {
                    symbols.extend(memory_symbols.clone());
                }
                return dedupe_symbol_records(symbols);
            }
        }
        let mut symbols = if self.cached_symbol_count > 0 {
            load_symbols_by_name_from_db_without_docs(&self.db_path, name, &self.options.roots)
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let persistent_symbols = symbols.clone();
        if let Ok(mut cache) = self.symbol_lookup_cache.lock() {
            cache.insert(key.clone(), persistent_symbols);
        }
        if let Some(memory_symbols) = self.symbols_by_name.get(&key) {
            symbols.extend(memory_symbols.clone());
        }
        dedupe_symbol_records(symbols)
    }

    pub(super) fn resolve_member_symbol(
        &self,
        source: &str,
        owner: &str,
        member: &str,
        module_hint: Option<&str>,
        target_line: u32,
    ) -> MemberResolution {
        let owner_type = infer_owner_type_before(source, owner, member, target_line)
            .or_else(|| infer_owner_type_from_member_hint(member));
        if let Some(owner_type) = owner_type {
            if let Some(record) = self.resolve_known_sage_method_record(owner_type, member) {
                return MemberResolution {
                    record: Some(record),
                    owner_type: Some(owner_type),
                    confidence: "high",
                    reason: format!(
                        "resolved Sage {} method `{}` from owner `{}`",
                        owner_type.as_str(),
                        member,
                        owner
                    ),
                    candidate_count: 1,
                    suppress_global_fallback: true,
                };
            }
        }
        if let Some(owner_resolution) =
            self.resolve_source_derived_namespace_owner(source, owner, module_hint)
        {
            let record = self.resolve_member_in_namespace_owner(&owner_resolution.record, member);
            let found = record.is_some();
            return MemberResolution {
                record,
                owner_type,
                confidence: if found { "high" } else { "ambiguous" },
                reason: format!(
                    "resolved Sage namespace member `{owner}.{member}` through {}",
                    owner_resolution.reason
                ),
                candidate_count: self.symbol_candidates(member).len(),
                suppress_global_fallback: true,
            };
        }
        if is_sage_namespace_owner(owner) {
            let record = self
                .resolve_symbol(member, module_hint)
                .or_else(|| self.resolve_symbol(member, None));
            let confidence = if record.is_some() {
                "high"
            } else {
                "ambiguous"
            };
            return MemberResolution {
                record,
                owner_type,
                confidence,
                reason: format!("resolved Sage namespace member `{owner}.{member}`"),
                candidate_count: self.symbol_candidates(member).len(),
                suppress_global_fallback: true,
            };
        }
        let candidates: Vec<_> = self
            .symbol_candidates(member)
            .into_iter()
            .filter(|candidate| candidate.kind != SymbolKind::Import)
            .collect();
        let candidate_count = candidates.len();
        let Some(constructor) = assignment_constructor_before_line(source, owner, target_line)
        else {
            return MemberResolution {
                record: None,
                owner_type,
                confidence: if owner_type.is_some() {
                    "ambiguous"
                } else {
                    "none"
                },
                reason: if let Some(owner_type) = owner_type {
                    format!(
                        "no static target for Sage {} method `{}`",
                        owner_type.as_str(),
                        member
                    )
                } else if is_known_sage_method(member) {
                    format!("ambiguous Sage method `{member}` without a known owner type")
                } else {
                    format!("no owner type for dotted member `{owner}.{member}`")
                },
                candidate_count,
                suppress_global_fallback: true,
            };
        };
        let constructor_name = constructor.rsplit('.').next().unwrap_or(&constructor);
        let Some(owner_symbol) = self
            .resolve_symbol(constructor_name, module_hint)
            .or_else(|| self.resolve_symbol(constructor_name, None))
        else {
            return MemberResolution {
                record: None,
                owner_type,
                confidence: "ambiguous",
                reason: format!("constructor `{constructor}` for `{owner}` was not indexed"),
                candidate_count,
                suppress_global_fallback: true,
            };
        };
        if candidates.is_empty() {
            return MemberResolution {
                record: None,
                owner_type,
                confidence: "ambiguous",
                reason: format!("no indexed candidates for member `{member}`"),
                candidate_count,
                suppress_global_fallback: true,
            };
        }
        let constructor_candidates: Vec<_> = candidates
            .iter()
            .filter(|candidate| {
                method_candidate_matches_constructor(
                    candidate,
                    member,
                    constructor_name,
                    &owner_symbol.name,
                )
            })
            .cloned()
            .collect();
        let candidates = if constructor_candidates.is_empty() {
            candidates
        } else {
            constructor_candidates
        };
        let same_path: Vec<_> = candidates
            .iter()
            .filter(|candidate| {
                !owner_symbol.path.as_os_str().is_empty() && candidate.path == owner_symbol.path
            })
            .cloned()
            .collect();
        if !same_path.is_empty() {
            return MemberResolution {
                record: best_symbol(same_path),
                owner_type,
                confidence: "high",
                reason: format!("member `{member}` matched constructor source path"),
                candidate_count,
                suppress_global_fallback: true,
            };
        }
        let same_module: Vec<_> = candidates
            .iter()
            .filter(|candidate| candidate.module == owner_symbol.module)
            .cloned()
            .collect();
        if !same_module.is_empty() {
            return MemberResolution {
                record: best_symbol(same_module),
                owner_type,
                confidence: "high",
                reason: format!("member `{member}` matched constructor module"),
                candidate_count,
                suppress_global_fallback: true,
            };
        }
        MemberResolution {
            record: None,
            owner_type,
            confidence: "ambiguous",
            reason: format!("member `{member}` did not match constructor module or source path"),
            candidate_count,
            suppress_global_fallback: true,
        }
    }

    pub(super) fn resolve_loaded_symbol_before_line(
        &self,
        query_path: &Path,
        source: &str,
        name: &str,
        max_line: u32,
    ) -> Option<SymbolRecord> {
        let loaded_paths = sage_load_attach_paths_before_line(query_path, source, max_line);
        for loaded_path in loaded_paths.into_iter().rev() {
            let record = self
                .file_for_path(&loaded_path)
                .or_else(|| self.parse_indexable_file_on_demand(&loaded_path))
                .and_then(|file| {
                    best_symbol(
                        file.symbols
                            .into_iter()
                            .filter(|symbol| {
                                symbol.name == name
                                    && !matches!(
                                        symbol.kind,
                                        SymbolKind::Import | SymbolKind::Module
                                    )
                            })
                            .collect(),
                    )
                });
            if record.is_some() {
                return record;
            }
        }
        None
    }

    fn parse_indexable_file_on_demand(&self, path: &Path) -> Option<IndexedFile> {
        if !is_indexable(path, self.options.enable_pyx)
            || !path_is_under_roots(path, &self.options.roots)
        {
            return None;
        }
        let source = fs::read_to_string(path).ok()?;
        let root = self
            .options
            .roots
            .iter()
            .find(|root| path.strip_prefix(root).is_ok())?;
        let module = module_name_from_path(root, path);
        Some(parse_source(&module, path, &source))
    }

    fn resolve_known_sage_method_record(
        &self,
        owner_type: SageOwnerType,
        member: &str,
    ) -> Option<SymbolRecord> {
        let cache_key = sage_method_cache_key(owner_type, member);
        if let Ok(cache) = self.sage_method_lookup_cache.lock() {
            if let Some(cached) = cache.get(&cache_key) {
                return cached.clone();
            }
        }
        if self.cached_symbol_count > 0 || self.db_path.exists() {
            if let Ok(Some(record)) = load_materialized_sage_method_from_db(
                &self.db_path,
                owner_type,
                member,
                &self.options.roots,
            ) {
                self.insert_sage_method_lookup_cache(owner_type, member, Some(record.clone()));
                return Some(record);
            }
        }
        let resolved = if let Some(spec) = SAGE_METHOD_SPECS
            .iter()
            .find(|spec| spec.owner_type == owner_type && spec.member == member)
        {
            self.resolve_symbol_in_module(member, spec.module)
        } else {
            SAGE_METHOD_ALIAS_SPECS
                .iter()
                .find(|spec| spec.owner_type == owner_type && spec.member == member)
                .and_then(|alias| self.resolve_symbol_in_module(alias.source_name, alias.module))
        };
        self.insert_sage_method_lookup_cache(owner_type, member, resolved.clone());
        resolved
    }

    fn known_sage_method_completions(
        &self,
        owner_type: SageOwnerType,
        prefix: &str,
        limit: usize,
    ) -> Vec<QueryCompletion> {
        let needle = prefix.to_ascii_lowercase();
        let mut completions = BTreeMap::<String, QueryCompletion>::new();
        if self.cached_symbol_count > 0 || self.db_path.exists() {
            if let Ok(entries) = load_materialized_sage_method_completions_from_db(
                &self.db_path,
                owner_type,
                prefix,
                &self.options.roots,
                limit,
            ) {
                for (member, record) in entries {
                    if member.starts_with('_') && !prefix.starts_with('_') {
                        continue;
                    }
                    self.insert_sage_method_lookup_cache(owner_type, &member, Some(record.clone()));
                    completions.entry(member.clone()).or_insert_with(|| {
                        method_completion_from_record(owner_type, &member, Some(&record))
                    });
                }
            }
        }
        for spec in SAGE_METHOD_SPECS
            .iter()
            .filter(|spec| spec.owner_type == owner_type)
            .filter(|spec| spec.member.starts_with(&needle))
        {
            let record = self.resolve_known_sage_method_record(owner_type, spec.member);
            completions
                .entry(spec.member.to_string())
                .or_insert_with(|| {
                    method_completion_from_record(owner_type, spec.member, record.as_ref())
                });
        }
        for spec in SAGE_METHOD_ALIAS_SPECS
            .iter()
            .filter(|spec| spec.owner_type == owner_type)
            .filter(|spec| spec.member.starts_with(&needle))
        {
            let record = self.resolve_known_sage_method_record(owner_type, spec.member);
            completions
                .entry(spec.member.to_string())
                .or_insert_with(|| {
                    method_completion_from_record(owner_type, spec.member, record.as_ref())
                });
        }
        completions.into_values().take(limit).collect()
    }

    fn resolve_symbol_in_module(&self, name: &str, module: &str) -> Option<SymbolRecord> {
        let mut symbols = if self.cached_symbol_count > 0 {
            load_symbols_by_name_and_module_from_db(
                &self.db_path,
                name,
                module,
                &self.options.roots,
            )
            .unwrap_or_default()
        } else {
            Vec::new()
        };
        if let Some(memory_symbols) = self.symbols_by_name.get(&name.to_ascii_lowercase()) {
            symbols.extend(
                memory_symbols
                    .iter()
                    .filter(|symbol| {
                        symbol.kind != SymbolKind::Import
                            && symbol.name == name
                            && module_matches_import(&symbol.module, module)
                    })
                    .cloned(),
            );
        }
        best_symbol(dedupe_symbol_records(symbols))
    }

    pub(super) fn resolve_symbol_in_module_without_docs(
        &self,
        name: &str,
        module: &str,
    ) -> Option<SymbolRecord> {
        if let Ok(cache) = self.symbol_lookup_cache.lock() {
            if let Some(symbols) = cache.get(&name.to_ascii_lowercase()) {
                if let Some(symbol) = best_symbol(
                    symbols
                        .iter()
                        .filter(|symbol| {
                            symbol.kind != SymbolKind::Import
                                && symbol.name == name
                                && module_matches_import(&symbol.module, module)
                                && path_is_under_roots(&symbol.path, &self.options.roots)
                        })
                        .cloned()
                        .collect(),
                ) {
                    return Some(symbol);
                }
            }
        }
        let mut symbols = if self.cached_symbol_count > 0 {
            load_symbols_by_name_and_module_from_db_without_docs(
                &self.db_path,
                name,
                module,
                &self.options.roots,
            )
            .unwrap_or_default()
        } else {
            Vec::new()
        };
        if let Some(memory_symbols) = self.symbols_by_name.get(&name.to_ascii_lowercase()) {
            symbols.extend(
                memory_symbols
                    .iter()
                    .filter(|symbol| {
                        symbol.kind != SymbolKind::Import
                            && symbol.name == name
                            && module_matches_import(&symbol.module, module)
                    })
                    .cloned(),
            );
        }
        best_symbol(dedupe_symbol_records(symbols))
    }

    fn resolve_source_derived_namespace_owner(
        &self,
        source: &str,
        owner: &str,
        module_hint: Option<&str>,
    ) -> Option<SageExportResolution> {
        if let Some(lookup) = source_imported_sage_all_lookup(source, owner) {
            if let Some(resolution) =
                self.resolve_sage_exported_symbol_from(&lookup.import_module, &lookup.source_name)
            {
                if is_namespace_owner_record(&resolution.record) {
                    return Some(resolution);
                }
            }
        }
        if let Some(resolution) = self.resolve_sage_exported_symbol(owner) {
            if is_namespace_owner_record(&resolution.record) {
                return Some(resolution);
            }
        }
        let record = self
            .resolve_symbol(owner, module_hint)
            .or_else(|| self.resolve_symbol(owner, None))?;
        (record.kind == SymbolKind::Module && record.module.starts_with("sage.")).then_some(
            SageExportResolution {
                record,
                reason: "indexed namespace owner",
            },
        )
    }

    fn resolve_member_in_namespace_owner(
        &self,
        owner_record: &SymbolRecord,
        member: &str,
    ) -> Option<SymbolRecord> {
        let candidates = self
            .symbol_candidates(member)
            .into_iter()
            .filter(|candidate| namespace_member_matches_owner(candidate, owner_record, member))
            .collect::<Vec<_>>();
        let mut resolved = Vec::new();
        for candidate in candidates {
            if candidate.kind == SymbolKind::Import {
                resolved.push(
                    self.resolve_import_record(&candidate)
                        .unwrap_or_else(|| candidate.clone()),
                );
            } else {
                resolved.push(candidate);
            }
        }
        best_symbol(dedupe_symbol_records(resolved))
    }
}

fn method_candidate_matches_constructor(
    candidate: &SymbolRecord,
    member: &str,
    constructor_name: &str,
    owner_symbol_name: &str,
) -> bool {
    let owner_matches =
        |class_name: &str| class_name == constructor_name || class_name == owner_symbol_name;
    if let Some((class_name, method_name)) = method_detail_parts(&candidate.detail) {
        return method_name == member && owner_matches(class_name);
    }
    if let Some((class_name, alias, _target)) = class_method_alias_detail_parts(&candidate.detail) {
        return alias == member && owner_matches(class_name);
    }
    false
}
