use super::*;
use crate::symbol_resolution::MemberResolutionContext;

#[derive(Clone, Copy, Default)]
struct PreparedQueryContext<'a> {
    local_symbols: Option<&'a [SymbolRecord]>,
    source_map: Option<&'a CodeMap>,
}

impl WorkspaceIndex {
    pub fn documentation_for_symbol(&self, name: &str) -> Option<DocumentationRecord> {
        if let Some(export) = self.resolve_sage_exported_symbol(name) {
            let documentation = self.documentation_for_resolved_symbol(&export.record);
            if documentation_has_specific_docstring(&documentation) {
                return Some(documentation);
            }
        }
        let static_documentation = self
            .resolve_symbol(name, None)
            .or_else(|| builtin_symbol_record(name))
            .map(|symbol| self.documentation_for_resolved_symbol(&symbol));
        if static_documentation
            .as_ref()
            .is_some_and(documentation_has_specific_docstring)
        {
            return static_documentation;
        }
        if let Ok(Some(runtime_documentation)) =
            load_runtime_documentation_from_db(&self.db_path, name)
        {
            return Some(runtime_documentation);
        }
        static_documentation
    }

    pub fn documentation_for_symbol_with_module(
        &self,
        name: &str,
        module_hint: Option<&str>,
    ) -> Option<DocumentationRecord> {
        let static_documentation = self
            .resolve_symbol(name, module_hint)
            .or_else(|| builtin_symbol_record(name))
            .map(|symbol| self.documentation_for_resolved_symbol(&symbol));
        if static_documentation
            .as_ref()
            .is_some_and(documentation_has_specific_docstring)
        {
            return static_documentation;
        }
        self.documentation_for_symbol(name).or(static_documentation)
    }

    pub fn write_runtime_documentation(
        &self,
        symbol: &str,
        record: &DocumentationRecord,
    ) -> Result<()> {
        if symbol.trim().is_empty() {
            return Ok(());
        }
        if record
            .docstring
            .as_deref()
            .is_none_or(|docstring| docstring.trim().is_empty())
            && record.summary.trim().is_empty()
        {
            return Ok(());
        }
        let connection = Connection::open(&self.db_path)
            .with_context(|| format!("open index db {}", self.db_path.display()))?;
        create_schema(&connection)?;
        upsert_runtime_documentation(&connection, symbol, record)
    }

    fn documentation_for_resolved_symbol(&self, symbol: &SymbolRecord) -> DocumentationRecord {
        let mut documentation = documentation_for_symbol(symbol);
        if symbol.kind == SymbolKind::Variable
            && symbol.docstring.as_ref().is_none_or(String::is_empty)
        {
            for related_name in [
                format!("{}Factory", symbol.name),
                format!("{}_class", symbol.name),
            ] {
                let related = self
                    .resolve_symbol(&related_name, Some(&symbol.module))
                    .or_else(|| self.resolve_symbol(&related_name, None));
                if let Some(related) = related {
                    if let Some(docstring) = related
                        .docstring
                        .as_ref()
                        .filter(|docstring| !docstring.is_empty())
                    {
                        documentation.summary =
                            documentation_summary(docstring).unwrap_or_else(|| docstring.clone());
                        documentation.docstring = Some(docstring.clone());
                        documentation
                            .markers
                            .push(format!("related-doc:{}:{}", related.module, related.name));
                        break;
                    }
                }
            }
        }
        documentation
    }

    pub fn query_source_at(
        &self,
        path: &Path,
        source: &str,
        position: QueryPosition,
        rename_to: Option<&str>,
    ) -> QueryResult {
        self.query_source_at_with_features(path, source, position, rename_to, QueryFeatures::full())
    }

    pub fn query_source_at_navigation(
        &self,
        path: &Path,
        source: &str,
        position: QueryPosition,
    ) -> QueryResult {
        self.query_source_at_with_features(
            path,
            source,
            position,
            None,
            QueryFeatures::navigation(),
        )
    }

    pub fn query_source_at_navigation_with_symbols(
        &self,
        path: &Path,
        source: &str,
        position: QueryPosition,
        local_symbols: &[SymbolRecord],
    ) -> QueryResult {
        self.query_source_at_with_features_and_symbols(
            path,
            source,
            position,
            None,
            QueryFeatures::navigation(),
            Some(local_symbols),
        )
    }

