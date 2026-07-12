use super::*;

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

    pub fn type_definition_at_source(
        &self,
        _path: &Path,
        source: &str,
        position: QueryPosition,
    ) -> Option<QueryDefinition> {
        let (word, _range) = word_at_source_position(source, position.line, position.character)?;
        let constructor = assignment_constructor_before_line(source, &word, position.line);
        let type_symbol = constructor
            .as_deref()
            .and_then(type_symbol_for_constructor)
            .or_else(|| {
                infer_owner_type_before(source, &word, "", position.line)
                    .and_then(type_symbol_for_owner_type)
            })?;
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
        self.query_source_symbol_with_options(
            path,
            source,
            &word,
            Some(range),
            QueryExecutionOptions {
                rename_to,
                diagnostics,
                features,
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
        let QueryExecutionOptions {
            rename_to,
            diagnostics,
            features,
        } = options;
        let query_path = normalize_path(path.to_path_buf());
        let target_range = known_range
            .or_else(|| range_for_first_symbol(source, symbol))
            .unwrap_or_default();
        let source_map = CodeMap::new(source);
        let target_is_code = source_map
            .offset(target_range.start_line, target_range.start_character)
            .is_some_and(|offset| source_map.is_code_offset(offset));
        let dotted_symbol = dotted_symbol_at_range(source, &target_range);
        let lookup_name = dotted_symbol
            .as_deref()
            .and_then(|value| value.rsplit('.').next())
            .filter(|value| !value.is_empty())
            .unwrap_or(symbol);
        let module_hint = self
            .file_for_path(&query_path)
            .map(|file| file.module)
            .or_else(|| {
                self.options
                    .roots
                    .iter()
                    .find(|root| query_path.strip_prefix(root).is_ok())
                    .map(|root| module_name_from_path(root, &query_path))
            });
        let dotted_owner_member = dotted_symbol.as_deref().and_then(dotted_owner_member);
        let source_import_lookup = source_explicit_import_lookup(source, lookup_name);
        let sage_all_export_lookup = source_imported_sage_all_lookup(source, lookup_name);
        let implicit_sage_all_lookup =
            is_sage_source_path(&query_path) && dotted_symbol.is_none() && target_is_code;
        let member_resolution = dotted_owner_member.map(|(owner, member)| {
            self.resolve_member_symbol(
                source,
                owner,
                member,
                module_hint.as_deref(),
                target_range.start_line,
            )
        });
        let mut suppress_global_fallback = member_resolution
            .as_ref()
            .is_some_and(|resolution| resolution.suppress_global_fallback);
        let mut resolution_confidence = member_resolution
            .as_ref()
            .map(|resolution| resolution.confidence.to_string());
        let mut resolution_reason = member_resolution
            .as_ref()
            .map(|resolution| resolution.reason.clone());
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
        if resolved.is_none()
            && !suppress_global_fallback
            && dotted_symbol.is_none()
            && target_is_code
            && (implicit_sage_all_lookup
                || sage_all_export_lookup.is_some()
                || source_import_lookup.is_some())
        {
            let local_module = module_hint.as_deref().unwrap_or("document");
            if let Some(local_symbol) = local_shadow_symbol_from_source(
                local_module,
                &query_path,
                source,
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
                if let Some(record) = self
                    .symbol_candidates(&import_lookup.source_name)
                    .into_iter()
                    .filter(|candidate| {
                        import_target_definition_matches(
                            candidate,
                            &import_module,
                            &import_lookup.source_name,
                        )
                    })
                    .min_by_key(symbol_choice_key)
                    .or_else(|| {
                        self.resolve_module_symbol_from_roots(
                            &import_module,
                            &import_lookup.source_name,
                            0,
                            &mut seen,
                        )
                    })
                {
                    resolution_confidence = Some("high".to_string());
                    resolution_reason = Some(format!(
                        "resolved `{lookup_name}` from explicit import target {}",
                        import_module
                    ));
                    resolved = Some(record);
                } else {
                    suppress_global_fallback = true;
                    resolution_confidence = Some("ambiguous".to_string());
                    resolution_reason = Some(format!(
                        "`{lookup_name}` is explicitly imported from {} but the target module is not indexed or resolvable",
                        import_module
                    ));
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
        if resolved.is_none() && !suppress_global_fallback {
            resolved = self
                .resolve_symbol(lookup_name, module_hint.as_deref())
                .or_else(|| self.resolve_symbol(symbol, module_hint.as_deref()))
                .or_else(|| builtin_symbol_record(dotted_symbol.as_deref().unwrap_or(symbol)))
                .or_else(|| builtin_symbol_record(lookup_name));
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
        if !precise_lookup {
            if let Some(record) = &resolved {
                candidate_count = candidate_count.max(self.symbol_candidates(&record.name).len());
            }
        }
        let mut documentation = resolved
            .as_ref()
            .map(|record| self.documentation_for_resolved_symbol(record));
        let mut hover = resolved.as_ref().map(|record| QueryHover {
            markdown: hover_markdown_for_symbol(record, documentation.as_ref()),
            range: target_range.clone(),
        });
        if resolved.is_none() {
            if let Some(ambiguous_documentation) =
                member_resolution.as_ref().and_then(|resolution| {
                    self.ambiguous_member_documentation(
                        lookup_name,
                        &resolution.reason,
                        resolution.candidate_count,
                    )
                })
            {
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
        let read_only_definition = resolved.as_ref().is_some_and(|record| {
            !record.path.as_os_str().is_empty() && !self.is_editable_path(&record.path)
        });
        let references = if should_collect_references && !read_only_definition {
            scope_references_for_resolved_symbol(
                self.editable_references(lookup_name),
                resolved.as_ref(),
                &query_path,
            )
        } else {
            Vec::new()
        };
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
            target_is_code
                .then(|| {
                    function_call_at_position_with_code_map(
                        source,
                        target_range.start_line,
                        target_range.start_character,
                        &source_map,
                    )
                })
                .flatten()
                .and_then(|(name, active_parameter)| {
                    self.resolve_symbol(&name, module_hint.as_deref())
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

    fn ambiguous_member_documentation(
        &self,
        member: &str,
        reason: &str,
        candidate_count: usize,
    ) -> Option<DocumentationRecord> {
        let mut candidates: Vec<_> = self
            .symbol_candidates(member)
            .into_iter()
            .filter(|symbol| symbol.kind != SymbolKind::Import)
            .filter(|symbol| symbol.signature.is_some() || symbol.docstring.is_some())
            .collect();
        if candidates.is_empty() {
            return None;
        }
        candidates.sort_by_key(symbol_choice_key);
        candidates.dedup_by(|left, right| {
            left.name == right.name
                && left.module == right.module
                && left.path == right.path
                && left.range == right.range
        });
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
                    title: candidate.detail.clone(),
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
                "Ambiguous Sage member `{member}` has {candidate_count} indexed candidates; no definition jump was returned to avoid a wrong target."
            ),
            docstring: Some(format!(
                "Reason: {reason}\n\nUse completion or refine the receiver type to choose a specific implementation."
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
