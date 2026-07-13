#![allow(deprecated)]

mod call_hierarchy;
mod document_links;
mod editor_features;
mod index_jobs;
mod linked_document_prewarm;
mod open_documents;
mod runtime_docs;
mod signature_help;
mod source_symbols;
mod text_positions;

use call_hierarchy::{
    call_hierarchy_item_for_live_index_symbol, call_hierarchy_item_for_local_definition,
    call_hierarchy_item_for_local_symbol_at_position, call_hierarchy_item_for_symbol_with_folds,
    call_hierarchy_item_from_definition, call_hierarchy_item_from_symbol_record,
    call_ranges_in_range, enclosing_call_hierarchy_item,
    enclosing_call_hierarchy_item_from_context, is_identifier_start, push_incoming_call,
    push_outgoing_call, CallHierarchySourceContext,
};
use document_links::sage_document_links;
#[cfg(test)]
use editor_features::sage_selection_range;
use editor_features::{
    code_before_comment, sage_folding_ranges, sage_inlay_hints, sage_selection_ranges,
};
#[cfg(test)]
use index_jobs::index_job_result_is_current;
#[cfg(test)]
use linked_document_prewarm::import_modules_for_prewarm;
use linked_document_prewarm::LinkedDocumentPrewarmer;
#[cfg(test)]
use open_documents::source_text_fingerprint;
use open_documents::{
    canonical_path_for_comparison, live_document_for_path, live_document_for_uri_or_path,
    physical_paths as open_document_physical_paths, unique_live_documents, uri_to_path,
    OpenDocument, OpenDocumentMap,
};
use runtime_docs::{RuntimeDocsConfig, RuntimeDocsWorker};
use sage_index::{
    default_cache_dir, function_call_at_position, parse_source, semantic_spans,
    DocumentationRecord, IndexOptions, QueryCompletion, QueryDefinition, QueryFeatures,
    QueryPosition, QueryResult, SymbolKind as SageSymbolKind, SymbolRecord, WorkspaceIndex,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use signature_help::signature_information;
#[cfg(test)]
use signature_help::signature_parameter_offsets;
use source_symbols::{document_symbols_for_source, is_call_hierarchy_symbol, module_name_for_path};
use std::collections::{BTreeSet, HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;
use text_positions::{
    apply_text_document_change, byte_offset_to_utf16_character, is_word_byte, line_byte_bounds,
    lsp_position_for_byte_column, lsp_range_for_path, lsp_range_for_text, query_position_from_lsp,
    utf16_character_to_byte_offset, word_at_position,
};
use tokio::sync::{Mutex, RwLock};
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::request::{
    GotoDeclarationParams, GotoDeclarationResponse, GotoImplementationParams,
    GotoImplementationResponse, GotoTypeDefinitionParams, GotoTypeDefinitionResponse,
};
use tower_lsp::lsp_types::*;
use tower_lsp::{async_trait, Client, LanguageServer, LspService, Server};

const TOKEN_TYPES: &[SemanticTokenType] = &[
    SemanticTokenType::NAMESPACE,
    SemanticTokenType::TYPE,
    SemanticTokenType::CLASS,
    SemanticTokenType::FUNCTION,
    SemanticTokenType::METHOD,
    SemanticTokenType::VARIABLE,
    SemanticTokenType::PARAMETER,
    SemanticTokenType::KEYWORD,
    SemanticTokenType::DECORATOR,
];
const TOKEN_MODIFIERS: &[SemanticTokenModifier] = &[
    SemanticTokenModifier::DECLARATION,
    SemanticTokenModifier::READONLY,
    SemanticTokenModifier::DEFAULT_LIBRARY,
];
const COMMAND_INDEX_STATUS: &str = "sage.__rust.indexStatus";
const COMMAND_DOCS_STATUS: &str = "sage.__rust.docsStatus";
const COMMAND_REBUILD_INDEX: &str = "sage.__rust.rebuildIndex";
const COMMAND_GET_DOCUMENTATION: &str = "sage.__rust.getDocumentation";
const COMMAND_QUERY_AT_POSITION: &str = "sage.__rust.queryAtPosition";

fn trace_initialize_phase(started: Instant, phase: &str) {
    if std::env::var_os("SAGE_LS_TRACE_INITIALIZE").is_some() {
        eprintln!(
            "[sage-ls] initialize phase={phase} elapsed_ms={}",
            started.elapsed().as_millis()
        );
    }
}

fn sage_document_symbol_options() -> DocumentSymbolOptions {
    DocumentSymbolOptions {
        label: Some("Sage".to_string()),
        work_done_progress_options: WorkDoneProgressOptions::default(),
    }
}

async fn refresh_editor_feature_caches(client: &Client) {
    let _ = client.semantic_tokens_refresh().await;
    let _ = client.inlay_hint_refresh().await;
    let _ = client.workspace_diagnostic_refresh().await;
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(default)]
struct InitializationOptions {
    interpreter: InterpreterOptions,
    analysis: AnalysisOptions,
    workspace: WorkspaceOptions,
    documentation: DocumentationOptions,
    rust: RustOptions,
    pyright: PyrightOptions,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(default)]
struct InterpreterOptions {
    path: String,
    args: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(default)]
struct AnalysisOptions {
    extra_paths: Vec<String>,
    source_roots: Vec<String>,
    enable_diagnostics: bool,
    enable_runtime_introspection: bool,
    enable_pyx_parsing: bool,
}

impl Default for AnalysisOptions {
    fn default() -> Self {
        Self {
            extra_paths: Vec::new(),
            source_roots: Vec::new(),
            enable_diagnostics: true,
            enable_runtime_introspection: true,
            enable_pyx_parsing: true,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(default)]
struct WorkspaceOptions {
    folders: Vec<String>,
    source_roots: Vec<String>,
    exclude: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(default)]
struct DocumentationOptions {
    preferred_source: String,
    show_on_hover: bool,
}

impl Default for DocumentationOptions {
    fn default() -> Self {
        Self {
            preferred_source: "auto".to_string(),
            show_on_hover: true,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DocumentationPreferredSource {
    Auto,
    Workspace,
    Runtime,
    Reference,
}

impl DocumentationPreferredSource {
    fn from_config(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "workspace" => Self::Workspace,
            "runtime" => Self::Runtime,
            "reference" => Self::Reference,
            _ => Self::Auto,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Workspace => "workspace",
            Self::Runtime => "runtime",
            Self::Reference => "reference",
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(default)]
struct RustOptions {
    cache_dir: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(default)]
struct PyrightOptions {
    node_path: Option<String>,
    server_path: Option<String>,
}

struct Backend {
    client: Client,
    index: Arc<RwLock<WorkspaceIndex>>,
    open_documents: Arc<RwLock<OpenDocumentMap>>,
    navigation_cache: Arc<RwLock<NavigationQueryCache>>,
    diagnostics_enabled: Arc<RwLock<bool>>,
    docs_on_hover_enabled: Arc<RwLock<bool>>,
    docs_preferred_source: Arc<RwLock<DocumentationPreferredSource>>,
    pending_jobs: Arc<RwLock<usize>>,
    pending_index_task: Arc<RwLock<Option<String>>>,
    index_job_generation: Arc<AtomicU64>,
    index_work_gate: Arc<Mutex<()>>,
    shutting_down: Arc<AtomicBool>,
    linked_document_prewarmer: LinkedDocumentPrewarmer,
    runtime_docs: RuntimeDocsWorker,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct NavigationQueryCacheKey {
    uri: String,
    version: i32,
    content_fingerprint: Option<u64>,
    line: u32,
    character: u32,
    index_generation: u64,
}

#[derive(Debug, Default)]
struct NavigationQueryCache {
    entries: HashMap<NavigationQueryCacheKey, QueryResult>,
    order: VecDeque<NavigationQueryCacheKey>,
}

impl NavigationQueryCache {
    fn get(&self, key: &NavigationQueryCacheKey) -> Option<QueryResult> {
        self.entries.get(key).cloned()
    }

    fn insert(&mut self, key: NavigationQueryCacheKey, query: QueryResult) {
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

    fn invalidate_uri(&mut self, uri: &Url) {
        let uri = uri.to_string();
        self.entries.retain(|key, _| key.uri != uri);
        self.order.retain(|key| key.uri != uri);
    }

    fn clear(&mut self) {
        self.entries.clear();
        self.order.clear();
    }
}

fn navigation_query_cache_key(
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

#[derive(Clone, Debug)]
struct RenameTarget {
    word: String,
    range: Range,
    definition: QueryDefinition,
    definition_ranges: Vec<sage_index::SourceRange>,
    declaration: Location,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ReferenceCollectionMode {
    References,
    Rename,
}

#[derive(Clone, Debug)]
struct ResolvedReferenceTarget {
    word: String,
    range: Range,
    definition: QueryDefinition,
    definition_ranges: Vec<sage_index::SourceRange>,
    declaration: Option<Location>,
}

#[derive(Clone, Debug)]
struct DocumentationPositionContext {
    path: PathBuf,
    text: String,
    position: QueryPosition,
}

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    let (service, socket) = LspService::new(|client| Backend {
        client,
        index: Arc::new(RwLock::new(WorkspaceIndex::default())),
        open_documents: Arc::new(RwLock::new(OpenDocumentMap::new())),
        navigation_cache: Arc::new(RwLock::new(NavigationQueryCache::default())),
        diagnostics_enabled: Arc::new(RwLock::new(true)),
        docs_on_hover_enabled: Arc::new(RwLock::new(true)),
        docs_preferred_source: Arc::new(RwLock::new(DocumentationPreferredSource::Auto)),
        pending_jobs: Arc::new(RwLock::new(0)),
        pending_index_task: Arc::new(RwLock::new(None)),
        index_job_generation: Arc::new(AtomicU64::new(0)),
        index_work_gate: Arc::new(Mutex::new(())),
        shutting_down: Arc::new(AtomicBool::new(false)),
        linked_document_prewarmer: LinkedDocumentPrewarmer::default(),
        runtime_docs: RuntimeDocsWorker::default(),
    });
    Server::new(stdin, stdout, socket).serve(service).await;
}

#[async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {
        let initialize_started = Instant::now();
        let options = parse_initialization_options(params.initialization_options);
        trace_initialize_phase(initialize_started, "parse-options");
        let roots = source_roots_from_options(&options);
        trace_initialize_phase(initialize_started, "source-roots");
        *self.diagnostics_enabled.write().await = options.analysis.enable_diagnostics;
        *self.docs_on_hover_enabled.write().await = options.documentation.show_on_hover;
        *self.docs_preferred_source.write().await =
            DocumentationPreferredSource::from_config(&options.documentation.preferred_source);
        trace_initialize_phase(initialize_started, "feature-flags");
        self.runtime_docs
            .configure(RuntimeDocsConfig {
                enabled: options.analysis.enable_runtime_introspection,
                interpreter_path: options.interpreter.path.clone(),
                interpreter_args: options.interpreter.args.clone(),
                source_roots: roots.clone(),
            })
            .await;
        trace_initialize_phase(initialize_started, "runtime-docs");
        let cache_dir = options
            .rust
            .cache_dir
            .as_ref()
            .map(PathBuf::from)
            .unwrap_or_else(default_cache_dir);
        trace_initialize_phase(initialize_started, "cache-dir");
        let editable_roots = workspace_folders_from_options(&options);
        trace_initialize_phase(initialize_started, "editable-roots");
        let index_options = IndexOptions {
            editable_roots: editable_roots.clone(),
            roots,
            exclude_globs: if options.workspace.exclude.is_empty() {
                default_excludes()
            } else {
                options.workspace.exclude.clone()
            },
            cache_dir,
            enable_pyx: options.analysis.enable_pyx_parsing,
        };
        let mut hydrated = WorkspaceIndex::new(index_options);
        trace_initialize_phase(initialize_started, "index-new");
        let _ = hydrated.hydrate_from_cache();
        trace_initialize_phase(initialize_started, "hydrate");
        *self.index.write().await = hydrated;
        self.spawn_cache_reconcile();
        trace_initialize_phase(initialize_started, "spawn-reconcile");

        Ok(InitializeResult {
            server_info: Some(ServerInfo {
                name: "sage-ls".to_string(),
                version: Some(env!("CARGO_PKG_VERSION").to_string()),
            }),
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Options(
                    TextDocumentSyncOptions {
                        open_close: Some(true),
                        change: Some(TextDocumentSyncKind::INCREMENTAL),
                        will_save: None,
                        will_save_wait_until: None,
                        save: Some(TextDocumentSyncSaveOptions::SaveOptions(SaveOptions {
                            include_text: Some(true),
                        })),
                    },
                )),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                declaration_provider: Some(DeclarationCapability::Simple(true)),
                definition_provider: Some(OneOf::Left(true)),
                type_definition_provider: Some(TypeDefinitionProviderCapability::Simple(true)),
                implementation_provider: Some(ImplementationProviderCapability::Simple(true)),
                completion_provider: Some(CompletionOptions {
                    trigger_characters: Some(vec![".".to_string(), "(".to_string()]),
                    resolve_provider: Some(true),
                    ..CompletionOptions::default()
                }),
                document_link_provider: Some(DocumentLinkOptions {
                    resolve_provider: Some(false),
                    work_done_progress_options: WorkDoneProgressOptions::default(),
                }),
                code_action_provider: Some(CodeActionProviderCapability::Options(
                    CodeActionOptions {
                        code_action_kinds: Some(vec![CodeActionKind::QUICKFIX]),
                        resolve_provider: Some(false),
                        ..CodeActionOptions::default()
                    },
                )),
                document_highlight_provider: Some(OneOf::Left(true)),
                references_provider: Some(OneOf::Left(true)),
                rename_provider: Some(OneOf::Right(RenameOptions {
                    prepare_provider: Some(true),
                    work_done_progress_options: WorkDoneProgressOptions::default(),
                })),
                signature_help_provider: Some(SignatureHelpOptions {
                    trigger_characters: Some(vec!["(".to_string(), ",".to_string()]),
                    ..SignatureHelpOptions::default()
                }),
                call_hierarchy_provider: Some(CallHierarchyServerCapability::Simple(true)),
                selection_range_provider: Some(SelectionRangeProviderCapability::Simple(true)),
                folding_range_provider: Some(FoldingRangeProviderCapability::Simple(true)),
                inlay_hint_provider: Some(OneOf::Left(true)),
                document_symbol_provider: Some(OneOf::Right(sage_document_symbol_options())),
                workspace_symbol_provider: Some(OneOf::Left(true)),
                semantic_tokens_provider: Some(
                    SemanticTokensServerCapabilities::SemanticTokensOptions(
                        SemanticTokensOptions {
                            legend: SemanticTokensLegend {
                                token_types: TOKEN_TYPES.to_vec(),
                                token_modifiers: TOKEN_MODIFIERS.to_vec(),
                            },
                            range: Some(true),
                            full: Some(SemanticTokensFullOptions::Bool(true)),
                            ..SemanticTokensOptions::default()
                        },
                    ),
                ),
                execute_command_provider: Some(ExecuteCommandOptions {
                    commands: vec![
                        COMMAND_INDEX_STATUS.to_string(),
                        COMMAND_DOCS_STATUS.to_string(),
                        COMMAND_REBUILD_INDEX.to_string(),
                        COMMAND_GET_DOCUMENTATION.to_string(),
                        COMMAND_QUERY_AT_POSITION.to_string(),
                    ],
                    ..ExecuteCommandOptions::default()
                }),
                ..ServerCapabilities::default()
            },
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        self.client
            .log_message(MessageType::INFO, "sage-ls v2 runtime initialized")
            .await;
    }

    async fn shutdown(&self) -> Result<()> {
        self.shutting_down.store(true, Ordering::Release);
        self.linked_document_prewarmer.cancel_all();
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let uri = params.text_document.uri.clone();
        let text = params.text_document.text.clone();
        self.open_documents.write().await.insert(
            params.text_document.uri.clone(),
            OpenDocument::live(
                &params.text_document.uri,
                params.text_document.text,
                params.text_document.version,
            ),
        );
        self.navigation_cache.write().await.invalidate_uri(&uri);
        self.publish_diagnostics_for_text(uri.clone(), text.clone())
            .await;
        self.schedule_linked_document_prewarm(uri, text, params.text_document.version);
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let uri = params.text_document.uri.clone();
        let mut errors = Vec::new();
        let text = {
            let mut documents = self.open_documents.write().await;
            let document = documents.entry(uri.clone()).or_insert_with(|| {
                OpenDocument::live(&uri, String::new(), params.text_document.version)
            });
            document.version = params.text_document.version;
            document.content_fingerprint = None;
            for change in params.content_changes {
                if let Err(error) = apply_text_document_change(&mut document.text, &change) {
                    errors.push(error);
                }
            }
            document.text.clone()
        };
        self.navigation_cache.write().await.invalidate_uri(&uri);
        self.schedule_linked_document_prewarm(
            uri.clone(),
            text.clone(),
            params.text_document.version,
        );
        for error in errors {
            self.client
                .log_message(
                    MessageType::WARNING,
                    format!("sage-ls ignored invalid incremental document change: {error}"),
                )
                .await;
        }
        self.publish_diagnostics_for_text(uri, text).await;
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        self.linked_document_prewarmer
            .cancel(&params.text_document.uri);
        self.open_documents
            .write()
            .await
            .remove(&params.text_document.uri);
        self.navigation_cache
            .write()
            .await
            .invalidate_uri(&params.text_document.uri);
    }

    async fn did_save(&self, params: DidSaveTextDocumentParams) {
        let mut saved_text = params.text.clone();
        if let Some(text) = params.text {
            if let Some(document) = self
                .open_documents
                .write()
                .await
                .get_mut(&params.text_document.uri)
            {
                document.text = text.clone();
            }
            saved_text = Some(text);
        }
        self.navigation_cache
            .write()
            .await
            .invalidate_uri(&params.text_document.uri);
        if let Some(text) = saved_text.as_ref() {
            let version = self
                .open_documents
                .read()
                .await
                .get(&params.text_document.uri)
                .map_or(i32::MIN, |document| document.version);
            self.schedule_linked_document_prewarm(
                params.text_document.uri.clone(),
                text.clone(),
                version,
            );
        }
        if let Some(path) = uri_to_path(&params.text_document.uri) {
            self.refresh_paths(vec![path], Vec::new()).await;
        }
        if let Some(text) = saved_text.or_else(|| {
            uri_to_path(&params.text_document.uri)
                .and_then(|path| std::fs::read_to_string(path).ok())
        }) {
            self.publish_diagnostics_for_text(params.text_document.uri, text)
                .await;
        }
    }

    async fn did_change_watched_files(&self, params: DidChangeWatchedFilesParams) {
        let mut changed = Vec::new();
        let mut deleted = Vec::new();
        for event in params.changes {
            let Some(path) = uri_to_path(&event.uri) else {
                continue;
            };
            match event.typ {
                FileChangeType::DELETED => deleted.push(path),
                FileChangeType::CREATED | FileChangeType::CHANGED => changed.push(path),
                _ => {}
            }
        }
        if !changed.is_empty() || !deleted.is_empty() {
            self.navigation_cache.write().await.clear();
            self.refresh_paths(changed, deleted).await;
        }
    }

    async fn code_action(&self, params: CodeActionParams) -> Result<Option<CodeActionResponse>> {
        let actions = code_actions_for_diagnostics(
            params.text_document.uri,
            params.context.diagnostics.as_slice(),
        );
        Ok((!actions.is_empty()).then_some(actions))
    }

    async fn document_link(&self, params: DocumentLinkParams) -> Result<Option<Vec<DocumentLink>>> {
        let Some(document) = self.document_for_uri(&params.text_document.uri).await else {
            return Ok(Some(Vec::new()));
        };
        let Some(path) = uri_to_path(&params.text_document.uri) else {
            return Ok(Some(Vec::new()));
        };
        Ok(Some(sage_document_links(&document.text, &path)))
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let uri = &params.text_document_position_params.text_document.uri;
        let Some(document) = self.document_for_uri(uri).await else {
            return Ok(None);
        };
        let Some(path) = uri_to_path(uri) else {
            return Ok(None);
        };
        let position = params.text_document_position_params.position;
        let Some(query_position) = query_position_from_lsp(&document.text, position) else {
            return Ok(None);
        };
        let index = self.index.read().await;
        let index_generation = index.status().generation;
        let key = navigation_query_cache_key(uri, &document, position, index_generation);
        let cached_query = self.navigation_cache.read().await.get(&key);
        let query = if let Some(query) = cached_query {
            drop(index);
            query
        } else {
            let query = index.query_source_at_with_features(
                &path,
                &document.text,
                query_position,
                None,
                QueryFeatures::hover(),
            );
            self.navigation_cache.write().await.insert(
                navigation_query_cache_key(uri, &document, position, index_generation),
                query.clone(),
            );
            drop(index);
            query
        };
        let show_docs_on_hover = *self.docs_on_hover_enabled.read().await;
        let docs_preferred_source = *self.docs_preferred_source.read().await;
        if show_docs_on_hover {
            let runtime_record = self
                .runtime_docs_for_query(&query, docs_preferred_source)
                .await;
            if let Some(record) = runtime_record {
                let range = query
                    .target
                    .as_ref()
                    .map(|target| target.range.clone())
                    .unwrap_or_default();
                return Ok(Some(Hover {
                    contents: HoverContents::Markup(MarkupContent {
                        kind: MarkupKind::Markdown,
                        value: hover_markdown_for_documentation(&record),
                    }),
                    range: Some(lsp_range_for_text(&document.text, &range)),
                }));
            }
        }
        let Some(hover) = query.hover else {
            let Some(target) = query.target else {
                return Ok(None);
            };
            return Ok(Some(Hover {
                contents: HoverContents::Scalar(MarkedString::String(format!(
                    "symbol {}",
                    target.symbol
                ))),
                range: None,
            }));
        };
        Ok(Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: hover_markdown_for_hover_setting(&hover.markdown, show_docs_on_hover),
            }),
            range: Some(lsp_range_for_text(&document.text, &hover.range)),
        }))
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        let uri = &params.text_document_position_params.text_document.uri;
        let Some(document) = self.document_for_uri(uri).await else {
            return Ok(None);
        };
        let Some(path) = uri_to_path(uri) else {
            return Ok(None);
        };
        let query = self
            .navigation_query_for_document(
                uri,
                &document,
                &path,
                params.text_document_position_params.position,
            )
            .await;
        let Some(definition) = query.definition else {
            return Ok(None);
        };
        let Some(location) = self.location_for_query_definition(&definition).await else {
            return Ok(None);
        };
        Ok(Some(GotoDefinitionResponse::Scalar(location)))
    }

    async fn goto_declaration(
        &self,
        params: GotoDeclarationParams,
    ) -> Result<Option<GotoDeclarationResponse>> {
        let uri = &params.text_document_position_params.text_document.uri;
        let Some(location) = self
            .definition_location_at(uri, params.text_document_position_params.position)
            .await
        else {
            return Ok(None);
        };
        Ok(Some(GotoDeclarationResponse::Scalar(location)))
    }

    async fn goto_type_definition(
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

    async fn goto_implementation(
        &self,
        params: GotoImplementationParams,
    ) -> Result<Option<GotoImplementationResponse>> {
        let uri = &params.text_document_position_params.text_document.uri;
        let Some(document) = self.document_for_uri(uri).await else {
            return Ok(None);
        };
        let Some(path) = uri_to_path(uri) else {
            return Ok(None);
        };
        let query = self
            .navigation_query_for_document(
                uri,
                &document,
                &path,
                params.text_document_position_params.position,
            )
            .await;
        let Some(definition) = query.definition else {
            return Ok(None);
        };
        if should_defer_python_import_definition_to_python_provider(
            &path,
            &document.text,
            params.text_document_position_params.position,
            &definition,
        ) {
            return Ok(None);
        }
        let Some(location) = self.location_for_query_definition(&definition).await else {
            return Ok(None);
        };
        Ok(Some(GotoImplementationResponse::Scalar(location)))
    }

    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        let uri = &params.text_document_position.text_document.uri;
        if let (Some(document), Some(_path)) = (self.document_for_uri(uri).await, uri_to_path(uri))
        {
            let Some(query_position) =
                query_position_from_lsp(&document.text, params.text_document_position.position)
            else {
                return Ok(Some(CompletionResponse::Array(Vec::new())));
            };
            let index = self.index.read().await;
            let items = index
                .completion_items_at_source(&document.text, query_position, 100)
                .into_iter()
                .map(query_completion_item)
                .collect();
            return Ok(Some(CompletionResponse::Array(items)));
        }
        let prefix = self
            .prefix_at_uri_position(
                &params.text_document_position.text_document.uri,
                params.text_document_position.position,
            )
            .await
            .unwrap_or_default();
        let index = self.index.read().await;
        let items = index
            .symbols_with_prefix(&prefix, 100)
            .into_iter()
            .map(completion_item)
            .collect();
        Ok(Some(CompletionResponse::Array(items)))
    }

    async fn completion_resolve(&self, mut item: CompletionItem) -> Result<CompletionItem> {
        if item.documentation.is_some() {
            return Ok(item);
        }
        let resolve_data = item
            .data
            .clone()
            .and_then(|value| serde_json::from_value::<CompletionResolveData>(value).ok());
        let name = resolve_data
            .as_ref()
            .map(|data| data.name.as_str())
            .unwrap_or(item.label.as_str());
        let module_hint = resolve_data
            .as_ref()
            .and_then(|data| data.module.as_deref())
            .filter(|module| !module.is_empty());
        let documentation = self
            .index
            .read()
            .await
            .documentation_for_symbol_with_module(name, module_hint);
        if let Some(documentation) = documentation {
            item.documentation = Some(completion_documentation(hover_markdown_for_documentation(
                &documentation,
            )));
        }
        Ok(item)
    }

    async fn references(&self, params: ReferenceParams) -> Result<Option<Vec<Location>>> {
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

    async fn document_highlight(
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
        let Some((word, range)) = word_at_position(
            &document.text,
            params.text_document_position_params.position,
        ) else {
            return Ok(Some(Vec::new()));
        };
        Ok(Some(document_highlights_for_source(
            &path,
            &document.text,
            &word,
            range,
        )))
    }

    async fn rename(&self, params: RenameParams) -> Result<Option<WorkspaceEdit>> {
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

    async fn prepare_rename(
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

    async fn signature_help(&self, params: SignatureHelpParams) -> Result<Option<SignatureHelp>> {
        let uri = &params.text_document_position_params.text_document.uri;
        let Some(document) = self.document_for_uri(uri).await else {
            return Ok(None);
        };
        let Some(path) = uri_to_path(uri) else {
            return Ok(None);
        };
        let fallback = query_position_from_lsp(
            &document.text,
            params.text_document_position_params.position,
        )
        .and_then(|position| {
            function_call_at_position(&document.text, position.line, position.character)
        });
        let query = self
            .navigation_query_for_document(
                uri,
                &document,
                &path,
                params.text_document_position_params.position,
            )
            .await;
        let Some(signature) = query.signature else {
            let Some((name, active_parameter)) = fallback else {
                return Ok(None);
            };
            let index = self.index.read().await;
            let Some(symbol) = index.resolve_symbol(&name, None) else {
                return Ok(None);
            };
            let Some(signature) = symbol.signature.clone() else {
                return Ok(None);
            };
            return Ok(Some(SignatureHelp {
                signatures: vec![signature_information(
                    signature,
                    symbol.docstring,
                    active_parameter,
                )],
                active_signature: Some(0),
                active_parameter: Some(active_parameter),
            }));
        };
        if signature.label.is_empty() {
            return Ok(None);
        }
        Ok(Some(SignatureHelp {
            signatures: vec![signature_information(
                signature.label,
                signature.documentation,
                signature.active_parameter,
            )],
            active_signature: Some(0),
            active_parameter: Some(signature.active_parameter),
        }))
    }

    async fn prepare_call_hierarchy(
        &self,
        params: CallHierarchyPrepareParams,
    ) -> Result<Option<Vec<CallHierarchyItem>>> {
        let uri = &params.text_document_position_params.text_document.uri;
        let Some(document) = self.document_for_uri(uri).await else {
            return Ok(Some(Vec::new()));
        };
        let Some(path) = uri_to_path(uri) else {
            return Ok(Some(Vec::new()));
        };
        let position = params.text_document_position_params.position;
        if let Some(item) =
            call_hierarchy_item_for_local_symbol_at_position(uri, &path, &document.text, position)
        {
            return Ok(Some(vec![item]));
        }
        let query = self
            .navigation_query_for_document(uri, &document, &path, position)
            .await;
        if let Some(definition) = query.definition {
            if canonical_path_for_comparison(&definition.path)
                == canonical_path_for_comparison(&path)
            {
                if let Some(item) = call_hierarchy_item_for_local_definition(
                    uri,
                    &path,
                    &document.text,
                    &definition,
                ) {
                    return Ok(Some(vec![item]));
                }
            }
            let Some(location) = self.location_for_query_definition(&definition).await else {
                return Ok(Some(Vec::new()));
            };
            return Ok(Some(vec![call_hierarchy_item_from_definition(
                &definition,
                location.uri,
                location.range,
            )]));
        }
        Ok(Some(
            enclosing_call_hierarchy_item(uri, &path, &document.text, position)
                .into_iter()
                .collect(),
        ))
    }

    async fn incoming_calls(
        &self,
        params: CallHierarchyIncomingCallsParams,
    ) -> Result<Option<Vec<CallHierarchyIncomingCall>>> {
        let mut calls = Vec::new();
        let mut contexts: HashMap<Url, CallHierarchySourceContext> = HashMap::new();
        let Some(target) = self
            .resolved_reference_target_at(&params.item.uri, params.item.selection_range.start)
            .await
        else {
            return Ok(Some(Vec::new()));
        };
        for location in self
            .reference_locations(&target, false, ReferenceCollectionMode::References)
            .await
        {
            if location.uri == params.item.uri && location.range == params.item.selection_range {
                continue;
            }
            let Some(path) = uri_to_path(&location.uri) else {
                continue;
            };
            let Some(text) = self.text_for_uri_or_file(&location.uri).await else {
                continue;
            };
            if !contexts.contains_key(&location.uri) {
                let parsed = parse_source(module_name_for_path(&path), &path, &text);
                let folds = sage_folding_ranges(&text);
                contexts.insert(
                    location.uri.clone(),
                    CallHierarchySourceContext {
                        text,
                        symbols: parsed.symbols,
                        folds,
                    },
                );
            }
            let Some(context) = contexts.get(&location.uri) else {
                continue;
            };
            let Some(from) = enclosing_call_hierarchy_item_from_context(
                &location.uri,
                context,
                location.range.start,
            ) else {
                continue;
            };
            if from.uri == params.item.uri && from.selection_range == params.item.selection_range {
                continue;
            }
            push_incoming_call(&mut calls, from, location.range);
        }
        Ok(Some(calls))
    }

    async fn outgoing_calls(
        &self,
        params: CallHierarchyOutgoingCallsParams,
    ) -> Result<Option<Vec<CallHierarchyOutgoingCall>>> {
        let Some(path) = uri_to_path(&params.item.uri) else {
            return Ok(Some(Vec::new()));
        };
        let Some(text) = self.text_for_uri_or_file(&params.item.uri).await else {
            return Ok(Some(Vec::new()));
        };
        let parsed = parse_source(module_name_for_path(&path), &path, &text);
        let folds = sage_folding_ranges(&text);
        let call_ranges = call_ranges_in_range(&text, params.item.range);
        let mut calls = Vec::new();
        for (name, from_range) in call_ranges {
            if name == params.item.name {
                continue;
            }
            let local = parsed
                .symbols
                .iter()
                .find(|symbol| symbol.name == name && is_call_hierarchy_symbol(symbol))
                .map(|symbol| {
                    call_hierarchy_item_for_symbol_with_folds(
                        &params.item.uri,
                        &text,
                        &folds,
                        symbol,
                    )
                });
            let to = if local.is_some() {
                local
            } else {
                let indexed = self.index.read().await.resolve_symbol(&name, None);
                match indexed {
                    Some(symbol) => self.call_hierarchy_item_for_index_symbol(&symbol).await,
                    None => None,
                }
            };
            let Some(to) = to else {
                continue;
            };
            push_outgoing_call(&mut calls, to, from_range);
        }
        Ok(Some(calls))
    }

    async fn inlay_hint(&self, params: InlayHintParams) -> Result<Option<Vec<InlayHint>>> {
        let Some(document) = self
            .open_documents
            .read()
            .await
            .get(&params.text_document.uri)
            .cloned()
        else {
            return Ok(Some(Vec::new()));
        };
        Ok(Some(sage_inlay_hints(&document.text, params.range)))
    }

    async fn folding_range(&self, params: FoldingRangeParams) -> Result<Option<Vec<FoldingRange>>> {
        let Some(document) = self
            .open_documents
            .read()
            .await
            .get(&params.text_document.uri)
            .cloned()
        else {
            return Ok(Some(Vec::new()));
        };
        Ok(Some(sage_folding_ranges(&document.text)))
    }

    async fn selection_range(
        &self,
        params: SelectionRangeParams,
    ) -> Result<Option<Vec<SelectionRange>>> {
        let Some(document) = self
            .open_documents
            .read()
            .await
            .get(&params.text_document.uri)
            .cloned()
        else {
            return Ok(Some(Vec::new()));
        };
        Ok(Some(sage_selection_ranges(
            &document.text,
            &params.positions,
        )))
    }

    async fn document_symbol(
        &self,
        params: DocumentSymbolParams,
    ) -> Result<Option<DocumentSymbolResponse>> {
        let Some(document) = self
            .open_documents
            .read()
            .await
            .get(&params.text_document.uri)
            .cloned()
        else {
            return Ok(Some(DocumentSymbolResponse::Flat(Vec::new())));
        };
        let path = uri_to_path(&params.text_document.uri)
            .unwrap_or_else(|| PathBuf::from("document.sage"));
        let parsed = parse_source(module_name_for_path(&path), &path, &document.text);
        Ok(Some(DocumentSymbolResponse::Nested(
            document_symbols_for_source(&document.text, &parsed.symbols),
        )))
    }

    async fn symbol(
        &self,
        params: WorkspaceSymbolParams,
    ) -> Result<Option<Vec<SymbolInformation>>> {
        Ok(Some(
            self.workspace_symbol_information(&params.query, 200).await,
        ))
    }

    async fn semantic_tokens_full(
        &self,
        params: SemanticTokensParams,
    ) -> Result<Option<SemanticTokensResult>> {
        let Some(document) = self
            .open_documents
            .read()
            .await
            .get(&params.text_document.uri)
            .cloned()
        else {
            return Ok(Some(SemanticTokensResult::Tokens(SemanticTokens {
                result_id: None,
                data: Vec::new(),
            })));
        };
        Ok(Some(SemanticTokensResult::Tokens(SemanticTokens {
            result_id: None,
            data: encode_semantic_tokens(&document.text),
        })))
    }

    async fn semantic_tokens_range(
        &self,
        params: SemanticTokensRangeParams,
    ) -> Result<Option<SemanticTokensRangeResult>> {
        let Some(document) = self
            .open_documents
            .read()
            .await
            .get(&params.text_document.uri)
            .cloned()
        else {
            return Ok(Some(SemanticTokensRangeResult::Tokens(SemanticTokens {
                result_id: None,
                data: Vec::new(),
            })));
        };
        Ok(Some(SemanticTokensRangeResult::Tokens(SemanticTokens {
            result_id: None,
            data: encode_semantic_tokens_for_range(&document.text, params.range),
        })))
    }

    async fn execute_command(&self, params: ExecuteCommandParams) -> Result<Option<Value>> {
        match params.command.as_str() {
            COMMAND_INDEX_STATUS => Ok(Some(self.index_status_payload().await)),
            COMMAND_DOCS_STATUS => Ok(Some(self.docs_status_payload().await)),
            COMMAND_REBUILD_INDEX => {
                self.spawn_rebuild();
                Ok(Some(self.index_status_payload().await))
            }
            COMMAND_GET_DOCUMENTATION => {
                let payload = params
                    .arguments
                    .first()
                    .cloned()
                    .unwrap_or_else(|| json!({}));
                Ok(self.documentation_payload(payload).await)
            }
            COMMAND_QUERY_AT_POSITION => {
                let payload = params
                    .arguments
                    .first()
                    .cloned()
                    .unwrap_or_else(|| json!({}));
                Ok(self.query_payload(payload).await)
            }
            _ => Ok(None),
        }
    }
}

fn query_features_from_payload(payload: &Value, diagnostics_enabled: bool) -> QueryFeatures {
    let mut features = match payload.get("mode").and_then(Value::as_str) {
        Some("hover") => QueryFeatures::hover(),
        Some("navigation") => QueryFeatures::navigation(),
        _ => QueryFeatures::full(),
    };
    if let Some(value) = query_feature_bool(payload, "completions") {
        features.completions = value;
    }
    if let Some(value) = query_feature_bool(payload, "references") {
        features.references = value;
    }
    if let Some(value) = query_feature_bool(payload, "renamePreview") {
        features.rename_preview = value;
    }
    if let Some(value) = query_feature_bool(payload, "rename_preview") {
        features.rename_preview = value;
    }
    if let Some(value) = query_feature_bool(payload, "signature") {
        features.signature = value;
    }
    if let Some(value) = query_feature_bool(payload, "diagnostics") {
        features.diagnostics = value;
    }
    if !diagnostics_enabled {
        features.diagnostics = false;
    }
    features
}

fn query_feature_bool(payload: &Value, key: &str) -> Option<bool> {
    payload
        .get("features")
        .and_then(|features| features.get(key))
        .and_then(Value::as_bool)
}

impl Backend {
    async fn publish_diagnostics_for_text(&self, uri: Url, text: String) {
        let Some(path) = uri_to_path(&uri) else {
            return;
        };
        let diagnostics = if *self.diagnostics_enabled.read().await {
            self.index
                .read()
                .await
                .diagnostics_for_source(&path, &text)
                .into_iter()
                .map(|diagnostic| Diagnostic {
                    range: lsp_range_for_text(&text, &diagnostic.range),
                    severity: Some(diagnostic_severity(&diagnostic.severity)),
                    code: Some(NumberOrString::String(diagnostic.code)),
                    source: Some("sage-ls".to_string()),
                    message: diagnostic.message,
                    ..Diagnostic::default()
                })
                .collect()
        } else {
            Vec::new()
        };
        self.client
            .publish_diagnostics(uri, diagnostics, None)
            .await;
    }

    async fn document_for_uri(&self, uri: &Url) -> Option<OpenDocument> {
        let documents = self.open_documents.read().await;
        if let Some(live) = live_document_for_uri_or_path(&documents, uri) {
            return Some(live.document);
        }
        drop(documents);
        let path = uri_to_path(uri)?;
        let text = std::fs::read_to_string(path).ok()?;
        Some(OpenDocument::on_disk(uri, text))
    }

    async fn location_for_query_definition(
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
        let uri = Url::from_file_path(&definition.path).ok()?;
        Some(Location {
            uri,
            range: lsp_range_for_path(&definition.path, &definition.range),
        })
    }

    async fn open_document_for_path(&self, path: &Path) -> Option<(Url, OpenDocument)> {
        let documents = self.open_documents.read().await;
        let live = live_document_for_path(&documents, path)?;
        Some((live.uri, live.document))
    }

    async fn call_hierarchy_item_for_index_symbol(
        &self,
        symbol: &SymbolRecord,
    ) -> Option<CallHierarchyItem> {
        let documents = self.open_documents.read().await;
        if let Some(live) = live_document_for_path(&documents, &symbol.path) {
            return call_hierarchy_item_for_live_index_symbol(
                &live.uri,
                &live.path,
                &live.document.text,
                symbol,
            );
        }
        call_hierarchy_item_from_symbol_record(symbol)
    }

    fn schedule_linked_document_prewarm(&self, uri: Url, text: String, version: i32) {
        self.linked_document_prewarmer.schedule(
            Arc::clone(&self.index),
            Arc::clone(&self.index_work_gate),
            Arc::clone(&self.navigation_cache),
            Arc::clone(&self.shutting_down),
            uri,
            text,
            version,
        );
    }

    async fn navigation_query_for_document(
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

    async fn text_for_uri_or_file(&self, uri: &Url) -> Option<String> {
        let documents = self.open_documents.read().await;
        if let Some(live) = live_document_for_uri_or_path(&documents, uri) {
            return Some(live.document.text);
        }
        drop(documents);
        let path = uri_to_path(uri)?;
        std::fs::read_to_string(path).ok()
    }

    async fn prefix_at_uri_position(&self, uri: &Url, position: Position) -> Option<String> {
        let document = self.document_for_uri(uri).await?;
        current_prefix(&document.text, position)
    }

    async fn rename_target(&self, uri: &Url, position: Position) -> Option<RenameTarget> {
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
        })
    }

    async fn resolved_reference_target_at(
        &self,
        uri: &Url,
        position: Position,
    ) -> Option<ResolvedReferenceTarget> {
        let document = self.document_for_uri(uri).await?;
        let path = uri_to_path(uri)?;
        let (word, range) = word_at_position(&document.text, position)?;
        if !is_valid_identifier(&word)
            || !is_code_reference_range(&path, &document.text, &word, range)
        {
            return None;
        }
        let query = self
            .navigation_query_for_document(uri, &document, &path, position)
            .await;
        let definition = query.definition?;
        let definition_ranges = self.definition_identity_ranges(&definition).await;
        let declaration = self.location_for_query_definition(&definition).await;
        Some(ResolvedReferenceTarget {
            word,
            range,
            definition,
            definition_ranges,
            declaration,
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

    async fn definition_location_at(&self, uri: &Url, position: Position) -> Option<Location> {
        let path = uri_to_path(uri)?;
        let query = if let Some(document) = self.document_for_uri(uri).await {
            self.navigation_query_for_document(uri, &document, &path, position)
                .await
        } else {
            let text = std::fs::read_to_string(&path).ok()?;
            let query_position = query_position_from_lsp(&text, position)?;
            let index = self.index.read().await;
            index.query_source_at_navigation(&path, &text, query_position)
        };
        let definition = query.definition.as_ref()?;
        self.location_for_query_definition(definition).await
    }

    async fn reference_locations(
        &self,
        target: &ResolvedReferenceTarget,
        include_declaration: bool,
        mode: ReferenceCollectionMode,
    ) -> Vec<Location> {
        let mut seen = BTreeSet::new();
        let mut locations = Vec::new();
        let mut source_text_by_path: HashMap<PathBuf, Option<String>> = HashMap::new();
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
        for reference in index.editable_references(&target.word) {
            if open_paths.contains(&canonical_path_for_comparison(&reference.path)) {
                continue;
            }
            let Some(text) = source_text_by_path
                .entry(reference.path.clone())
                .or_insert_with(|| std::fs::read_to_string(&reference.path).ok())
                .as_deref()
            else {
                continue;
            };
            if !reference_candidate_matches_target(
                &index,
                &reference.path,
                text,
                &reference.range,
                target,
            ) {
                continue;
            }
            if let Ok(uri) = Url::from_file_path(&reference.path) {
                push_scoped_reference_location(
                    &mut locations,
                    &mut seen,
                    Location {
                        uri,
                        range: lsp_range_for_text(text, &reference.range),
                    },
                    target.declaration.as_ref(),
                    include_declaration,
                );
            }
        }
        for live in unique_live_documents(&open_documents) {
            if !reference_path_is_collectible(&index, &live.path, mode) {
                continue;
            }
            for reference in
                sage_index::references_in_source(&live.path, &live.document.text, &target.word)
            {
                if !reference_candidate_matches_target(
                    &index,
                    &live.path,
                    &live.document.text,
                    &reference.range,
                    target,
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

    async fn documentation_payload(&self, payload: Value) -> Option<Value> {
        let explicit_symbol = payload
            .get("symbol")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|symbol| !symbol.is_empty())
            .map(str::to_string);
        let position_context = self.documentation_position_context(&payload).await;
        let symbol = explicit_symbol.clone().or_else(|| {
            position_context.as_ref().and_then(|context| {
                word_at_position(
                    &context.text,
                    lsp_position_for_byte_column(
                        &context.text,
                        context.position.line,
                        context.position.character,
                    ),
                )
                .map(|(word, _)| word)
            })
        })?;
        let preferred_source = *self.docs_preferred_source.read().await;
        let mut record = None;
        if explicit_symbol.is_none() {
            if let Some(context) = &position_context {
                record = {
                    let index = self.index.read().await;
                    documentation_record_for_source_position(
                        &index,
                        &context.path,
                        &context.text,
                        context.position,
                    )
                };
            }
        }
        if record.is_none() {
            record = self
                .documentation_record_for_symbol(&symbol, preferred_source)
                .await;
        }
        let record = record?;
        Some(json!({
            "name": record.name,
            "moduleName": record.module_name,
            "kind": record.kind,
            "detail": record.detail,
            "summary": record.summary,
            "docstring": record.docstring,
            "uri": record.uri,
            "markers": record.markers,
            "sections": record.sections,
        }))
    }

    async fn documentation_position_context(
        &self,
        payload: &Value,
    ) -> Option<DocumentationPositionContext> {
        let uri = payload.get("textDocument")?.get("uri")?.as_str()?;
        let line = payload.get("position")?.get("line")?.as_u64()? as u32;
        let character = payload.get("position")?.get("character")?.as_u64()? as u32;
        let uri = Url::parse(uri).ok()?;
        let path = uri_to_path(&uri)?;
        let text = self.text_for_uri_or_file(&uri).await?;
        let position = query_position_from_lsp(&text, Position::new(line, character))?;
        Some(DocumentationPositionContext {
            path,
            text,
            position,
        })
    }

    async fn query_payload(&self, payload: Value) -> Option<Value> {
        let uri = payload.get("textDocument")?.get("uri")?.as_str()?;
        let uri = Url::parse(uri).ok()?;
        let path = uri_to_path(&uri)?;
        let document = self.document_for_uri(&uri).await?;
        let rename_to = payload.get("renameTo").and_then(Value::as_str);
        let diagnostics_enabled = *self.diagnostics_enabled.read().await;
        let features = query_features_from_payload(&payload, diagnostics_enabled);
        let diagnostics = if features.diagnostics {
            let index = self.index.read().await;
            index.diagnostics_for_source(&path, &document.text)
        } else {
            Vec::new()
        };
        let mut query = if let Some(symbol) = payload.get("symbol").and_then(Value::as_str) {
            self.index.read().await.query_source_symbol_with_options(
                &path,
                &document.text,
                symbol,
                None,
                sage_index::QueryExecutionOptions {
                    rename_to,
                    diagnostics,
                    features,
                },
            )
        } else {
            let line = payload.get("position")?.get("line")?.as_u64()? as u32;
            let character = payload.get("position")?.get("character")?.as_u64()? as u32;
            let position = query_position_from_lsp(&document.text, Position::new(line, character))?;
            self.index.read().await.query_source_at_with_features(
                &path,
                &document.text,
                position,
                rename_to,
                features,
            )
        };
        self.enhance_query_with_runtime_docs(&mut query).await;
        serde_json::to_value(query).ok()
    }

    async fn docs_status_payload(&self) -> Value {
        let preferred_source = self.docs_preferred_source.read().await.as_str().to_string();
        let mut status = {
            let index = self.index.read().await;
            index.docs_status()
        };
        status.preferred_source = preferred_source;
        let status = self.runtime_docs.status(status).await;
        serde_json::to_value(status).unwrap_or_else(|_| json!({}))
    }

    async fn persist_runtime_documentation(&self, symbol: &str, record: &DocumentationRecord) {
        let index = self.index.read().await;
        let _ = index.write_runtime_documentation(symbol, record);
    }

    async fn documentation_record_for_symbol(
        &self,
        symbol: &str,
        preferred_source: DocumentationPreferredSource,
    ) -> Option<DocumentationRecord> {
        let static_record = self.index.read().await.documentation_for_symbol(symbol);
        match preferred_source {
            DocumentationPreferredSource::Runtime => {
                if let Some(runtime_record) = self.runtime_docs.lookup(symbol).await {
                    self.persist_runtime_documentation(symbol, &runtime_record)
                        .await;
                    Some(runtime_record)
                } else {
                    static_record
                }
            }
            DocumentationPreferredSource::Auto => match static_record {
                Some(record) if is_runtime_placeholder_documentation(&record) => {
                    match self.runtime_docs.lookup(symbol).await {
                        Some(runtime_record) => {
                            self.persist_runtime_documentation(symbol, &runtime_record)
                                .await;
                            Some(runtime_record)
                        }
                        None => Some(record),
                    }
                }
                Some(record) => Some(record),
                None => {
                    let runtime_record = self.runtime_docs.lookup(symbol).await?;
                    self.persist_runtime_documentation(symbol, &runtime_record)
                        .await;
                    Some(runtime_record)
                }
            },
            DocumentationPreferredSource::Workspace | DocumentationPreferredSource::Reference => {
                static_record
            }
        }
    }

    async fn runtime_docs_for_query(
        &self,
        query: &QueryResult,
        preferred_source: DocumentationPreferredSource,
    ) -> Option<DocumentationRecord> {
        let symbol = runtime_docs_symbol_for_query(query, preferred_source)?;
        let record = self.runtime_docs.cached(symbol);
        if record.is_none() {
            self.runtime_docs.prefetch(symbol);
        }
        record
    }

    async fn enhance_query_with_runtime_docs(&self, query: &mut QueryResult) {
        let preferred_source = *self.docs_preferred_source.read().await;
        let Some(record) = self.runtime_docs_for_query(query, preferred_source).await else {
            return;
        };
        let range = query
            .target
            .as_ref()
            .map(|target| target.range.clone())
            .unwrap_or_default();
        query.hover = Some(sage_index::QueryHover {
            markdown: hover_markdown_for_documentation(&record),
            range,
        });
        query.documentation = Some(record);
    }
}

fn documentation_record_for_source_position(
    index: &WorkspaceIndex,
    path: &Path,
    text: &str,
    position: QueryPosition,
) -> Option<DocumentationRecord> {
    if let Some(documentation) = index
        .query_source_at(path, text, position, None)
        .documentation
    {
        return Some(documentation);
    }
    let (word, _) = word_at_position(
        text,
        lsp_position_for_byte_column(text, position.line, position.character),
    )?;
    let symbols = parse_source(module_name_for_path(path), path, text).symbols;
    symbols
        .iter()
        .find(|symbol| {
            symbol.name == word && source_range_contains_position(&symbol.range, position)
        })
        .or_else(|| {
            symbols
                .iter()
                .find(|symbol| symbol.name == word && symbol.range.start_line == position.line)
        })
        .cloned()
        .map(documentation_record_from_symbol)
}

fn source_range_contains_position(
    range: &sage_index::SourceRange,
    position: QueryPosition,
) -> bool {
    let starts_before = range.start_line < position.line
        || (range.start_line == position.line && range.start_character <= position.character);
    let ends_after = range.end_line > position.line
        || (range.end_line == position.line && range.end_character >= position.character);
    starts_before && ends_after
}

fn documentation_record_from_symbol(symbol: SymbolRecord) -> DocumentationRecord {
    let summary = symbol
        .docstring
        .as_deref()
        .and_then(first_docstring_summary_line)
        .unwrap_or_else(|| symbol.detail.clone());
    DocumentationRecord {
        name: symbol.name,
        module_name: symbol.module,
        kind: format!("{:?}", symbol.kind),
        detail: symbol
            .signature
            .clone()
            .unwrap_or_else(|| symbol.detail.clone()),
        summary,
        docstring: symbol.docstring,
        uri: Url::from_file_path(&symbol.path)
            .ok()
            .map(|uri| uri.to_string()),
        markers: Vec::new(),
        sections: Vec::new(),
    }
}

fn first_docstring_summary_line(docstring: &str) -> Option<String> {
    docstring
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(str::to_string)
}

fn parse_initialization_options(value: Option<Value>) -> InitializationOptions {
    value
        .and_then(|raw| serde_json::from_value(raw).ok())
        .unwrap_or_default()
}

fn source_roots_from_options(options: &InitializationOptions) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    let workspace_folders = workspace_folders_from_options(options);
    roots.extend(
        options
            .workspace
            .source_roots
            .iter()
            .filter_map(|entry| uri_or_path(entry)),
    );
    roots.extend(workspace_folders.clone());
    roots.extend(resolve_configured_paths(
        &options.analysis.source_roots,
        &workspace_folders,
    ));
    roots.extend(resolve_configured_paths(
        &options.analysis.extra_paths,
        &workspace_folders,
    ));
    roots.sort();
    roots.dedup();
    roots
}

fn workspace_folders_from_options(options: &InitializationOptions) -> Vec<PathBuf> {
    let mut folders: Vec<_> = options
        .workspace
        .folders
        .iter()
        .filter_map(|entry| uri_or_path(entry))
        .collect();
    folders.sort();
    folders.dedup();
    folders
}

fn resolve_configured_paths(values: &[String], workspace_folders: &[PathBuf]) -> Vec<PathBuf> {
    values
        .iter()
        .flat_map(|value| {
            let path = PathBuf::from(value);
            if path.is_absolute() || workspace_folders.is_empty() {
                vec![path]
            } else {
                workspace_folders
                    .iter()
                    .map(|folder| folder.join(value))
                    .collect()
            }
        })
        .collect()
}

fn uri_or_path(value: &str) -> Option<PathBuf> {
    Url::parse(value)
        .ok()
        .and_then(|url| url.to_file_path().ok())
        .or_else(|| Some(PathBuf::from(value)))
}

fn default_excludes() -> Vec<String> {
    vec![
        "**/.git/**".to_string(),
        "**/__pycache__/**".to_string(),
        "**/.venv/**".to_string(),
        "**/build/**".to_string(),
        "**/target/**".to_string(),
    ]
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CompletionResolveData {
    name: String,
    module: Option<String>,
}

fn completion_item(symbol: SymbolRecord) -> CompletionItem {
    CompletionItem {
        label: symbol.name.clone(),
        kind: Some(completion_kind(&symbol.kind)),
        detail: symbol.signature.clone().or(Some(symbol.detail)),
        data: Some(json!(CompletionResolveData {
            name: symbol.name,
            module: Some(symbol.module),
        })),
        ..CompletionItem::default()
    }
}

fn query_completion_item(completion: QueryCompletion) -> CompletionItem {
    let QueryCompletion {
        label,
        kind,
        detail,
        signature,
        documentation,
        resolve_name,
        module,
    } = completion;
    let inline_documentation = if module.as_deref() == Some("document") {
        documentation.map(completion_documentation)
    } else {
        None
    };
    CompletionItem {
        label: label.clone(),
        kind: Some(query_completion_kind(&kind)),
        detail: signature.or(Some(detail)),
        documentation: inline_documentation,
        data: Some(json!(CompletionResolveData {
            name: resolve_name.unwrap_or(label),
            module,
        })),
        ..CompletionItem::default()
    }
}

fn completion_documentation(value: String) -> Documentation {
    Documentation::MarkupContent(MarkupContent {
        kind: MarkupKind::Markdown,
        value,
    })
}

fn code_actions_for_diagnostics(uri: Url, diagnostics: &[Diagnostic]) -> CodeActionResponse {
    let mut actions = Vec::new();
    for diagnostic in diagnostics {
        if is_python_sage_exponent_diagnostic(diagnostic) {
            actions.push(CodeActionOrCommand::CodeAction(CodeAction {
                title: "Replace Sage-style ^ with Python exponent **".to_string(),
                kind: Some(CodeActionKind::QUICKFIX),
                diagnostics: Some(vec![diagnostic.clone()]),
                edit: Some(workspace_edit_for_text_edits(
                    uri.clone(),
                    vec![TextEdit::new(diagnostic.range, "**".to_string())],
                )),
                ..CodeAction::default()
            }));
            continue;
        }
        if !is_incomplete_sage_exponent_diagnostic(diagnostic) {
            continue;
        }
        actions.push(CodeActionOrCommand::CodeAction(CodeAction {
            title: "Remove incomplete Sage exponent operator".to_string(),
            kind: Some(CodeActionKind::QUICKFIX),
            diagnostics: Some(vec![diagnostic.clone()]),
            edit: Some(workspace_edit_for_text_edits(
                uri.clone(),
                vec![TextEdit::new(diagnostic.range, String::new())],
            )),
            ..CodeAction::default()
        }));
        actions.push(CodeActionOrCommand::CodeAction(CodeAction {
            title: "Insert exponent placeholder".to_string(),
            kind: Some(CodeActionKind::QUICKFIX),
            diagnostics: Some(vec![diagnostic.clone()]),
            edit: Some(workspace_edit_for_text_edits(
                uri.clone(),
                vec![TextEdit::new(
                    Range::new(diagnostic.range.end, diagnostic.range.end),
                    "1".to_string(),
                )],
            )),
            ..CodeAction::default()
        }));
    }
    actions
}

fn document_highlights_for_source(
    path: &Path,
    text: &str,
    word: &str,
    target_range: Range,
) -> Vec<DocumentHighlight> {
    let references = sage_index::references_in_source(path, text, word);
    if !references
        .iter()
        .any(|reference| lsp_range_for_text(text, &reference.range) == target_range)
    {
        return Vec::new();
    }
    references
        .into_iter()
        .map(|reference| DocumentHighlight {
            range: lsp_range_for_text(text, &reference.range),
            kind: Some(DocumentHighlightKind::TEXT),
        })
        .collect()
}

fn is_code_reference_range(_path: &Path, text: &str, word: &str, target_range: Range) -> bool {
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

fn is_incomplete_sage_exponent_diagnostic(diagnostic: &Diagnostic) -> bool {
    matches!(
        diagnostic.code.as_ref(),
        Some(NumberOrString::String(code)) if code == "syntax-error"
    ) && diagnostic
        .message
        .contains("incomplete Sage exponentiation")
}

fn is_python_sage_exponent_diagnostic(diagnostic: &Diagnostic) -> bool {
    matches!(
        diagnostic.code.as_ref(),
        Some(NumberOrString::String(code)) if code == "sage-python-caret-exponent"
    )
}

fn diagnostic_severity(value: &str) -> DiagnosticSeverity {
    match value {
        "warning" => DiagnosticSeverity::WARNING,
        "information" => DiagnosticSeverity::INFORMATION,
        "hint" => DiagnosticSeverity::HINT,
        _ => DiagnosticSeverity::ERROR,
    }
}

fn workspace_edit_for_text_edits(uri: Url, edits: Vec<TextEdit>) -> WorkspaceEdit {
    let mut changes = HashMap::new();
    changes.insert(uri, edits);
    WorkspaceEdit {
        changes: Some(changes),
        ..WorkspaceEdit::default()
    }
}

fn is_runtime_placeholder_documentation(record: &DocumentationRecord) -> bool {
    record
        .docstring
        .as_deref()
        .is_some_and(|docstring| docstring.contains("Runtime documentation worker can provide"))
}

fn runtime_docs_symbol_for_query(
    query: &QueryResult,
    preferred_source: DocumentationPreferredSource,
) -> Option<&str> {
    match preferred_source {
        DocumentationPreferredSource::Workspace | DocumentationPreferredSource::Reference => None,
        DocumentationPreferredSource::Auto => {
            let documentation = query.documentation.as_ref()?;
            if !is_runtime_placeholder_documentation(documentation) {
                return None;
            }
            query
                .target
                .as_ref()
                .and_then(|target| target.dotted_symbol.as_deref())
                .or(Some(documentation.name.as_str()))
        }
        DocumentationPreferredSource::Runtime => query
            .target
            .as_ref()
            .and_then(|target| target.dotted_symbol.as_deref())
            .or_else(|| query.target.as_ref().map(|target| target.symbol.as_str()))
            .or_else(|| {
                query
                    .documentation
                    .as_ref()
                    .map(|record| record.name.as_str())
            }),
    }
}

fn hover_markdown_for_documentation(record: &DocumentationRecord) -> String {
    let mut lines = vec![
        "```sage".to_string(),
        if record.detail.is_empty() {
            record.name.clone()
        } else {
            record.detail.clone()
        },
        "```".to_string(),
        String::new(),
        format!("Module: `{}`", record.module_name),
    ];
    let body = record
        .docstring
        .as_deref()
        .filter(|docstring| !docstring.trim().is_empty())
        .unwrap_or(&record.summary);
    if !body.trim().is_empty() {
        lines.push(String::new());
        lines.push(compact_hover_docstring(body));
    }
    lines.join("\n")
}

fn hover_markdown_for_hover_setting(markdown: &str, show_docs_on_hover: bool) -> String {
    if show_docs_on_hover {
        return markdown.to_string();
    }
    hover_markdown_without_doc_preview(markdown)
}

fn hover_markdown_without_doc_preview(markdown: &str) -> String {
    let mut sections = markdown.split("\n\n");
    let signature = sections.next().unwrap_or_default().trim_end();
    let module = sections
        .find(|section| section.trim_start().starts_with("Module:"))
        .map(str::trim_end);

    match (signature.is_empty(), module) {
        (false, Some(module)) => format!("{signature}\n\n{module}"),
        (false, None) => signature.to_string(),
        (true, Some(module)) => module.to_string(),
        (true, None) => markdown.lines().take(3).collect::<Vec<_>>().join("\n"),
    }
}

fn compact_hover_docstring(docstring: &str) -> String {
    const MAX_LINES: usize = 24;
    const MAX_BYTES: usize = 2400;

    let trimmed = docstring.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    let mut output = String::new();
    let mut truncated = false;
    for (line_count, line) in trimmed.lines().enumerate() {
        if line_count >= MAX_LINES || output.len() + line.len() + 1 > MAX_BYTES {
            truncated = true;
            break;
        }
        if !output.is_empty() {
            output.push('\n');
        }
        output.push_str(line);
    }

    if truncated {
        output.push_str("\n\n... (open Sage documentation for the full docstring)");
    }
    output
}

fn completion_kind(kind: &SageSymbolKind) -> CompletionItemKind {
    match kind {
        SageSymbolKind::Class => CompletionItemKind::CLASS,
        SageSymbolKind::Function => CompletionItemKind::FUNCTION,
        SageSymbolKind::Module => CompletionItemKind::MODULE,
        SageSymbolKind::Variable | SageSymbolKind::PreparserGenerator => {
            CompletionItemKind::VARIABLE
        }
        SageSymbolKind::Import => CompletionItemKind::REFERENCE,
        SageSymbolKind::CythonDeclaration => CompletionItemKind::FUNCTION,
    }
}

fn query_completion_kind(kind: &str) -> CompletionItemKind {
    match kind {
        "Class" => CompletionItemKind::CLASS,
        "Module" => CompletionItemKind::MODULE,
        "Variable" | "PreparserGenerator" => CompletionItemKind::VARIABLE,
        "Import" => CompletionItemKind::REFERENCE,
        "Method" => CompletionItemKind::METHOD,
        "CythonDeclaration" | "Function" => CompletionItemKind::FUNCTION,
        _ => CompletionItemKind::METHOD,
    }
}

fn live_definition_range(
    definition: &QueryDefinition,
    text: &str,
) -> Option<sage_index::SourceRange> {
    parse_source(
        module_name_for_path(&definition.path),
        &definition.path,
        text,
    )
    .symbols
    .into_iter()
    .filter(|symbol| symbol.name == definition.name)
    .filter(|symbol| !matches!(symbol.kind, SageSymbolKind::Module | SageSymbolKind::Import))
    .filter(|symbol| definition.detail.is_empty() || symbol.detail == definition.detail)
    .min_by_key(|symbol| {
        symbol
            .range
            .start_line
            .abs_diff(definition.range.start_line)
    })
    .map(|symbol| symbol.range)
}

fn should_defer_python_import_definition_to_python_provider(
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

fn current_prefix(text: &str, position: Position) -> Option<String> {
    let line = text.lines().nth(position.line as usize)?;
    let character = utf16_character_to_byte_offset(line, position.character)?;
    let bytes = line.as_bytes();
    let mut start = character;
    while start > 0 && is_word_byte(bytes[start - 1]) {
        start -= 1;
    }
    Some(line[start..character].to_string())
}

fn is_valid_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first == '_' || first.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

fn location_reference_key(uri: &Url, range: &Range) -> String {
    format!(
        "{}:{}:{}:{}:{}",
        uri, range.start.line, range.start.character, range.end.line, range.end.character
    )
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

fn push_scoped_reference_location(
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

fn reference_candidate_matches_target(
    index: &WorkspaceIndex,
    path: &Path,
    text: &str,
    range: &sage_index::SourceRange,
    target: &ResolvedReferenceTarget,
) -> bool {
    let candidate = index
        .query_source_at_navigation(
            path,
            text,
            QueryPosition {
                line: range.start_line,
                character: range.start_character,
            },
        )
        .definition;
    candidate.is_some_and(|candidate| {
        same_definition_owner_identity(&target.definition, &candidate)
            && target.definition_ranges.contains(&candidate.range)
    })
}

fn reference_path_is_collectible(
    index: &WorkspaceIndex,
    path: &Path,
    mode: ReferenceCollectionMode,
) -> bool {
    mode == ReferenceCollectionMode::References
        || index.is_editable_path(&canonical_path_for_comparison(path))
}

#[cfg(test)]
fn same_definition_identity(left: &QueryDefinition, right: &QueryDefinition) -> bool {
    same_definition_owner_identity(left, right) && left.range == right.range
}

fn same_definition_owner_identity(left: &QueryDefinition, right: &QueryDefinition) -> bool {
    left.name == right.name
        && left.module == right.module
        && left.detail == right.detail
        && canonical_path_for_comparison(&left.path) == canonical_path_for_comparison(&right.path)
}

fn encode_semantic_tokens(text: &str) -> Vec<SemanticToken> {
    encode_semantic_spans(text, semantic_spans(text))
}

fn encode_semantic_tokens_for_range(text: &str, range: Range) -> Vec<SemanticToken> {
    encode_semantic_spans(
        text,
        semantic_spans(text)
            .into_iter()
            .filter(|span| semantic_span_intersects_range(text, span, &range)),
    )
}

fn encode_semantic_spans<I>(text: &str, spans: I) -> Vec<SemanticToken>
where
    I: IntoIterator<Item = sage_index::SemanticSpan>,
{
    let mut data = Vec::new();
    let mut previous_line = 0u32;
    let mut previous_start = 0u32;
    for span in spans {
        let Some((start, length)) = semantic_span_lsp_columns(text, &span) else {
            continue;
        };
        let delta_line = span.line.saturating_sub(previous_line);
        let delta_start = if delta_line == 0 {
            start.saturating_sub(previous_start)
        } else {
            start
        };
        data.push(SemanticToken {
            delta_line,
            delta_start,
            length,
            token_type: token_type_index(&span.token_type),
            token_modifiers_bitset: modifier_bitset(&span.modifiers),
        });
        previous_line = span.line;
        previous_start = start;
    }
    data
}

fn semantic_span_lsp_columns(text: &str, span: &sage_index::SemanticSpan) -> Option<(u32, u32)> {
    let (line_start, line_end) = line_byte_bounds(text, span.line)?;
    let line = &text[line_start..line_end];
    let start = byte_offset_to_utf16_character(line, span.start as usize)?;
    let end =
        byte_offset_to_utf16_character(line, span.start.saturating_add(span.length) as usize)?;
    Some((start, end.saturating_sub(start)))
}

fn semantic_span_intersects_range(
    text: &str,
    span: &sage_index::SemanticSpan,
    range: &Range,
) -> bool {
    if span.line < range.start.line || span.line > range.end.line {
        return false;
    }
    let Some((start, length)) = semantic_span_lsp_columns(text, span) else {
        return false;
    };
    let span_end = start.saturating_add(length);
    if span.line == range.start.line && span_end <= range.start.character {
        return false;
    }
    if span.line == range.end.line && start >= range.end.character {
        return false;
    }
    true
}

fn token_type_index(name: &str) -> u32 {
    match name {
        "namespace" => 0,
        "type" => 1,
        "class" => 2,
        "function" => 3,
        "method" => 4,
        "variable" => 5,
        "parameter" => 6,
        "keyword" => 7,
        "decorator" => 8,
        _ => 5,
    }
}

fn modifier_bitset(modifiers: &[String]) -> u32 {
    let mut bits = 0;
    for modifier in modifiers {
        match modifier.as_str() {
            "declaration" => bits |= 1 << 0,
            "readonly" => bits |= 1 << 1,
            "defaultLibrary" => bits |= 1 << 2,
            _ => {}
        }
    }
    bits
}

#[cfg(test)]
mod tests;