    pub fn query_source_definition_with_symbols(
        &self,
        path: &Path,
        source: &str,
        position: QueryPosition,
        local_symbols: &[SymbolRecord],
    ) -> QueryResult {
        self.query_source_at_with_features_and_symbols(
            path,
            source,
            position,
            None,
            QueryFeatures::definition_only(),
            Some(local_symbols),
        )
    }

    pub fn parse_source_for_query(&self, path: &Path, source: &str) -> IndexedFile {
        let query_path = normalize_path(path.to_path_buf());
        let module = self
            .module_hint_for_query_path(&query_path)
            .unwrap_or_else(|| "document".to_string());
        parse_source(&module, &query_path, source)
    }

    pub fn type_definition_at_source(
        &self,
        path: &Path,
        source: &str,
        position: QueryPosition,
    ) -> Option<QueryDefinition> {
        let query_path = normalize_path(path.to_path_buf());
        let (word, range) = word_at_source_position(source, position.line, position.character)?;
        let source_map = CodeMap::new(source);
        let target_is_code = source_map
            .offset(range.start_line, range.start_character)
            .is_some_and(|offset| source_map.is_code_offset(offset));
        if !target_is_code {
            return None;
        }
        let owner_type = infer_owner_type_before_strict(source, &word, "", position.line)?;
        let parsed = self.parse_source_for_query(&query_path, source);
        if !self.owner_type_has_reliable_sage_binding(
            source,
            &word,
            &query_path,
            &range,
            &parsed.symbols,
            owner_type,
        ) {
            return None;
        }
        let type_symbol = type_symbol_for_owner_type(owner_type)?;
        self.type_definition_for_symbol(type_symbol)
    }

    fn type_definition_for_symbol(&self, type_symbol: &str) -> Option<QueryDefinition> {
        let target = SAGE_EXPORT_MAP
            .iter()
            .find(|target| target.import_module == "sage.all" && target.name == type_symbol);
        let record = target
            .and_then(|target| {
                self.resolve_symbol_in_module_without_docs(target.source_name, target.source_module)
                    .or_else(|| {
                        self.resolve_module_symbol_from_roots(
                            target.source_module,
                            target.source_name,
                            0,
                            &mut BTreeSet::new(),
                        )
                    })
            })
            .or_else(|| self.resolve_symbol(type_symbol, None))?;
        query_definition_from_record(&record)
    }

    pub fn query_source_at_with_features(
        &self,
        path: &Path,
        source: &str,
        position: QueryPosition,
        rename_to: Option<&str>,
        features: QueryFeatures,
    ) -> QueryResult {
        self.query_source_at_with_features_and_symbols(
            path, source, position, rename_to, features, None,
        )
    }

    fn query_source_at_with_features_and_symbols(
        &self,
        path: &Path,
        source: &str,
        position: QueryPosition,
        rename_to: Option<&str>,
        features: QueryFeatures,
        local_symbols: Option<&[SymbolRecord]>,
    ) -> QueryResult {
        let diagnostics = if features.diagnostics {
            self.diagnostics_for_source(path, source)
        } else {
            Vec::new()
        };
        let Some((word, range)) =
            word_at_source_position(source, position.line, position.character)
        else {
            return QueryResult {
                diagnostics,
                fallback_reason: Some("no-symbol-at-position".to_string()),
                ..QueryResult::default()
            };
        };
        self.query_source_symbol_with_options_and_symbols(
            path,
            source,
            &word,
            Some(range),
            QueryExecutionOptions {
                rename_to,
                diagnostics,
                features,
            },
            PreparedQueryContext {
                local_symbols,
                source_map: None,
            },
        )
    }

    pub fn query_source_symbol(
        &self,
        path: &Path,
        source: &str,
        symbol: &str,
        known_range: Option<SourceRange>,
        rename_to: Option<&str>,
        diagnostics: Vec<DiagnosticRecord>,
    ) -> QueryResult {
        self.query_source_symbol_with_options(
            path,
            source,
            symbol,
            known_range,
            QueryExecutionOptions {
                rename_to,
                diagnostics,
                features: QueryFeatures::full(),
            },
        )
    }

