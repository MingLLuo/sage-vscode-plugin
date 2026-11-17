//! Identity-safe reference collection and rename orchestration.
//!
//! Reference and rename operations deliberately require the same high-confidence
//! owner identity used by navigation. This keeps broad workspace scans from turning
//! a same-name symbol into an unrelated edit.

use super::navigation::{live_definition_range, location_reference_key};
use super::open_documents::{
    canonical_path_for_comparison, physical_paths as open_document_physical_paths,
    unique_live_documents, uri_to_path,
};
use super::source_symbols::module_name_for_path;
use super::text_positions::{lsp_range_for_text, query_position_from_lsp, word_at_position};
use super::Backend;
use rayon::prelude::*;
use sage_index::{
    local_import_alias_symbol_from_source, local_import_alias_symbol_from_source_name,
    local_import_alias_symbol_from_symbols, parse_source, QueryDefinition, QueryPosition,
    QueryResult, SymbolRecord, WorkspaceIndex,
};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use tower_lsp_server::jsonrpc::Result;
use tower_lsp_server::ls_types::{
    DocumentChanges, DocumentHighlight, DocumentHighlightKind, DocumentHighlightParams, Location,
    OneOf, OptionalVersionedTextDocumentIdentifier, Position, PrepareRenameResponse, Range,
    ReferenceParams, RenameParams, TextDocumentEdit, TextDocumentPositionParams, TextEdit, Uri,
    WorkspaceEdit,
};

#[derive(Clone, Debug)]
pub(super) struct RenameTarget {
    pub(super) word: String,
    pub(super) range: Range,
    pub(super) definition: QueryDefinition,
    pub(super) definition_ranges: Vec<sage_index::SourceRange>,
    pub(super) declaration: Location,
    pub(super) local_import_alias: Option<SymbolRecord>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ReferenceCollectionMode {
    References,
    Rename,
}

#[derive(Clone, Debug)]
pub(super) struct ResolvedReferenceTarget {
    pub(super) word: String,
    pub(super) range: Range,
    pub(super) definition: QueryDefinition,
    pub(super) definition_ranges: Vec<sage_index::SourceRange>,
    pub(super) declaration: Option<Location>,
    pub(super) local_import_alias: Option<SymbolRecord>,
}

impl Backend {
    pub(super) async fn document_highlight_response(
        &self,
        params: DocumentHighlightParams,
    ) -> Result<Option<Vec<DocumentHighlight>>> {
        let uri = &params.text_document_position_params.text_document.uri;
        let Some(document) = self.document_for_uri(uri).await else {
            return Ok(Some(Vec::new()));
        };
        let Some(path) = uri_to_path(uri) else {
            return Ok(Some(Vec::new()));
        };
        let Some((word, target_range)) = word_at_position(
            &document.text,
            params.text_document_position_params.position,
        ) else {
            return Ok(Some(Vec::new()));
        };
        if !is_valid_identifier(&word)
            || !is_code_reference_range(&document.text, &word, target_range)
        {
            return Ok(Some(Vec::new()));
        }
        if let Some(target_position) = source_range_from_lsp(&document.text, target_range) {
            if let Some(ranges) = sage_index::local_document_highlight_ranges(
                &document.text,
                &word,
                target_position.start_line,
                target_position.start_character,
            ) {
                return Ok(Some(
                    ranges
                        .into_iter()
                        .map(|range| DocumentHighlight {
                            range: lsp_range_for_text(&document.text, &range),
                            kind: Some(DocumentHighlightKind::TEXT),
                        })
                        .collect(),
                ));
            }
        }
        let Some(target) = self
            .resolved_reference_target_at(uri, params.text_document_position_params.position)
            .await
        else {
            return Ok(Some(Vec::new()));
        };
        let index = self.index.read().await;
        let parsed = index.parse_source_for_query(&path, &document.text);
        let highlights = sage_index::references_in_source(&path, &document.text, &target.word)
            .into_iter()
            .filter(|reference| {
                reference_candidate_matches_target_with_symbols(
                    &index,
                    &path,
                    &document.text,
                    &reference.range,
                    &target,
                    Some(&parsed.symbols),
                )
            })
            .map(|reference| DocumentHighlight {
                range: lsp_range_for_text(&document.text, &reference.range),
                kind: Some(DocumentHighlightKind::TEXT),
            })
            .collect();
        Ok(Some(highlights))
    }

