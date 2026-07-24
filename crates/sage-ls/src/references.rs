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
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::{
    Location, Position, PrepareRenameResponse, Range, ReferenceParams, RenameParams,
    TextDocumentPositionParams, TextEdit, Url, WorkspaceEdit,
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
        let mut changes: HashMap<Url, Vec<TextEdit>> = HashMap::new();
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
        Ok(Some(WorkspaceEdit {
            changes: Some(changes),
            document_changes: None,
            change_annotations: None,
        }))
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

    async fn rename_target(&self, uri: &Url, position: Position) -> Option<RenameTarget> {
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
        uri: &Url,
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
            let parsed = index.parse_source_for_query(&live.path, &live.document.text);
            for reference in
                sage_index::references_in_source(&live.path, &live.document.text, &target.word)
            {
                if !reference_candidate_matches_target_with_symbols(
                    &index,
                    &live.path,
                    &live.document.text,
                    &reference.range,
                    target,
                    Some(&parsed.symbols),
                ) {
                    continue;
                }
                push_scoped_reference_location(
                    &mut locations,
                    &mut seen,
                    Location {
                        uri: live.uri.clone(),
                        range: lsp_range_for_text(&live.document.text, &reference.range),
                    },
                    target.declaration.as_ref(),
                    include_declaration,
                );
            }
        }
        drop(index);
        locations
    }
}

#[cfg(test)]
pub(super) fn local_import_alias_rename_target(
    uri: &Url,
    physical_path: &Path,
    text: &str,
    word: &str,
    range: Range,
) -> Option<RenameTarget> {
    let symbols = parse_source(module_name_for_path(physical_path), physical_path, text).symbols;
    local_import_alias_rename_target_with_symbols(uri, text, word, range, &symbols)
}

pub(super) fn local_import_alias_rename_target_with_symbols(
    uri: &Url,
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
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first == '_' || first.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
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
            let Ok(uri) = Url::from_file_path(&path) else {
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