    pub fn query_source_symbol_with_options(
        &self,
        path: &Path,
        source: &str,
        symbol: &str,
        known_range: Option<SourceRange>,
        options: QueryExecutionOptions<'_>,
    ) -> QueryResult {
        self.query_source_symbol_with_options_and_symbols(
            path,
            source,
            symbol,
            known_range,
            options,
            PreparedQueryContext::default(),
        )
    }

    pub fn query_source_definitions_for_ranges_with_symbols(
        &self,
        path: &Path,
        source: &str,
        symbol: &str,
        ranges: &[SourceRange],
        local_symbols: &[SymbolRecord],
    ) -> Vec<QueryResult> {
        let source_map = CodeMap::new(source);
        ranges
            .iter()
            .map(|range| {
                self.query_source_symbol_with_options_and_symbols(
                    path,
                    source,
                    symbol,
                    Some(range.clone()),
                    QueryExecutionOptions {
                        rename_to: None,
                        diagnostics: Vec::new(),
                        features: QueryFeatures::definition_only(),
                    },
                    PreparedQueryContext {
                        local_symbols: Some(local_symbols),
                        source_map: Some(&source_map),
                    },
                )
            })
            .collect()
    }

    pub fn query_source_definitions_for_named_ranges_with_symbols(
        &self,
        path: &Path,
        source: &str,
        queries: &[(String, SourceRange)],
        local_symbols: &[SymbolRecord],
    ) -> Vec<QueryResult> {
        let source_map = CodeMap::new(source);
        queries
            .iter()
            .map(|(symbol, range)| {
                self.query_source_symbol_with_options_and_symbols(
                    path,
                    source,
                    symbol,
                    Some(range.clone()),
                    QueryExecutionOptions {
                        rename_to: None,
                        diagnostics: Vec::new(),
                        features: QueryFeatures::definition_only(),
                    },
                    PreparedQueryContext {
                        local_symbols: Some(local_symbols),
                        source_map: Some(&source_map),
                    },
                )
            })
            .collect()
    }