    pub(super) async fn references_response(
        &self,
        params: ReferenceParams,
    ) -> Result<Option<Vec<Location>>> {
        let uri = &params.text_document_position.text_document.uri;
        let Some(target) = self
            .resolved_reference_target_at(uri, params.text_document_position.position)
            .await
        else {
            return Ok(Some(Vec::new()));
        };
        Ok(Some(
            self.reference_locations(
                &target,
                params.context.include_declaration,
                ReferenceCollectionMode::References,
            )
            .await,
        ))
    }

    pub(super) async fn rename_response(
        &self,
        params: RenameParams,
    ) -> Result<Option<WorkspaceEdit>> {
        let versions_before: HashMap<Uri, i32> = self
            .open_documents
            .read()
            .await
            .iter()
            .map(|(uri, document)| (uri.clone(), document.version))
            .collect();
        let uri = &params.text_document_position.text_document.uri;
        let Some(target) = self
            .rename_target(uri, params.text_document_position.position)
            .await
        else {
            return Ok(None);
        };
        if !is_valid_identifier(&params.new_name) {
            return Ok(None);
        }
        if self.rename_would_collide(&target, &params.new_name).await {
            return Ok(None);
        }
        let mut changes: HashMap<Uri, Vec<TextEdit>> = HashMap::new();
        for location in self
            .reference_locations(
                &ResolvedReferenceTarget {
                    word: target.word.clone(),
                    range: target.range,
                    definition: target.definition.clone(),
                    definition_ranges: target.definition_ranges.clone(),
                    declaration: Some(target.declaration),
                    local_import_alias: target.local_import_alias,
                },
                true,
                ReferenceCollectionMode::Rename,
            )
            .await
        {
            changes.entry(location.uri).or_default().push(TextEdit {
                range: location.range,
                new_text: params.new_name.clone(),
            });
        }
        if *self.versioned_workspace_edits.read().await {
            let documents = self.open_documents.read().await;
            if changes.keys().any(|uri| {
                documents.get(uri).map(|document| document.version)
                    != versions_before.get(uri).copied()
            }) {
                return Ok(None);
            }
            let edits = changes
                .into_iter()
                .map(|(uri, edits)| {
                    let version = documents.get(&uri).map(|document| document.version);
                    TextDocumentEdit {
                        text_document: OptionalVersionedTextDocumentIdentifier { uri, version },
                        edits: edits.into_iter().map(OneOf::Left).collect(),
                    }
                })
                .collect();
            Ok(Some(WorkspaceEdit {
                changes: None,
                document_changes: Some(DocumentChanges::Edits(edits)),
                change_annotations: None,
            }))
        } else {
            Ok(Some(WorkspaceEdit {
                changes: Some(changes),
                document_changes: None,
                change_annotations: None,
            }))
        }
    }

    async fn rename_would_collide(&self, target: &RenameTarget, new_name: &str) -> bool {
        if target.word == new_name {
            return false;
        }
        let text = if let Some(document) = self.document_for_uri(&target.declaration.uri).await {
            document.text
        } else if let Ok(on_disk) = std::fs::read_to_string(&target.definition.path) {
            on_disk
        } else {
            return false;
        };
        let index = self.index.read().await;
        let parsed = index.parse_source_for_query(&target.definition.path, &text);
        let target_line = target.declaration.range.start.line as usize;
        let target_indent = text
            .lines()
            .nth(target_line)
            .map(|line| line.len() - line.trim_start().len());
        if target_indent != Some(0) {
            return false;
        }
        parsed.symbols.iter().any(|symbol| {
            symbol.name == new_name
                && symbol.range.start_line != target.declaration.range.start.line
                && text
                    .lines()
                    .nth(symbol.range.start_line as usize)
                    .is_some_and(|line| line.len() == line.trim_start().len())
        })
    }

