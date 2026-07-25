//! LSP navigation orchestration and navigation-query caching.
//!
//! The `LanguageServer` implementation in `main.rs` intentionally remains a thin
//! protocol boundary. This module owns the definition/declaration/type-definition/
//! implementation behavior and the helpers that turn Sage index results into LSP
//! locations.

use super::call_hierarchy::is_identifier_start;
use super::editor_features::code_before_comment;
use super::open_documents::{live_document_for_path, uri_to_path, OpenDocument};
use super::source_symbols::module_name_for_path;
use super::text_positions::{lsp_range_for_text, query_position_from_lsp, word_at_position};
use super::Backend;
use sage_index::{is_code_reference_at_range, parse_source, QueryDefinition, QueryResult};
use std::collections::{BTreeSet, HashMap, VecDeque};
use std::path::Path;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::request::{
    GotoDeclarationParams, GotoDeclarationResponse, GotoImplementationParams,
    GotoImplementationResponse, GotoTypeDefinitionParams, GotoTypeDefinitionResponse,
};
use tower_lsp::lsp_types::{
    ClientCapabilities, GotoDefinitionParams, GotoDefinitionResponse, Location, LocationLink,
    Position, Range, TextDocumentPositionParams, Url,
};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct NavigationLinkSupport {
    pub(super) declaration: bool,
    pub(super) definition: bool,
    pub(super) implementation: bool,
}