    fn query_source_symbol_with_options_and_symbols(
        &self,
        path: &Path,
        source: &str,
        symbol: &str,
        known_range: Option<SourceRange>,
        options: QueryExecutionOptions<'_>,
        context: PreparedQueryContext<'_>,
    ) -> QueryResult {
        let QueryExecutionOptions {
            rename_to,
            diagnostics,
            features,
        } = options;
        let query_path = normalize_path(path.to_path_buf());
        let owned_source_map;
        let source_map = if let Some(source_map) = context.source_map {
            source_map
        } else {
            owned_source_map = CodeMap::new(source);
            &owned_source_map
        };
        let target_range = known_range
            .or_else(|| range_for_first_code_symbol(source, symbol, source_map))
            .or_else(|| range_for_first_symbol(source, symbol))
            .unwrap_or_default();
        let target_is_code = source_map
            .offset(target_range.start_line, target_range.start_character)
            .is_some_and(|offset| source_map.is_code_offset(offset));
        let dotted_symbol = dotted_symbol_at_range(source, &target_range);
        let lookup_name = dotted_symbol
            .as_deref()
            .and_then(|value| value.rsplit('.').next())
            .filter(|value| !value.is_empty())
            .unwrap_or(symbol);
        let module_hint = context
            .local_symbols
            .and_then(|symbols| symbols.first())
            .map(|symbol| symbol.module.clone())
            .or_else(|| self.module_hint_for_query_path(&query_path));
        let local_module = module_hint.as_deref().unwrap_or("document");
        let parsed_local_symbols;
        let local_symbols = if let Some(symbols) = context.local_symbols {
            symbols
        } else {
            parsed_local_symbols = parse_source(local_module, &query_path, source).symbols;
            &parsed_local_symbols
        };
        let dotted_owner_member = dotted_symbol.as_deref().and_then(dotted_owner_member);
        let local_import =
            local_import_symbol_from_symbols(source, local_symbols, lookup_name, &target_range);
        let source_import_lookup = local_import.as_ref().and_then(|record| {
            let (import_module, source_name) = record.import_from.as_deref()?.rsplit_once("::")?;
            Some(SourceImportLookup {
                import_module: import_module.to_string(),
                source_name: source_name.to_string(),
            })
        });
        let sage_all_export_lookup = source_import_lookup
            .as_ref()
            .filter(|lookup| module_is_sage_all_export_module(&lookup.import_module))
            .map(|lookup| SourceImportLookup {
                import_module: lookup.import_module.clone(),
                source_name: lookup.source_name.clone(),
            })
            .or_else(|| source_imported_sage_all_star_lookup(source, lookup_name));
        let implicit_sage_all_lookup =
            is_sage_source_path(&query_path) && dotted_symbol.is_none() && target_is_code;
        let member_resolution =
            dotted_owner_member
                .filter(|_| target_is_code)
                .map(|(owner, member)| {
                    if let Some(record) = local_receiver_member_symbol_from_symbols(
                        source,
                        local_symbols,
                        owner,
                        member,
                        &target_range,
                    ) {
                        return MemberResolution {
                            record: Some(record),
                            candidates: Vec::new(),
                            owner_type: None,
                            confidence: "high",
                            reason: format!(
                                "resolved local receiver member `{owner}.{member}` in the enclosing class"
                            ),
                            candidate_count: 1,
                            suppress_global_fallback: true,
                        };
                    }
                    self.resolve_member_symbol(
                        source,
                        owner,
                        member,
                        MemberResolutionContext {
                            module_hint: module_hint.as_deref(),
                            query_path: &query_path,
                            target_range: &target_range,
                            local_symbols,
                        },
                    )
                });
        let mut suppress_global_fallback = !target_is_code
            || member_resolution
                .as_ref()
                .is_some_and(|resolution| resolution.suppress_global_fallback);
        let mut resolution_confidence = member_resolution
            .as_ref()
            .map(|resolution| resolution.confidence.to_string());
        let mut resolution_reason = member_resolution
            .as_ref()
            .map(|resolution| resolution.reason.clone());
        if !target_is_code {
            resolution_reason = Some(
                "the selected symbol occurrence is inside a comment or string literal".to_string(),
            );
        }
        let owner_type = member_resolution
            .as_ref()
            .and_then(|resolution| resolution.owner_type)
            .map(|owner_type| owner_type.as_str().to_string());
        let mut candidate_count = member_resolution
            .as_ref()
            .map(|resolution| resolution.candidate_count)
            .unwrap_or(0);
        let mut resolved = member_resolution
            .as_ref()
            .and_then(|resolution| resolution.record.clone());
        let mut navigation_candidates = member_resolution
            .as_ref()
            .filter(|resolution| resolution.record.is_none())
            .map(|resolution| resolution.candidates.clone())
            .unwrap_or_default();
        if resolved.is_none()
            && !suppress_global_fallback
            && dotted_symbol.is_none()
            && target_is_code
            && (implicit_sage_all_lookup
                || sage_all_export_lookup.is_some()
                || source_import_lookup.is_some())
        {
            if let Some(local_symbol) = local_shadow_symbol_from_symbols(
                local_module,
                &query_path,
                source,
                local_symbols,
                lookup_name,
                &target_range,
            ) {
                resolution_confidence = Some("high".to_string());
                resolution_reason = Some(format!(
                    "current document local symbol `{lookup_name}` shadows Sage import/export"
                ));
                resolved = Some(local_symbol);
            }
        }
        if resolved.is_none() && !suppress_global_fallback && implicit_sage_all_lookup {
            if let Some(export) = self.resolve_sage_exported_symbol(lookup_name) {
                resolution_confidence = Some("high".to_string());
                resolution_reason = Some(format!(
                    "resolved `{lookup_name}` through implicit .sage {}",
                    export.reason
                ));
                candidate_count = 1;
                resolved = Some(export.record);
            }
        }
        if resolved.is_none() && !suppress_global_fallback {
            if let Some(export_lookup) = sage_all_export_lookup.as_ref() {
                if let Some(export) = self.resolve_sage_exported_symbol_from(
                    &export_lookup.import_module,
                    &export_lookup.source_name,
                ) {
                    resolution_confidence = Some("high".to_string());
                    resolution_reason = Some(format!(
                        "resolved `{lookup_name}` through {}",
                        export.reason
                    ));
                    resolved = Some(export.record);
                } else {
                    suppress_global_fallback = true;
                    resolution_confidence = Some("ambiguous".to_string());
                    resolution_reason = Some(format!(
                        "`{lookup_name}` is imported from {} but is not present in the materialized Sage export cache",
                        export_lookup.import_module
                    ));
                }
            }
        }
        if resolved.is_none() && !suppress_global_fallback {
            if let Some(import_lookup) = source_import_lookup
                .as_ref()
                .filter(|lookup| !module_is_sage_all_export_module(&lookup.import_module))
            {
                let import_module = module_hint
                    .as_deref()
                    .map(|module| resolve_relative_module(&import_lookup.import_module, module))
                    .unwrap_or_else(|| import_lookup.import_module.clone());
                let mut seen = BTreeSet::new();
                let mut import_candidates = dedupe_symbol_records(
                    self.symbol_candidates(&import_lookup.source_name)
                        .into_iter()
                        .filter(|candidate| {
                            import_target_definition_matches(
                                candidate,
                                &import_module,
                                &import_lookup.source_name,
                            )
                        })
                        .collect(),
                );
                if import_candidates.is_empty() {
                    if let Some(record) = self.resolve_module_symbol_from_roots(
                        &import_module,
                        &import_lookup.source_name,
                        0,
                        &mut seen,
                    ) {
                        import_candidates.push(record);
                    }
                }
                import_candidates.sort_by_key(symbol_choice_key);
                match import_candidates.len() {
                    1 => {
                        resolution_confidence = Some("high".to_string());
                        resolution_reason = Some(format!(
                            "resolved `{lookup_name}` from explicit import target {}",
                            import_module
                        ));
                        candidate_count = 1;
                        resolved = import_candidates.pop();
                    }
                    0 => {
                        suppress_global_fallback = true;
                        resolution_confidence = Some("ambiguous".to_string());
                        resolution_reason = Some(format!(
                            "`{lookup_name}` is explicitly imported from {} but the target module is not indexed or resolvable",
                            import_module
                        ));
                    }
                    count => {
                        suppress_global_fallback = true;
                        resolution_confidence = Some("ambiguous".to_string());
                        resolution_reason = Some(format!(
                            "`{lookup_name}` has {count} indexed definitions in explicit import target {}",
                            import_module
                        ));
                        candidate_count = count;
                        navigation_candidates = import_candidates;
                    }
                }
            }
        }
        if resolved.is_none()
            && !suppress_global_fallback
            && dotted_symbol.is_none()
            && target_is_code
        {
            if let Some(record) = self.resolve_loaded_symbol_before_line(
                &query_path,
                source,
                lookup_name,
                target_range.start_line,
            ) {
                resolution_confidence = Some("high".to_string());
                resolution_reason = Some(format!(
                    "resolved `{lookup_name}` from a Sage load/attach target"
                ));
                candidate_count = 1;
                resolved = Some(record);
            }
        }
        if resolved.is_none()
            && !suppress_global_fallback
            && dotted_symbol.is_none()
            && target_is_code
        {
            if let Some(local_symbol) = local_shadow_symbol_from_symbols(
                local_module,
                &query_path,
                source,
                local_symbols,
                lookup_name,
                &target_range,
            ) {
                resolution_confidence = Some("high".to_string());
                resolution_reason = Some(format!(
                    "resolved `{lookup_name}` from the current document's lexical scope"
                ));
                candidate_count = 1;
                resolved = Some(local_symbol);
            }
        }
        if resolved.is_none() && !suppress_global_fallback {
            let mut global_candidates = self.symbol_candidates(lookup_name);
            if lookup_name != symbol {
                global_candidates.extend(self.symbol_candidates(symbol));
            }
            let global_candidates = dedupe_symbol_records(
                global_candidates
                    .into_iter()
                    .map(|candidate| {
                        if candidate.kind == SymbolKind::Import {
                            self.resolve_import_record(&candidate).unwrap_or(candidate)
                        } else {
                            candidate
                        }
                    })
                    .collect(),
            );
            match global_candidates.len() {
                0 => {
                    resolved = builtin_symbol_record(dotted_symbol.as_deref().unwrap_or(symbol))
                        .or_else(|| builtin_symbol_record(lookup_name));
                }
                1 => {
                    resolved = global_candidates.into_iter().next();
                }
                count => {
                    candidate_count = count;
                    resolution_confidence = Some("ambiguous".to_string());
                    resolution_reason = Some(format!(
                        "`{lookup_name}` has {count} indexed definitions and no reliable binding; explicit selection is required"
                    ));
                    navigation_candidates = global_candidates;
                    navigation_candidates.sort_by_key(symbol_choice_key);
                }
            }
        }
        if resolved.is_some() && resolution_confidence.is_none() {
            resolution_confidence = Some("medium".to_string());
            resolution_reason = Some("resolved by indexed symbol/import lookup".to_string());
        }
        if let Some(record) = &resolved {
            if record.kind == SymbolKind::Import {
                if let Some(source_record) = self.resolve_import_record(record) {
                    resolved = Some(source_record);
                }
            }
        }
        let precise_lookup = resolution_reason.as_deref().is_some_and(|reason| {
            reason.contains("sage.all")
                || reason.contains("explicit import target")
                || reason.contains("shadows Sage import/export")
                || reason.contains("load/attach target")
                || reason.contains("resolved Sage ")
        });
        if features.presentation && !precise_lookup {
            if let Some(record) = &resolved {
                candidate_count = candidate_count.max(self.symbol_candidates(&record.name).len());
            }
        }
        let definition_candidates: Vec<_> = navigation_candidates
            .iter()
            .take(5)
            .filter_map(|candidate| {
                Some(QueryDefinitionCandidate {
                    definition: query_definition_from_record(candidate)?,
                    confidence: "candidate".to_string(),
                    reason: resolution_reason.clone().unwrap_or_default(),
                    signature: candidate.signature.clone(),
                    summary: candidate
                        .docstring
                        .as_deref()
                        .and_then(documentation_summary),
                })
            })
            .collect();
        let mut documentation = if features.presentation {
            resolved
                .as_ref()
                .map(|record| self.documentation_for_resolved_symbol(record))
        } else {
            None
        };
        let mut hover = if features.presentation {
            resolved.as_ref().map(|record| QueryHover {
                markdown: hover_markdown_for_symbol(record, documentation.as_ref()),
                range: target_range.clone(),
            })
        } else {
            None
        };
        if features.presentation && resolved.is_none() {
            if let Some(ambiguous_documentation) = resolution_reason.as_deref().and_then(|reason| {
                self.ambiguous_member_documentation(
                    lookup_name,
                    reason,
                    candidate_count,
                    &navigation_candidates,
                )
            }) {
                hover = Some(QueryHover {
                    markdown: hover_markdown_for_ambiguous_member(&ambiguous_documentation),
                    range: target_range.clone(),
                });
                documentation = Some(ambiguous_documentation);
            }
        }
        let definition = resolved.as_ref().and_then(query_definition_from_record);
        let completions = if features.completions {
            self.completion_items_at_source_with_fallback(
                source,
                QueryPosition {
                    line: target_range.start_line,
                    character: target_range.start_character,
                },
                80,
                Some(lookup_name),
            )
        } else {
            Vec::new()
        };
        let should_collect_references = features.references || features.rename_preview;
        let has_unique_high_confidence_definition =
            resolved.is_some() && resolution_confidence.as_deref() == Some("high");
        let read_only_definition = resolved.as_ref().is_some_and(|record| {
            !record.path.as_os_str().is_empty() && !self.is_editable_path(&record.path)
        });
        let mut references = if should_collect_references
            && has_unique_high_confidence_definition
            && !read_only_definition
        {
            scope_references_for_resolved_symbol(
                self.editable_references(lookup_name),
                resolved.as_ref(),
                &query_path,
            )
        } else {
            Vec::new()
        };
        if let Some(parameter) = resolved
            .as_ref()
            .filter(|record| is_local_parameter_symbol(record))
        {
            let local_module = module_hint.as_deref().unwrap_or("document");
            references.retain(|reference| {
                normalize_path(reference.path.clone()) == query_path
                    && local_parameter_reference_matches(
                        local_module,
                        &query_path,
                        source,
                        &reference.range,
                        parameter,
                    )
            });
        }
        let rename_preview = if features.rename_preview {
            rename_to
                .filter(|new_name| is_valid_identifier(new_name))
                .map(|new_name| {
                    references
                        .iter()
                        .map(|reference| QueryTextEdit {
                            path: reference.path.clone(),
                            range: reference.range.clone(),
                            new_text: new_name.to_string(),
                        })
                        .collect()
                })
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let signature = if features.signature {
            let ambiguous_member = member_resolution
                .as_ref()
                .is_some_and(|resolution| resolution.record.is_none());
            (!ambiguous_member)
                .then(|| {
                    target_is_code
                        .then(|| {
                            function_call_at_position_with_code_map(
                                source,
                                target_range.start_line,
                                target_range.start_character,
                                source_map,
                            )
                        })
                        .flatten()
                        .and_then(|(name, active_parameter)| {
                            resolved
                                .as_ref()
                                .filter(|record| record.name == name)
                                .cloned()
                                .or_else(|| self.resolve_symbol(&name, module_hint.as_deref()))
                                .or_else(|| builtin_symbol_record(&name))
                                .and_then(|record| {
                                    record.signature.clone().map(|label| QuerySignature {
                                        label,
                                        active_parameter,
                                        documentation: record.docstring,
                                    })
                                })
                        })
                        .or_else(|| {
                            resolved.as_ref().and_then(|record| {
                                record.signature.clone().map(|label| QuerySignature {
                                    label,
                                    active_parameter: 0,
                                    documentation: record.docstring.clone(),
                                })
                            })
                        })
                })
                .flatten()
        } else {
            None
        };
        let fallback_reason = resolved.as_ref().is_none().then(|| {
            resolution_reason.clone().unwrap_or_else(|| {
                member_resolution
                    .as_ref()
                    .filter(|resolution| resolution.suppress_global_fallback)
                    .map(|resolution| resolution.reason.clone())
                    .unwrap_or_else(|| "symbol-not-in-index-or-known-sage-set".to_string())
            })
        });

        QueryResult {
            target: Some(QueryTarget {
                symbol: symbol.to_string(),
                dotted_symbol,
                range: target_range,
            }),
            hover,
            documentation,
            definition,
            definition_candidates,
            completions,
            references,
            rename_preview,
            signature,
            diagnostics,
            fallback_reason,
            resolution_confidence,
            resolution_reason,
            owner_type,
            candidate_count,
        }
    }

    pub fn all_files(&self) -> Vec<IndexedFile> {
        self.files.values().cloned().collect()
    }

    fn module_hint_for_query_path(&self, query_path: &Path) -> Option<String> {
        self.file_for_path(query_path)
            .map(|file| file.module)
            .or_else(|| {
                self.options
                    .roots
                    .iter()
                    .find(|root| query_path.strip_prefix(root).is_ok())
                    .map(|root| module_name_from_path(root, query_path))
            })
    }

    fn ambiguous_member_documentation(
        &self,
        member: &str,
        reason: &str,
        candidate_count: usize,
        candidates: &[SymbolRecord],
    ) -> Option<DocumentationRecord> {
        let candidates = dedupe_symbol_records(candidates.to_vec());
        if candidates.is_empty() {
            return None;
        }
        let sections = candidates
            .iter()
            .take(5)
            .map(|candidate| {
                let mut body = Vec::new();
                if let Some(signature) = &candidate.signature {
                    body.push(format!("```sage\n{signature}\n```"));
                }
                if let Some(summary) = candidate
                    .docstring
                    .as_deref()
                    .and_then(documentation_summary)
                {
                    body.push(summary.to_string());
                }
                body.push(format!("Module: `{}`", candidate.module));
                if !candidate.path.as_os_str().is_empty() {
                    body.push(format!("Source: `{}`", candidate.path.display()));
                }
                DocumentationSection {
                    title: format!("{} — {}", candidate.detail, candidate.module),
                    body: body.join("\n\n"),
                }
            })
            .collect();
        Some(DocumentationRecord {
            name: member.to_string(),
            module_name: "ambiguous".to_string(),
            kind: "AmbiguousMember".to_string(),
            detail: format!("Ambiguous Sage member `{member}`"),
            summary: format!(
                "Ambiguous Sage member `{member}` has {candidate_count} ranked candidates; a multiple-definition preview is available when at least two targets remain."
            ),
            docstring: Some(format!(
                "Reason: {reason}\n\nRefine the receiver type to obtain a single definition, reference set, or rename target."
            )),
            uri: None,
            markers: vec![
                "ambiguous".to_string(),
                "source:rust-index-v2".to_string(),
            ],
            sections,
        })
    }
}