    pub(super) async fn prepare_rename_response(
        &self,
        params: TextDocumentPositionParams,
    ) -> Result<Option<PrepareRenameResponse>> {
        let Some(target) = self
            .rename_target(&params.text_document.uri, params.position)
            .await
        else {
            return Ok(None);
        };
        Ok(Some(PrepareRenameResponse::RangeWithPlaceholder {
            range: target.range,
            placeholder: target.word,
        }))
    }

    async fn rename_target(&self, uri: &Uri, position: Position) -> Option<RenameTarget> {
        let document = self.document_for_uri(uri).await?;
        let path = uri_to_path(uri)?;
        let physical_path = canonical_path_for_comparison(&path);
        let (word, range) = word_at_position(&document.text, position)?;
        if !is_valid_identifier(&word) || !is_code_reference_range(&document.text, &word, range) {
            return None;
        }
        let parsed = self
            .index
            .read()
            .await
            .parse_source_for_query(&physical_path, &document.text);
        if let Some(target) = local_import_alias_rename_target_with_symbols(
            uri,
            &document.text,
            &word,
            range,
            &parsed.symbols,
        ) {
            if !self
                .index
                .read()
                .await
                .is_editable_path(&target.definition.path)
            {
                return None;
            }
            return Some(target);
        }
        let target = self.resolved_reference_target_at(uri, position).await?;
        let is_editable = self
            .index
            .read()
            .await
            .is_editable_path(&target.definition.path);
        if !is_editable {
            return None;
        }
        Some(RenameTarget {
            word: target.word,
            range: target.range,
            definition: target.definition,
            definition_ranges: target.definition_ranges,
            declaration: target.declaration?,
            local_import_alias: None,
        })
    }

    pub(super) async fn resolved_reference_target_at(
        &self,
        uri: &Uri,
        position: Position,
    ) -> Option<ResolvedReferenceTarget> {
        let document = self.document_for_uri(uri).await?;
        let path = uri_to_path(uri)?;
        let (word, range) = word_at_position(&document.text, position)?;
        if !is_valid_identifier(&word) || !is_code_reference_range(&document.text, &word, range) {
            return None;
        }
        let query = self
            .navigation_query_for_document(uri, &document, &path, position)
            .await;
        if query.resolution_confidence.as_deref() != Some("high") {
            return None;
        }
        let definition = query.definition?;
        let definition_ranges = self.definition_identity_ranges(&definition).await;
        let declaration = self.location_for_query_definition(&definition).await;
        Some(ResolvedReferenceTarget {
            word,
            range,
            definition,
            definition_ranges,
            declaration,
            local_import_alias: None,
        })
    }

    async fn definition_identity_ranges(
        &self,
        definition: &QueryDefinition,
    ) -> Vec<sage_index::SourceRange> {
        let mut ranges = vec![definition.range.clone()];
        if let Some((_, document)) = self.open_document_for_path(&definition.path).await {
            if let Some(range) = live_definition_range(definition, &document.text) {
                if !ranges.contains(&range) {
                    ranges.push(range);
                }
            }
        }
        if let Ok(text) = std::fs::read_to_string(&definition.path) {
            let mut matching = parse_source(
                module_name_for_path(&definition.path),
                &definition.path,
                &text,
            )
            .symbols
            .into_iter()
            .filter(|symbol| symbol.name == definition.name && symbol.detail == definition.detail);
            if let Some(symbol) = matching.next() {
                if matching.next().is_none() && !ranges.contains(&symbol.range) {
                    ranges.push(symbol.range);
                }
            }
        }
        ranges
    }