impl NavigationLinkSupport {
    pub(super) fn from_client_capabilities(capabilities: &ClientCapabilities) -> Self {
        let Some(text_document) = capabilities.text_document.as_ref() else {
            return Self::default();
        };
        Self {
            declaration: text_document
                .declaration
                .as_ref()
                .and_then(|capability| capability.link_support)
                .unwrap_or(false),
            definition: text_document
                .definition
                .as_ref()
                .and_then(|capability| capability.link_support)
                .unwrap_or(false),
            implementation: text_document
                .implementation
                .as_ref()
                .and_then(|capability| capability.link_support)
                .unwrap_or(false),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum NavigationRequestKind {
    Declaration,
    Definition,
    Implementation,
}

impl NavigationRequestKind {
    pub(super) fn link_support(self, support: NavigationLinkSupport) -> bool {
        match self {
            Self::Declaration => support.declaration,
            Self::Definition => support.definition,
            Self::Implementation => support.implementation,
        }
    }

    pub(super) fn should_defer_python_import(
        self,
        path: &Path,
        text: &str,
        position: Position,
        definition: &QueryDefinition,
    ) -> bool {
        self == Self::Implementation
            && should_defer_python_import_definition_to_python_provider(
                path, text, position, definition,
            )
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(super) struct NavigationQueryCacheKey {
    pub(super) uri: String,
    pub(super) version: i32,
    pub(super) content_fingerprint: Option<u64>,
    pub(super) line: u32,
    pub(super) character: u32,
    pub(super) index_generation: u64,
}

#[derive(Debug, Default)]
pub(super) struct NavigationQueryCache {
    entries: HashMap<NavigationQueryCacheKey, QueryResult>,
    order: VecDeque<NavigationQueryCacheKey>,
}

impl NavigationQueryCache {
    pub(super) fn get(&self, key: &NavigationQueryCacheKey) -> Option<QueryResult> {
        self.entries.get(key).cloned()
    }

    pub(super) fn insert(&mut self, key: NavigationQueryCacheKey, query: QueryResult) {
        const CAPACITY: usize = 128;
        if !self.entries.contains_key(&key) {
            self.order.push_back(key.clone());
        }
        self.entries.insert(key, query);
        while self.entries.len() > CAPACITY {
            if let Some(oldest) = self.order.pop_front() {
                self.entries.remove(&oldest);
            }
        }
    }

    pub(super) fn invalidate_uri(&mut self, uri: &Url) {
        let uri = uri.to_string();
        self.entries.retain(|key, _| key.uri != uri);
        self.order.retain(|key| key.uri != uri);
    }

    pub(super) fn clear(&mut self) {
        self.entries.clear();
        self.order.clear();
    }
}

pub(super) fn navigation_query_cache_key(
    uri: &Url,
    document: &OpenDocument,
    position: Position,
    index_generation: u64,
) -> NavigationQueryCacheKey {
    NavigationQueryCacheKey {
        uri: uri.to_string(),
        version: document.version,
        content_fingerprint: document.content_fingerprint,
        line: position.line,
        character: position.character,
        index_generation,
    }
}

impl Backend {
    pub(super) async fn goto_definition_response(
        &self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        self.goto_navigation_response(
            params.text_document_position_params,
            NavigationRequestKind::Definition,
        )
        .await
    }

    pub(super) async fn goto_declaration_response(
        &self,
        params: GotoDeclarationParams,
    ) -> Result<Option<GotoDeclarationResponse>> {
        self.goto_navigation_response(
            params.text_document_position_params,
            NavigationRequestKind::Declaration,
        )
        .await
    }

    pub(super) async fn goto_type_definition_response(
        &self,
        params: GotoTypeDefinitionParams,
    ) -> Result<Option<GotoTypeDefinitionResponse>> {
        let uri = &params.text_document_position_params.text_document.uri;
        let Some(document) = self.document_for_uri(uri).await else {
            return Ok(None);
        };
        let Some(path) = uri_to_path(uri) else {
            return Ok(None);
        };
        let Some(query_position) = query_position_from_lsp(
            &document.text,
            params.text_document_position_params.position,
        ) else {
            return Ok(None);
        };
        let index = self.index.read().await;
        let Some(definition) =
            index.type_definition_at_source(&path, &document.text, query_position)
        else {
            return Ok(None);
        };
        drop(index);
        let Some(location) = self.location_for_query_definition(&definition).await else {
            return Ok(None);
        };
        Ok(Some(GotoTypeDefinitionResponse::Scalar(location)))
    }

    pub(super) async fn goto_implementation_response(
        &self,
        params: GotoImplementationParams,
    ) -> Result<Option<GotoImplementationResponse>> {
        self.goto_navigation_response(
            params.text_document_position_params,
            NavigationRequestKind::Implementation,
        )
        .await
    }

    async fn goto_navigation_response(
        &self,
        params: TextDocumentPositionParams,
        request_kind: NavigationRequestKind,
    ) -> Result<Option<GotoDefinitionResponse>> {
        let uri = &params.text_document.uri;
        let Some(document) = self.document_for_uri(uri).await else {
            return Ok(None);
        };
        let Some(path) = uri_to_path(uri) else {
            return Ok(None);
        };
        let query = self
            .navigation_query_for_document(uri, &document, &path, params.position)
            .await;
        if query.resolution_confidence.as_deref() == Some("high") {
            let Some(definition) = query.definition.as_ref() else {
                return Ok(None);
            };
            if request_kind.should_defer_python_import(
                &path,
                &document.text,
                params.position,
                definition,
            ) {
                return Ok(None);
            }
            let Some(location) = self.location_for_query_definition(definition).await else {
                return Ok(None);
            };
            return Ok(Some(GotoDefinitionResponse::Scalar(location)));
        }
        let origin_range = query
            .target
            .as_ref()
            .map(|target| lsp_range_for_text(&document.text, &target.range));
        let links = self
            .links_for_navigation_candidates(&query, origin_range)
            .await;
        if links.len() < 2 {
            return Ok(None);
        }
        let link_support = request_kind.link_support(*self.navigation_link_support.read().await);
        Ok(Some(navigation_response_for_links(links, link_support)))
    }

    pub(super) async fn location_for_query_definition(
        &self,
        definition: &QueryDefinition,
    ) -> Option<Location> {
        if let Some((uri, document)) = self.open_document_for_path(&definition.path).await {
            let range = live_definition_range(definition, &document.text)?;
            return Some(Location {
                uri,
                range: lsp_range_for_text(&document.text, &range),
            });
        }
        validated_disk_definition_location(definition)
    }

    pub(super) async fn open_document_for_path(&self, path: &Path) -> Option<(Url, OpenDocument)> {
        let documents = self.open_documents.read().await;
        let live = live_document_for_path(&documents, path)?;
        Some((live.uri, live.document))
    }

    pub(super) async fn navigation_query_for_document(
        &self,
        uri: &Url,
        document: &OpenDocument,
        path: &Path,
        position: Position,
    ) -> QueryResult {
        let Some(query_position) = query_position_from_lsp(&document.text, position) else {
            return QueryResult {
                fallback_reason: Some("invalid-lsp-position".to_string()),
                ..QueryResult::default()
            };
        };
        let index = self.index.read().await;
        let index_generation = index.status().generation;
        let key = navigation_query_cache_key(uri, document, position, index_generation);
        if let Some(query) = self.navigation_cache.read().await.get(&key) {
            drop(index);
            return query;
        }
        let query = index.query_source_at_navigation(path, &document.text, query_position);
        self.navigation_cache.write().await.insert(
            navigation_query_cache_key(uri, document, position, index_generation),
            query.clone(),
        );
        drop(index);
        query
    }

    async fn links_for_navigation_candidates(
        &self,
        query: &QueryResult,
        origin_selection_range: Option<Range>,
    ) -> Vec<LocationLink> {
        let mut links = Vec::new();
        let mut seen = BTreeSet::new();
        for candidate in &query.definition_candidates {
            let Some(location) = self
                .location_for_query_definition(&candidate.definition)
                .await
            else {
                continue;
            };
            if seen.insert(location_reference_key(&location.uri, &location.range)) {
                links.push(LocationLink {
                    origin_selection_range,
                    target_uri: location.uri,
                    target_range: location.range,
                    target_selection_range: location.range,
                });
            }
        }
        links
    }
}

pub(super) fn validated_disk_definition_location(definition: &QueryDefinition) -> Option<Location> {
    if definition.detail.is_empty() {
        return None;
    }
    let uri = Url::from_file_path(&definition.path).ok()?;
    let text = std::fs::read_to_string(&definition.path).ok()?;
    let range = live_definition_range(definition, &text)?;
    Some(Location {
        uri,
        range: lsp_range_for_text(&text, &range),
    })
}

pub(super) fn live_definition_range(
    definition: &QueryDefinition,
    text: &str,
) -> Option<sage_index::SourceRange> {
    let module = if definition.module.is_empty() {
        module_name_for_path(&definition.path)
    } else {
        definition.module.as_str()
    };
    let matches = parse_source(module, &definition.path, text)
        .symbols
        .into_iter()
        .filter(|symbol| symbol.name == definition.name)
        .filter(|symbol| definition.detail.is_empty() || symbol.detail == definition.detail)
        .collect::<Vec<_>>();
    if let Some(symbol) = matches
        .iter()
        .find(|symbol| symbol.range == definition.range)
    {
        return Some(symbol.range.clone());
    }
    match matches.as_slice() {
        [symbol] => Some(symbol.range.clone()),
        [] => live_parameter_definition_range(definition, text),
        _ => None,
    }
}

fn live_parameter_definition_range(
    definition: &QueryDefinition,
    text: &str,
) -> Option<sage_index::SourceRange> {
    if !definition.detail.starts_with("Local parameter ")
        || definition.range.start_line != definition.range.end_line
        || !is_code_reference_at_range(text, &definition.name, &definition.range)
    {
        return None;
    }
    Some(definition.range.clone())
}

pub(super) fn should_defer_python_import_definition_to_python_provider(
    path: &Path,
    text: &str,
    position: Position,
    definition: &QueryDefinition,
) -> bool {
    if path.extension().and_then(|extension| extension.to_str()) != Some("py") {
        return false;
    }
    if !definition.module.starts_with("sage.") {
        return false;
    }
    let Some((word, _)) = word_at_position(text, position) else {
        return false;
    };
    if word != definition.name {
        return false;
    }
    is_sage_from_import_item_position(text, position.line, &word)
}

fn is_sage_from_import_item_position(text: &str, line_number: u32, word: &str) -> bool {
    let lines = text.lines().collect::<Vec<_>>();
    let Some(line) = lines.get(line_number as usize) else {
        return false;
    };
    let trimmed = code_before_comment(line).trim();
    if !line_looks_like_import_item(trimmed, word) {
        return false;
    }
    if let Some(module) = sage_from_import_module(trimmed) {
        return module.starts_with("sage.");
    }

    let mut current = line_number as usize;
    let mut scanned = 0usize;
    while current > 0 && scanned < 40 {
        current -= 1;
        scanned += 1;
        let previous = code_before_comment(lines[current]).trim();
        if previous.is_empty() {
            return false;
        }
        if let Some(module) = sage_from_import_module(previous) {
            return module.starts_with("sage.");
        }
        if !line_looks_like_import_continuation(previous) {
            return false;
        }
    }
    false
}

fn sage_from_import_module(line: &str) -> Option<&str> {
    let rest = line.strip_prefix("from ")?;
    let import_index = rest.find(" import")?;
    Some(rest[..import_index].trim())
}

fn line_looks_like_import_item(line: &str, word: &str) -> bool {
    if let Some(module) = sage_from_import_module(line) {
        return module.starts_with("sage.") && line[5 + module.len()..].contains(word);
    }
    let item = line.trim_end_matches(',').trim();
    item == word
        || item
            .strip_prefix(word)
            .is_some_and(|rest| rest.trim_start().starts_with("as "))
}

fn line_looks_like_import_continuation(line: &str) -> bool {
    let item = line.trim_end_matches(',').trim();
    item == "(" || item == ")" || item.bytes().next().is_some_and(is_identifier_start)
}

pub(super) fn location_reference_key(uri: &Url, range: &Range) -> String {
    format!(
        "{}:{}:{}:{}:{}",
        uri, range.start.line, range.start.character, range.end.line, range.end.character
    )
}

pub(super) fn navigation_response_for_links(
    links: Vec<LocationLink>,
    link_support: bool,
) -> GotoDefinitionResponse {
    if link_support {
        return GotoDefinitionResponse::Link(links);
    }
    GotoDefinitionResponse::Array(
        links
            .into_iter()
            .map(|link| Location {
                uri: link.target_uri,
                range: link.target_selection_range,
            })
            .collect(),
    )
}