    pub(super) async fn reference_locations(
        &self,
        target: &ResolvedReferenceTarget,
        include_declaration: bool,
        mode: ReferenceCollectionMode,
    ) -> Vec<Location> {
        let mut seen = BTreeSet::new();
        let mut locations = Vec::new();
        let open_documents = self.open_documents.read().await.clone();
        let open_paths: BTreeSet<PathBuf> = open_document_physical_paths(&open_documents)
            .into_iter()
            .collect();
        if include_declaration {
            if let Some(declaration) = target.declaration.clone() {
                push_reference_location(&mut locations, &mut seen, declaration);
            }
        }
        let index = self.index.read().await;
        let indexed_locations = indexed_reference_locations(&index, target, mode, &open_paths);
        for location in indexed_locations {
            push_scoped_reference_location(
                &mut locations,
                &mut seen,
                location,
                target.declaration.as_ref(),
                include_declaration,
            );
        }
        for live in unique_live_documents(&open_documents) {
            if !reference_path_is_collectible(&index, &live.path, mode) {
                continue;
            }
            collect_source_reference_locations(
                &index,
                &live.uri,
                &live.path,
                &live.document.text,
                target,
                include_declaration,
                &mut locations,
                &mut seen,
            );
        }
        // Same-file references (sibling callers, recursion) must not depend on background index
        // readiness: always scan the definition's own file when no live document covers it.
        // Indexed hits for the same file use the same path-derived URI, so `seen` deduplicates.
        let definition_path = &target.definition.path;
        if !open_paths.contains(&canonical_path_for_comparison(definition_path))
            && reference_path_is_collectible(&index, definition_path, mode)
        {
            if let (Ok(text), Some(uri)) = (
                std::fs::read_to_string(definition_path),
                Uri::from_file_path(definition_path),
            ) {
                collect_source_reference_locations(
                    &index,
                    &uri,
                    definition_path,
                    &text,
                    target,
                    include_declaration,
                    &mut locations,
                    &mut seen,
                );
            }
        }
        drop(index);
        locations
    }
}

#[allow(clippy::too_many_arguments)]
fn collect_source_reference_locations(
    index: &WorkspaceIndex,
    uri: &Uri,
    path: &Path,
    text: &str,
    target: &ResolvedReferenceTarget,
    include_declaration: bool,
    locations: &mut Vec<Location>,
    seen: &mut BTreeSet<String>,
) {
    let parsed = index.parse_source_for_query(path, text);
    for reference in sage_index::references_in_source(path, text, &target.word) {
        if !reference_candidate_matches_target_with_symbols(
            index,
            path,
            text,
            &reference.range,
            target,
            Some(&parsed.symbols),
        ) {
            continue;
        }
        push_scoped_reference_location(
            locations,
            seen,
            Location {
                uri: uri.clone(),
                range: lsp_range_for_text(text, &reference.range),
            },
            target.declaration.as_ref(),
            include_declaration,
        );
    }
}

#[cfg(test)]
pub(super) fn local_import_alias_rename_target(
    uri: &Uri,
    physical_path: &Path,
    text: &str,
    word: &str,
    range: Range,
) -> Option<RenameTarget> {
    let symbols = parse_source(module_name_for_path(physical_path), physical_path, text).symbols;
    local_import_alias_rename_target_with_symbols(uri, text, word, range, &symbols)
}

pub(super) fn local_import_alias_rename_target_with_symbols(
    uri: &Uri,
    text: &str,
    word: &str,
    range: Range,
    symbols: &[SymbolRecord],
) -> Option<RenameTarget> {
    let source_range = source_range_from_lsp(text, range)?;
    let alias = local_import_alias_symbol_from_symbols(text, symbols, word, &source_range)?;
    let definition = QueryDefinition {
        name: alias.name.clone(),
        path: alias.path.clone(),
        range: alias.range.clone(),
        detail: alias.detail.clone(),
        module: alias.module.clone(),
    };
    Some(RenameTarget {
        word: word.to_string(),
        range,
        definition,
        definition_ranges: vec![alias.range.clone()],
        declaration: Location {
            uri: uri.clone(),
            range: lsp_range_for_text(text, &alias.range),
        },
        local_import_alias: Some(alias),
    })
}

fn is_code_reference_range(text: &str, word: &str, target_range: Range) -> bool {
    source_range_from_lsp(text, target_range)
        .is_some_and(|range| sage_index::is_code_reference_at_range(text, word, &range))
}

fn source_range_from_lsp(text: &str, range: Range) -> Option<sage_index::SourceRange> {
    let start = query_position_from_lsp(text, range.start)?;
    let end = query_position_from_lsp(text, range.end)?;
    Some(sage_index::SourceRange {
        start_line: start.line,
        start_character: start.character,
        end_line: end.line,
        end_character: end.character,
    })
}

fn is_valid_identifier(value: &str) -> bool {
    sage_index::identifiers::is_identifier(value)
}

fn push_reference_location(
    locations: &mut Vec<Location>,
    seen: &mut BTreeSet<String>,
    location: Location,
) {
    let key = location_reference_key(&location.uri, &location.range);
    if seen.insert(key) {
        locations.push(location);
    }
}

pub(super) fn push_scoped_reference_location(
    locations: &mut Vec<Location>,
    seen: &mut BTreeSet<String>,
    location: Location,
    declaration: Option<&Location>,
    include_declaration: bool,
) {
    if !include_declaration
        && declaration.is_some_and(|declaration| same_physical_location(declaration, &location))
    {
        return;
    }
    push_reference_location(locations, seen, location);
}

fn same_physical_location(left: &Location, right: &Location) -> bool {
    if left.range != right.range {
        return false;
    }
    match (uri_to_path(&left.uri), uri_to_path(&right.uri)) {
        (Some(left), Some(right)) => {
            canonical_path_for_comparison(&left) == canonical_path_for_comparison(&right)
        }
        _ => left.uri == right.uri,
    }
}

#[cfg(test)]
pub(super) fn reference_candidate_matches_target(
    index: &WorkspaceIndex,
    path: &Path,
    text: &str,
    range: &sage_index::SourceRange,
    target: &ResolvedReferenceTarget,
) -> bool {
    reference_candidate_matches_target_with_symbols(index, path, text, range, target, None)
}

pub(super) fn reference_candidate_matches_target_with_symbols(
    index: &WorkspaceIndex,
    path: &Path,
    text: &str,
    range: &sage_index::SourceRange,
    target: &ResolvedReferenceTarget,
    local_symbols: Option<&[SymbolRecord]>,
) -> bool {
    if let Some(alias) = target.local_import_alias.as_ref() {
        if canonical_path_for_comparison(path) != canonical_path_for_comparison(&alias.path) {
            return false;
        }
        let candidate = if let Some(symbols) = local_symbols {
            local_import_alias_symbol_from_symbols(text, symbols, &target.word, range)
        } else {
            local_import_alias_symbol_from_source(
                &alias.module,
                &alias.path,
                text,
                &target.word,
                range,
            )
        };
        return candidate.is_some_and(|candidate| {
            candidate.name == alias.name
                && candidate.module == alias.module
                && canonical_path_for_comparison(&candidate.path)
                    == canonical_path_for_comparison(&alias.path)
                && candidate.range == alias.range
                && candidate.import_from == alias.import_from
        });
    }
    let parsed_symbols;
    let symbols = if let Some(symbols) = local_symbols {
        symbols
    } else {
        parsed_symbols = parse_source(module_name_for_path(path), path, text).symbols;
        &parsed_symbols
    };
    let position = QueryPosition {
        line: range.start_line,
        character: range.start_character,
    };
    let query = if let Some(symbols) = local_symbols {
        index.query_source_definition_with_symbols(path, text, position, symbols)
    } else {
        index.query_source_at_navigation(path, text, position)
    };
    reference_query_or_aliased_import_matches_target(
        index, path, text, range, target, symbols, &query,
    )
}

fn reference_query_or_aliased_import_matches_target(
    index: &WorkspaceIndex,
    path: &Path,
    text: &str,
    range: &sage_index::SourceRange,
    target: &ResolvedReferenceTarget,
    symbols: &[SymbolRecord],
    query: &QueryResult,
) -> bool {
    if reference_query_matches_target(query, target) {
        return true;
    }
    // `from provider import target as alias` contains two distinct rename domains. A rename
    // started at `target` must update the imported source token, while preserving the local
    // binding and all `alias(...)` uses. Resolve through the alias declaration and retain the
    // existing high-confidence identity gate before accepting that source token.
    let Some(alias) =
        local_import_alias_symbol_from_source_name(text, symbols, &target.word, range)
    else {
        return false;
    };
    let alias_query = index.query_source_definition_with_symbols(
        path,
        text,
        QueryPosition {
            line: alias.range.start_line,
            character: alias.range.start_character,
        },
        symbols,
    );
    reference_query_matches_target(&alias_query, target)
}

pub(super) fn indexed_reference_locations(
    index: &WorkspaceIndex,
    target: &ResolvedReferenceTarget,
    mode: ReferenceCollectionMode,
    open_paths: &BTreeSet<PathBuf>,
) -> Vec<Location> {
    let indexed_references = match mode {
        ReferenceCollectionMode::References => index.references(&target.word),
        ReferenceCollectionMode::Rename => index.editable_references(&target.word),
    };
    let mut references_by_path: BTreeMap<PathBuf, Vec<sage_index::ReferenceRecord>> =
        BTreeMap::new();
    for reference in indexed_references {
        if open_paths.contains(&canonical_path_for_comparison(&reference.path)) {
            continue;
        }
        references_by_path
            .entry(reference.path.clone())
            .or_default()
            .push(reference);
    }
    let mut locations: Vec<Location> = references_by_path
        .into_iter()
        .collect::<Vec<_>>()
        .into_par_iter()
        .map(|(path, references)| {
            let Ok(text) = std::fs::read_to_string(&path) else {
                return Vec::new();
            };
            let Some(uri) = Uri::from_file_path(&path) else {
                return Vec::new();
            };
            let parsed = index
                .fresh_file_for_query(&path)
                .unwrap_or_else(|| index.parse_source_for_query(&path, &text));
            let ranges: Vec<_> = references
                .iter()
                .map(|reference| reference.range.clone())
                .collect();
            let queries = index.query_source_definitions_for_ranges_with_symbols(
                &path,
                &text,
                &target.word,
                &ranges,
                &parsed.symbols,
            );
            references
                .into_iter()
                .zip(queries)
                .filter(|(reference, query)| {
                    reference_query_or_aliased_import_matches_target(
                        index,
                        &path,
                        &text,
                        &reference.range,
                        target,
                        &parsed.symbols,
                        query,
                    )
                })
                .map(|(reference, _)| Location {
                    uri: uri.clone(),
                    range: lsp_range_for_text(&text, &reference.range),
                })
                .collect()
        })
        .flatten()
        .collect();
    locations.sort_by(|left, right| {
        left.uri
            .as_str()
            .cmp(right.uri.as_str())
            .then(left.range.start.line.cmp(&right.range.start.line))
            .then(left.range.start.character.cmp(&right.range.start.character))
    });
    locations
}

fn reference_query_matches_target(query: &QueryResult, target: &ResolvedReferenceTarget) -> bool {
    query.resolution_confidence.as_deref() == Some("high")
        && query.definition.as_ref().is_some_and(|candidate| {
            same_definition_owner_identity(&target.definition, candidate)
                && target.definition_ranges.contains(&candidate.range)
        })
}

pub(super) fn reference_path_is_collectible(
    index: &WorkspaceIndex,
    path: &Path,
    mode: ReferenceCollectionMode,
) -> bool {
    mode == ReferenceCollectionMode::References
        || index.is_editable_path(&canonical_path_for_comparison(path))
}

#[cfg(test)]
pub(super) fn same_definition_identity(left: &QueryDefinition, right: &QueryDefinition) -> bool {
    same_definition_owner_identity(left, right) && left.range == right.range
}

pub(super) fn same_definition_owner_identity(
    left: &QueryDefinition,
    right: &QueryDefinition,
) -> bool {
    left.name == right.name
        && left.module == right.module
        && left.detail == right.detail
        && canonical_path_for_comparison(&left.path) == canonical_path_for_comparison(&right.path)
}
