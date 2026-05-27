#![allow(deprecated)]

mod runtime_docs;

use runtime_docs::{RuntimeDocsConfig, RuntimeDocsWorker};
use sage_index::{
    default_cache_dir, function_call_at_position, parse_file_for_roots, parse_source,
    semantic_spans, DocumentationRecord, IndexOptions, QueryCompletion, QueryDefinition,
    QueryFeatures, QueryPosition, QueryResult, SymbolKind as SageSymbolKind, SymbolRecord,
    WorkspaceIndex,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeSet, HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::Instant;
use tokio::sync::RwLock;
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
    open_documents: Arc<RwLock<HashMap<Url, OpenDocument>>>,
    navigation_cache: Arc<RwLock<NavigationQueryCache>>,
    editable_roots: Arc<RwLock<Vec<PathBuf>>>,
    diagnostics_enabled: Arc<RwLock<bool>>,
    docs_on_hover_enabled: Arc<RwLock<bool>>,
    docs_preferred_source: Arc<RwLock<DocumentationPreferredSource>>,
    pending_jobs: Arc<RwLock<usize>>,
    pending_index_task: Arc<RwLock<Option<String>>>,
    runtime_docs: RuntimeDocsWorker,
}

#[derive(Clone, Debug)]
struct OpenDocument {
    text: String,
    version: i32,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct NavigationQueryCacheKey {
    uri: String,
    version: i32,
    line: u32,
    character: u32,
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

#[derive(Clone, Debug)]
struct CallHierarchySourceContext {
    text: String,
    symbols: Vec<SymbolRecord>,
    folds: Vec<FoldingRange>,
}

#[derive(Clone, Debug)]
struct RenameTarget {
    word: String,
    range: Range,
    declaration: Option<Location>,
}

#[derive(Clone, Debug)]
struct WordPositionContext {
    word: String,
    range: Range,
    path: PathBuf,
    text: String,
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
        open_documents: Arc::new(RwLock::new(HashMap::new())),
        navigation_cache: Arc::new(RwLock::new(NavigationQueryCache::default())),
        editable_roots: Arc::new(RwLock::new(Vec::new())),
        diagnostics_enabled: Arc::new(RwLock::new(true)),
        docs_on_hover_enabled: Arc::new(RwLock::new(true)),
        docs_preferred_source: Arc::new(RwLock::new(DocumentationPreferredSource::Auto)),
        pending_jobs: Arc::new(RwLock::new(0)),
        pending_index_task: Arc::new(RwLock::new(None)),
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
        *self.editable_roots.write().await = editable_roots;
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
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let uri = params.text_document.uri.clone();
        let text = params.text_document.text.clone();
        self.open_documents.write().await.insert(
            params.text_document.uri,
            OpenDocument {
                text: params.text_document.text,
                version: params.text_document.version,
            },
        );
        self.navigation_cache.write().await.invalidate_uri(&uri);
        self.publish_diagnostics_for_text(uri.clone(), text.clone())
            .await;
        self.schedule_linked_document_prewarm(uri, text);
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let uri = params.text_document.uri.clone();
        let mut errors = Vec::new();
        let text = {
            let mut documents = self.open_documents.write().await;
            let document = documents
                .entry(uri.clone())
                .or_insert_with(|| OpenDocument {
                    text: String::new(),
                    version: params.text_document.version,
                });
            document.version = params.text_document.version;
            for change in params.content_changes {
                if let Err(error) = apply_text_document_change(&mut document.text, &change) {
                    errors.push(error);
                }
            }
            document.text.clone()
        };
        self.navigation_cache.write().await.invalidate_uri(&uri);
        self.schedule_linked_document_prewarm(uri.clone(), text.clone());
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
            self.schedule_linked_document_prewarm(params.text_document.uri.clone(), text.clone());
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
        let key = NavigationQueryCacheKey {
            uri: uri.to_string(),
            version: document.version,
            line: position.line,
            character: position.character,
        };
        let cached_query = { self.navigation_cache.read().await.get(&key) };
        let query = if let Some(query) = cached_query {
            query
        } else {
            let index = self.index.read().await;
            let query = index.query_source_at_with_features(
                &path,
                &document.text,
                QueryPosition {
                    line: position.line,
                    character: position.character,
                },
                None,
                QueryFeatures::hover(),
            );
            self.navigation_cache
                .write()
                .await
                .insert(key, query.clone());
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
                    range: Some(lsp_range(&range)),
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
            range: Some(lsp_range(&hover.range)),
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
        Ok(Some(GotoDefinitionResponse::Scalar(Location {
            uri: Url::from_file_path(&definition.path)
                .unwrap_or(params.text_document_position_params.text_document.uri),
            range: lsp_range(&definition.range),
        })))
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
        let index = self.index.read().await;
        let Some(definition) = index.type_definition_at_source(
            &path,
            &document.text,
            QueryPosition {
                line: params.text_document_position_params.position.line,
                character: params.text_document_position_params.position.character,
            },
        ) else {
            return Ok(None);
        };
        Ok(Some(GotoTypeDefinitionResponse::Scalar(Location {
            uri: Url::from_file_path(&definition.path)
                .unwrap_or(params.text_document_position_params.text_document.uri),
            range: lsp_range(&definition.range),
        })))
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
        Ok(Some(GotoImplementationResponse::Scalar(Location {
            uri: Url::from_file_path(&definition.path)
                .unwrap_or(params.text_document_position_params.text_document.uri),
            range: lsp_range(&definition.range),
        })))
    }

    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        let uri = &params.text_document_position.text_document.uri;
        if let (Some(document), Some(_path)) = (self.document_for_uri(uri).await, uri_to_path(uri))
        {
            let index = self.index.read().await;
            let items = index
                .completion_items_at_source(
                    &document.text,
                    QueryPosition {
                        line: params.text_document_position.position.line,
                        character: params.text_document_position.position.character,
                    },
                    100,
                )
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
        let Some(context) = self
            .word_context_at_uri_position(uri, params.text_document_position.position)
            .await
        else {
            return Ok(Some(Vec::new()));
        };
        let declaration = if params.context.include_declaration {
            if source_range_is_declaration(&context.text, &context.word, &context.range) {
                Some(Location {
                    uri: uri.clone(),
                    range: context.range,
                })
            } else {
                self.definition_location_at(uri, params.text_document_position.position)
                    .await
                    .or_else(|| {
                        declaration_location_for_source_position(
                            uri,
                            &context.path,
                            &context.text,
                            &context.word,
                            params.text_document_position.position,
                        )
                    })
            }
        } else {
            None
        };
        Ok(Some(
            self.reference_locations(&context.word, declaration).await,
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
            .reference_locations(&target.word, target.declaration)
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
        let fallback = function_call_at_position(
            &document.text,
            params.text_document_position_params.position.line,
            params.text_document_position_params.position.character,
        );
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
            if definition.path == path {
                if let Some(item) = call_hierarchy_item_for_local_definition(
                    uri,
                    &path,
                    &document.text,
                    &definition,
                ) {
                    return Ok(Some(vec![item]));
                }
            }
            return Ok(Some(vec![call_hierarchy_item_from_definition(
                &definition,
                uri,
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
        for location in self
            .open_context_reference_locations(&params.item.name, &params.item.uri)
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
        let index = self.index.read().await;
        let mut calls = Vec::new();
        for (name, from_range) in call_ranges {
            if name == params.item.name {
                continue;
            }
            let to = parsed
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
                })
                .or_else(|| {
                    index
                        .resolve_symbol(&name, None)
                        .and_then(|symbol| call_hierarchy_item_from_symbol_record(&symbol))
                });
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
        let index = self.index.read().await;
        let symbols = index
            .workspace_symbols(&params.query, 200)
            .into_iter()
            .filter_map(|symbol| {
                Some(SymbolInformation {
                    name: symbol.name.clone(),
                    kind: symbol_kind(&symbol.kind),
                    tags: None,
                    deprecated: None,
                    location: Location {
                        uri: Url::from_file_path(&symbol.path).ok()?,
                        range: lsp_range(&symbol.range),
                    },
                    container_name: Some(symbol.module),
                })
            })
            .collect();
        Ok(Some(symbols))
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
    fn spawn_rebuild(&self) {
        let index = self.index.clone();
        let pending_jobs = self.pending_jobs.clone();
        let pending_index_task = self.pending_index_task.clone();
        let client = self.client.clone();
        tokio::spawn(async move {
            {
                let mut pending = pending_jobs.write().await;
                *pending = pending.saturating_add(1);
                *pending_index_task.write().await = Some("rebuild".to_string());
            }
            let options = { index.read().await.options().clone() };
            let mut rebuilt = WorkspaceIndex::new(options);
            let result = rebuilt.rebuild().map(|_| {
                let status = rebuilt.status();
                (rebuilt, status)
            });
            let result = match result {
                Ok((rebuilt, status)) => {
                    *index.write().await = rebuilt;
                    Ok(status)
                }
                Err(error) => Err(error),
            };
            {
                let mut pending = pending_jobs.write().await;
                *pending = pending.saturating_sub(1);
                if *pending == 0 {
                    *pending_index_task.write().await = None;
                }
            }
            match result {
                Ok(status) => {
                    client
                        .log_message(
                            MessageType::INFO,
                            format!(
                                "sage-ls indexed {} files and {} symbols in {}ms",
                                status.indexed_file_count,
                                status.symbol_count,
                                status.last_index_ms
                            ),
                        )
                        .await;
                    refresh_editor_feature_caches(&client).await;
                }
                Err(error) => {
                    client
                        .log_message(
                            MessageType::WARNING,
                            format!("sage-ls index rebuild failed: {error:#}"),
                        )
                        .await;
                }
            }
        });
    }

    fn spawn_cache_reconcile(&self) {
        let index = self.index.clone();
        let pending_jobs = self.pending_jobs.clone();
        let pending_index_task = self.pending_index_task.clone();
        let client = self.client.clone();
        tokio::spawn(async move {
            {
                let mut pending = pending_jobs.write().await;
                *pending = pending.saturating_add(1);
                *pending_index_task.write().await = Some("cache-check".to_string());
            }
            let (initial_generation, mut reconciled) = {
                let index = index.read().await;
                (index.status().generation, index.clone_for_background_work())
            };
            let result = reconciled.reconcile_with_cache().map(|_| {
                let status = reconciled.status();
                (reconciled, status)
            });
            let result = match result {
                Ok((reconciled, status)) => {
                    let mut current = index.write().await;
                    if current.status().generation == initial_generation {
                        *current = reconciled;
                        Ok((status, true))
                    } else {
                        Ok((current.status(), false))
                    }
                }
                Err(error) => Err(error),
            };
            {
                let mut pending = pending_jobs.write().await;
                *pending = pending.saturating_sub(1);
                if *pending == 0 {
                    *pending_index_task.write().await = None;
                }
            }
            match result {
                Ok((status, installed)) => {
                    client
                        .log_message(
                            MessageType::INFO,
                            format!(
                                "sage-ls reconciled {} files and {} symbols from persistent cache in {}ms ({} hit/{} miss, installed={})",
                                status.indexed_file_count,
                                status.symbol_count,
                                status.last_index_ms,
                                status.cache_hit_count,
                                status.cache_miss_count,
                                installed,
                            ),
                        )
                        .await;
                    if installed {
                        refresh_editor_feature_caches(&client).await;
                    }
                }
                Err(error) => {
                    client
                        .log_message(
                            MessageType::WARNING,
                            format!("sage-ls cache reconcile failed: {error:#}"),
                        )
                        .await;
                }
            }
        });
    }

    async fn refresh_paths(&self, changed: Vec<PathBuf>, deleted: Vec<PathBuf>) {
        let status = {
            let mut index = self.index.write().await;
            index.refresh_paths(&changed, &deleted)
        };
        match status {
            Ok(status) => {
                self.client
                    .log_message(
                        MessageType::INFO,
                        format!(
                            "sage-ls refreshed {} changed and {} deleted files in {}ms",
                            changed.len(),
                            deleted.len(),
                            status.last_index_ms
                        ),
                    )
                    .await;
                refresh_editor_feature_caches(&self.client).await;
            }
            Err(error) => {
                self.client
                    .log_message(
                        MessageType::WARNING,
                        format!("sage-ls incremental refresh failed: {error:#}"),
                    )
                    .await;
            }
        }
    }

    async fn index_status_payload(&self) -> Value {
        let mut payload =
            serde_json::to_value(self.index.read().await.status()).unwrap_or_else(|_| json!({}));
        if let Some(object) = payload.as_object_mut() {
            object.insert(
                "pending_jobs".to_string(),
                json!(*self.pending_jobs.read().await),
            );
            object.insert(
                "pending_task".to_string(),
                json!(self.pending_index_task.read().await.clone()),
            );
        }
        payload
    }

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
                    range: lsp_range(&diagnostic.range),
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
        self.open_documents.read().await.get(uri).cloned()
    }

    fn schedule_linked_document_prewarm(&self, uri: Url, text: String) {
        let index = Arc::clone(&self.index);
        tokio::spawn(async move {
            prewarm_linked_documents(index, uri, &text).await;
        });
    }

    async fn navigation_query_for_document(
        &self,
        uri: &Url,
        document: &OpenDocument,
        path: &Path,
        position: Position,
    ) -> QueryResult {
        let key = NavigationQueryCacheKey {
            uri: uri.to_string(),
            version: document.version,
            line: position.line,
            character: position.character,
        };
        if let Some(query) = self.navigation_cache.read().await.get(&key) {
            return query;
        }
        let query = {
            let index = self.index.read().await;
            index.query_source_at_navigation(
                path,
                &document.text,
                QueryPosition {
                    line: position.line,
                    character: position.character,
                },
            )
        };
        self.navigation_cache
            .write()
            .await
            .insert(key, query.clone());
        query
    }

    async fn text_for_uri_or_file(&self, uri: &Url) -> Option<String> {
        if let Some(document) = self.document_for_uri(uri).await {
            return Some(document.text);
        }
        let path = uri_to_path(uri)?;
        std::fs::read_to_string(path).ok()
    }

    async fn word_context_at_uri_position(
        &self,
        uri: &Url,
        position: Position,
    ) -> Option<WordPositionContext> {
        let path = uri_to_path(uri)?;
        let text = self.text_for_uri_or_file(uri).await?;
        let (word, range) = word_at_position(&text, position)?;
        Some(WordPositionContext {
            word,
            range,
            path,
            text,
        })
    }

    async fn prefix_at_uri_position(&self, uri: &Url, position: Position) -> Option<String> {
        let document = self.document_for_uri(uri).await?;
        current_prefix(&document.text, position)
    }

    async fn rename_target(&self, uri: &Url, position: Position) -> Option<RenameTarget> {
        let document = self.document_for_uri(uri).await?;
        let path = uri_to_path(uri)?;
        let (word, range) = word_at_position(&document.text, position)?;
        if !is_valid_identifier(&word)
            || !is_code_reference_range(&path, &document.text, &word, range)
        {
            return None;
        }
        if self.path_is_editable_fast(&path).await
            && local_rename_target_for_source(&path, &document.text, &word, range)
        {
            let declaration = self.editable_definition_location_at(uri, position).await;
            return Some(RenameTarget {
                word,
                range,
                declaration,
            });
        }
        let query = self
            .navigation_query_for_document(uri, &document, &path, position)
            .await;
        let definition = query.definition.as_ref()?;
        let index = self.index.read().await;
        if !index.is_editable_path(&definition.path) {
            return None;
        }
        Some(RenameTarget {
            word,
            range,
            declaration: location_for_query_definition(definition),
        })
    }

    async fn path_is_editable_fast(&self, path: &Path) -> bool {
        let roots = self.editable_roots.read().await;
        roots.is_empty() || roots.iter().any(|root| path.starts_with(root))
    }

    async fn editable_definition_location_at(
        &self,
        uri: &Url,
        position: Position,
    ) -> Option<Location> {
        let document = self.document_for_uri(uri).await?;
        let path = uri_to_path(uri)?;
        let query = self
            .navigation_query_for_document(uri, &document, &path, position)
            .await;
        let definition = query.definition.as_ref()?;
        let index = self.index.read().await;
        if !index.is_editable_path(&definition.path) {
            return None;
        }
        location_for_query_definition(definition)
    }

    async fn definition_location_at(&self, uri: &Url, position: Position) -> Option<Location> {
        let path = uri_to_path(uri)?;
        let query = if let Some(document) = self.document_for_uri(uri).await {
            self.navigation_query_for_document(uri, &document, &path, position)
                .await
        } else {
            let text = std::fs::read_to_string(&path).ok()?;
            let index = self.index.read().await;
            index.query_source_at_navigation(
                &path,
                &text,
                QueryPosition {
                    line: position.line,
                    character: position.character,
                },
            )
        };
        let definition = query.definition.as_ref()?;
        location_for_query_definition(definition)
    }

    async fn reference_locations(
        &self,
        word: &str,
        declaration: Option<Location>,
    ) -> Vec<Location> {
        let mut seen = BTreeSet::new();
        let mut locations = Vec::new();
        if let Some(location) = declaration {
            push_reference_location(&mut locations, &mut seen, location);
        }
        let index = self.index.read().await;
        for reference in index.editable_references(word) {
            if let Ok(uri) = Url::from_file_path(&reference.path) {
                push_reference_location(
                    &mut locations,
                    &mut seen,
                    Location {
                        uri,
                        range: lsp_range(&reference.range),
                    },
                );
            }
        }
        drop(index);
        for (uri, document) in self.open_documents.read().await.iter() {
            let Some(path) = uri_to_path(uri) else {
                continue;
            };
            for reference in sage_index::references_in_source(&path, &document.text, word) {
                push_reference_location(
                    &mut locations,
                    &mut seen,
                    Location {
                        uri: uri.clone(),
                        range: lsp_range(&reference.range),
                    },
                );
            }
        }
        locations
    }

    async fn open_context_reference_locations(
        &self,
        word: &str,
        fallback_uri: &Url,
    ) -> Vec<Location> {
        let mut seen = BTreeSet::new();
        let mut locations = Vec::new();
        let documents = self.open_documents.read().await.clone();
        for (uri, document) in documents {
            let Some(path) = uri_to_path(&uri) else {
                continue;
            };
            for reference in sage_index::references_in_source(&path, &document.text, word) {
                let key = reference_key(&uri, &reference.range);
                if seen.insert(key) {
                    locations.push(Location {
                        uri: uri.clone(),
                        range: lsp_range(&reference.range),
                    });
                }
            }
        }
        if locations.is_empty() {
            if let (Some(path), Some(text)) = (
                uri_to_path(fallback_uri),
                self.text_for_uri_or_file(fallback_uri).await,
            ) {
                for reference in sage_index::references_in_source(&path, &text, word) {
                    let key = reference_key(fallback_uri, &reference.range);
                    if seen.insert(key) {
                        locations.push(Location {
                            uri: fallback_uri.clone(),
                            range: lsp_range(&reference.range),
                        });
                    }
                }
            }
        }
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
                    Position::new(context.position.line, context.position.character),
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
        Some(DocumentationPositionContext {
            path,
            text,
            position: QueryPosition { line, character },
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
            self.index.read().await.query_source_at_with_features(
                &path,
                &document.text,
                QueryPosition { line, character },
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
    let (word, _) = word_at_position(text, Position::new(position.line, position.character))?;
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

fn declaration_location_for_source_position(
    uri: &Url,
    path: &Path,
    text: &str,
    word: &str,
    position: Position,
) -> Option<Location> {
    let query_position = QueryPosition {
        line: position.line,
        character: position.character,
    };
    parse_source(module_name_for_path(path), path, text)
        .symbols
        .into_iter()
        .filter(|symbol| symbol.name == word)
        .filter(|symbol| !matches!(symbol.kind, SageSymbolKind::Module | SageSymbolKind::Import))
        .find(|symbol| source_range_contains_position(&symbol.range, query_position))
        .map(|symbol| Location {
            uri: uri.clone(),
            range: lsp_range(&symbol.range),
        })
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

fn signature_information(
    label: String,
    documentation: Option<String>,
    active_parameter: u32,
) -> SignatureInformation {
    let parameters = signature_parameter_information(&label);
    SignatureInformation {
        label,
        documentation: documentation.map(Documentation::String),
        parameters: (!parameters.is_empty()).then_some(parameters),
        active_parameter: Some(active_parameter),
    }
}

fn signature_parameter_information(label: &str) -> Vec<ParameterInformation> {
    signature_parameter_offsets(label)
        .into_iter()
        .map(|offsets| ParameterInformation {
            label: ParameterLabel::LabelOffsets(offsets),
            documentation: None,
        })
        .collect()
}

fn signature_parameter_offsets(label: &str) -> Vec<[u32; 2]> {
    let Some(open) = label.find('(') else {
        return Vec::new();
    };
    let Some(close) = matching_signature_close(label, open) else {
        return Vec::new();
    };
    let mut offsets = Vec::new();
    let mut start = open + 1;
    let mut depth = 0usize;
    let mut quote: Option<char> = None;
    let mut escaped = false;

    for (relative, ch) in label[open + 1..close].char_indices() {
        let index = open + 1 + relative;
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        match quote {
            Some(active) if ch == active => quote = None,
            Some(_) => continue,
            None if ch == '\'' || ch == '"' => {
                quote = Some(ch);
                continue;
            }
            None => {}
        }
        match ch {
            '(' | '[' | '{' => depth = depth.saturating_add(1),
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                push_signature_parameter_offset(label, start, index, &mut offsets);
                start = index + 1;
            }
            _ => {}
        }
    }
    push_signature_parameter_offset(label, start, close, &mut offsets);
    offsets
}

fn matching_signature_close(label: &str, open: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for (relative, ch) in label[open..].char_indices() {
        let index = open + relative;
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        match quote {
            Some(active) if ch == active => quote = None,
            Some(_) => continue,
            None if ch == '\'' || ch == '"' => {
                quote = Some(ch);
                continue;
            }
            None => {}
        }
        match ch {
            '(' => depth = depth.saturating_add(1),
            ')' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => {}
        }
    }
    None
}

fn push_signature_parameter_offset(
    label: &str,
    start: usize,
    end: usize,
    offsets: &mut Vec<[u32; 2]>,
) {
    let mut trimmed_start = start;
    let mut trimmed_end = end;
    while trimmed_start < trimmed_end && label.as_bytes()[trimmed_start].is_ascii_whitespace() {
        trimmed_start += 1;
    }
    while trimmed_end > trimmed_start && label.as_bytes()[trimmed_end - 1].is_ascii_whitespace() {
        trimmed_end -= 1;
    }
    if trimmed_start < trimmed_end {
        offsets.push([trimmed_start as u32, trimmed_end as u32]);
    }
}

fn sage_document_links(text: &str, document_path: &Path) -> Vec<DocumentLink> {
    let base_dir = document_path.parent().unwrap_or_else(|| Path::new("."));
    let mut links = Vec::new();
    for (line_number, line) in text.lines().enumerate() {
        links.extend(sage_load_attach_links_in_line(
            line,
            line_number as u32,
            base_dir,
        ));
        if let Some(link) = cython_include_link_in_line(line, line_number as u32, base_dir) {
            links.push(link);
        }
    }
    links
}

fn sage_load_attach_links_in_line(
    line: &str,
    line_number: u32,
    base_dir: &Path,
) -> Vec<DocumentLink> {
    let bytes = line.as_bytes();
    let mut links = Vec::new();
    let mut quote: Option<u8> = None;
    let mut escaped = false;
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if escaped {
            escaped = false;
            index += 1;
            continue;
        }
        if byte == b'\\' {
            escaped = true;
            index += 1;
            continue;
        }
        if let Some(active) = quote {
            if byte == active {
                quote = None;
            }
            index += 1;
            continue;
        }
        if byte == b'#' {
            break;
        }
        if byte == b'\'' || byte == b'"' {
            quote = Some(byte);
            index += 1;
            continue;
        }
        if is_identifier_start(byte) && (index == 0 || !is_word_byte(bytes[index - 1])) {
            let start = index;
            let mut end = index + 1;
            while end < bytes.len() && is_word_byte(bytes[end]) {
                end += 1;
            }
            let name = &line[start..end];
            let mut cursor = end;
            while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
                cursor += 1;
            }
            if matches!(name, "load" | "attach") && cursor < bytes.len() && bytes[cursor] == b'(' {
                if let Some((target, inner_start, inner_end)) =
                    quoted_path_literal_after(line, cursor + 1)
                {
                    if let Some(link) = document_link_for_path_literal(
                        line_number,
                        inner_start,
                        inner_end,
                        base_dir,
                        &target,
                    ) {
                        links.push(link);
                    }
                }
            }
            index = end;
            continue;
        }
        index += 1;
    }
    links
}

fn cython_include_link_in_line(
    line: &str,
    line_number: u32,
    base_dir: &Path,
) -> Option<DocumentLink> {
    let code = code_before_comment(line);
    let leading = code.len().saturating_sub(code.trim_start().len());
    let trimmed = code.trim_start();
    let rest = trimmed.strip_prefix("include")?;
    if !rest
        .as_bytes()
        .first()
        .is_some_and(|byte| byte.is_ascii_whitespace())
    {
        return None;
    }
    let offset = leading + "include".len();
    let (target, inner_start, inner_end) = quoted_path_literal_after(line, offset)?;
    document_link_for_path_literal(line_number, inner_start, inner_end, base_dir, &target)
}

fn quoted_path_literal_after(line: &str, offset: usize) -> Option<(String, usize, usize)> {
    let bytes = line.as_bytes();
    let mut cursor = offset;
    while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
        cursor += 1;
    }
    let quote = *bytes.get(cursor)?;
    if quote != b'\'' && quote != b'"' {
        return None;
    }
    let inner_start = cursor + 1;
    cursor = inner_start;
    let mut escaped = false;
    let mut value = String::new();
    while cursor < bytes.len() {
        let byte = bytes[cursor];
        if escaped {
            value.push(byte as char);
            escaped = false;
            cursor += 1;
            continue;
        }
        if byte == b'\\' {
            escaped = true;
            cursor += 1;
            continue;
        }
        if byte == quote {
            return Some((value, inner_start, cursor));
        }
        value.push(byte as char);
        cursor += 1;
    }
    None
}

fn document_link_for_path_literal(
    line_number: u32,
    start: usize,
    end: usize,
    base_dir: &Path,
    target: &str,
) -> Option<DocumentLink> {
    if target.trim().is_empty() {
        return None;
    }
    let target_path = PathBuf::from(target);
    let resolved = if target_path.is_absolute() {
        target_path
    } else {
        base_dir.join(target_path)
    };
    let resolved = normalize_path_lexically(resolved);
    Some(DocumentLink {
        range: Range::new(
            Position::new(line_number, start as u32),
            Position::new(line_number, end as u32),
        ),
        target: Url::from_file_path(resolved).ok(),
        tooltip: Some("Open referenced Sage/Cython file".to_string()),
        data: None,
    })
}

fn normalize_path_lexically(path: PathBuf) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            std::path::Component::Normal(part) => normalized.push(part),
            std::path::Component::RootDir | std::path::Component::Prefix(_) => {
                normalized.push(component.as_os_str());
            }
        }
    }
    normalized
}

fn call_hierarchy_item_for_local_definition(
    uri: &Url,
    path: &Path,
    text: &str,
    definition: &QueryDefinition,
) -> Option<CallHierarchyItem> {
    let parsed = parse_source(module_name_for_path(path), path, text);
    parsed
        .symbols
        .iter()
        .find(|symbol| {
            symbol.name == definition.name
                && symbol.range.start_line == definition.range.start_line
                && is_call_hierarchy_symbol(symbol)
        })
        .map(|symbol| call_hierarchy_item_for_symbol(uri, text, symbol))
}

fn call_hierarchy_item_from_definition(
    definition: &QueryDefinition,
    fallback_uri: &Url,
) -> CallHierarchyItem {
    let uri = Url::from_file_path(&definition.path).unwrap_or_else(|_| fallback_uri.clone());
    let range = lsp_range(&definition.range);
    CallHierarchyItem {
        name: definition.name.clone(),
        kind: SymbolKind::FUNCTION,
        tags: None,
        detail: Some(definition.detail.clone()),
        uri,
        range,
        selection_range: range,
        data: None,
    }
}

fn call_hierarchy_item_from_symbol_record(symbol: &SymbolRecord) -> Option<CallHierarchyItem> {
    let uri = Url::from_file_path(&symbol.path).ok()?;
    let range = lsp_range(&symbol.range);
    Some(CallHierarchyItem {
        name: symbol.name.clone(),
        kind: symbol_kind(&symbol.kind),
        tags: None,
        detail: symbol
            .signature
            .clone()
            .or_else(|| (!symbol.detail.is_empty()).then(|| symbol.detail.clone())),
        uri,
        range,
        selection_range: range,
        data: None,
    })
}

fn enclosing_call_hierarchy_item(
    uri: &Url,
    path: &Path,
    text: &str,
    position: Position,
) -> Option<CallHierarchyItem> {
    let parsed = parse_source(module_name_for_path(path), path, text);
    let context = CallHierarchySourceContext {
        text: text.to_string(),
        symbols: parsed.symbols,
        folds: sage_folding_ranges(text),
    };
    enclosing_call_hierarchy_item_from_context(uri, &context, position)
}

fn call_hierarchy_item_for_local_symbol_at_position(
    uri: &Url,
    path: &Path,
    text: &str,
    position: Position,
) -> Option<CallHierarchyItem> {
    let (word, range) = word_at_position(text, position)?;
    if !is_code_reference_range(path, text, &word, range) {
        return None;
    }
    let parsed = parse_source(module_name_for_path(path), path, text);
    let folds = sage_folding_ranges(text);
    parsed
        .symbols
        .iter()
        .filter(|symbol| {
            symbol.name == word && symbol.path == path && is_call_hierarchy_symbol(symbol)
        })
        .min_by_key(|symbol| {
            (
                if lsp_range(&symbol.range) == range {
                    0
                } else {
                    1
                },
                symbol.range.start_line,
                symbol.range.start_character,
            )
        })
        .map(|symbol| call_hierarchy_item_for_symbol_with_folds(uri, text, &folds, symbol))
}

fn enclosing_call_hierarchy_item_from_context(
    uri: &Url,
    context: &CallHierarchySourceContext,
    position: Position,
) -> Option<CallHierarchyItem> {
    context
        .symbols
        .iter()
        .filter(|symbol| is_call_hierarchy_symbol(symbol))
        .filter_map(|symbol| {
            let item = call_hierarchy_item_for_symbol_with_folds(
                uri,
                &context.text,
                &context.folds,
                symbol,
            );
            contains_position(&item.range, position).then_some(item)
        })
        .min_by_key(|item| {
            (
                item.range.end.line.saturating_sub(item.range.start.line),
                item.range
                    .end
                    .character
                    .saturating_sub(item.range.start.character),
            )
        })
}

fn call_hierarchy_item_for_symbol(
    uri: &Url,
    text: &str,
    symbol: &SymbolRecord,
) -> CallHierarchyItem {
    let folds = sage_folding_ranges(text);
    call_hierarchy_item_for_symbol_with_folds(uri, text, &folds, symbol)
}

fn call_hierarchy_item_for_symbol_with_folds(
    uri: &Url,
    text: &str,
    folds: &[FoldingRange],
    symbol: &SymbolRecord,
) -> CallHierarchyItem {
    let selection_range = lsp_range(&symbol.range);
    let range = call_hierarchy_body_range(text, folds, symbol).unwrap_or(selection_range);
    CallHierarchyItem {
        name: symbol.name.clone(),
        kind: symbol_kind(&symbol.kind),
        tags: None,
        detail: symbol
            .signature
            .clone()
            .or_else(|| (!symbol.detail.is_empty()).then(|| symbol.detail.clone())),
        uri: uri.clone(),
        range,
        selection_range,
        data: None,
    }
}

fn call_hierarchy_body_range(
    text: &str,
    folds: &[FoldingRange],
    symbol: &SymbolRecord,
) -> Option<Range> {
    folds
        .iter()
        .find(|range| range.start_line == symbol.range.start_line)
        .map(|range| {
            Range::new(
                Position::new(range.start_line, 0),
                Position::new(range.end_line, line_length(text, range.end_line) as u32),
            )
        })
        .or_else(|| {
            line_selection_range(text, symbol.range.start_line, symbol.range.start_character)
        })
}

fn document_symbols_for_source(text: &str, symbols: &[SymbolRecord]) -> Vec<DocumentSymbol> {
    let folds = sage_folding_ranges(text);
    let mut items: Vec<_> = symbols
        .iter()
        .filter(|symbol| is_outline_document_symbol(&symbol.kind))
        .map(|symbol| document_symbol_for_record(text, &folds, symbol))
        .collect();
    items.sort_by_key(|symbol| {
        (
            symbol.range.start.line,
            symbol.range.start.character,
            symbol.selection_range.start.line,
            symbol.selection_range.start.character,
            symbol
                .range
                .end
                .line
                .saturating_sub(symbol.range.start.line),
        )
    });

    let mut roots = Vec::new();
    for item in items {
        insert_document_symbol(&mut roots, item);
    }
    roots
}

fn is_outline_document_symbol(kind: &SageSymbolKind) -> bool {
    !matches!(kind, SageSymbolKind::Module | SageSymbolKind::Import)
}

fn document_symbol_for_record(
    text: &str,
    folds: &[FoldingRange],
    symbol: &SymbolRecord,
) -> DocumentSymbol {
    let selection_range = lsp_range(&symbol.range);
    let range = if is_document_symbol_container_kind(&symbol.kind) {
        call_hierarchy_body_range(text, folds, symbol).unwrap_or(selection_range)
    } else {
        line_selection_range(text, symbol.range.start_line, symbol.range.start_character)
            .unwrap_or(selection_range)
    };
    DocumentSymbol {
        name: symbol.name.clone(),
        detail: symbol
            .signature
            .clone()
            .or_else(|| (!symbol.detail.is_empty()).then(|| symbol.detail.clone())),
        kind: symbol_kind(&symbol.kind),
        tags: None,
        deprecated: None,
        range,
        selection_range,
        children: None,
    }
}

fn insert_document_symbol(items: &mut Vec<DocumentSymbol>, symbol: DocumentSymbol) {
    if let Some(index) = (0..items.len())
        .rev()
        .find(|index| can_contain_document_symbol(&items[*index], &symbol))
    {
        let children = items[index].children.get_or_insert_with(Vec::new);
        insert_document_symbol(children, symbol);
        return;
    }
    items.push(symbol);
}

fn can_contain_document_symbol(parent: &DocumentSymbol, child: &DocumentSymbol) -> bool {
    is_document_symbol_container_kind_lsp(parent.kind)
        && parent.selection_range != child.selection_range
        && parent.range != child.range
        && contains_range(&parent.range, &child.selection_range)
}

fn is_document_symbol_container_kind(kind: &SageSymbolKind) -> bool {
    matches!(
        kind,
        SageSymbolKind::Function | SageSymbolKind::Class | SageSymbolKind::CythonDeclaration
    )
}

fn is_document_symbol_container_kind_lsp(kind: SymbolKindLsp) -> bool {
    matches!(
        kind,
        SymbolKindLsp::CLASS
            | SymbolKindLsp::FUNCTION
            | SymbolKindLsp::METHOD
            | SymbolKindLsp::CONSTRUCTOR
    )
}

fn is_call_hierarchy_symbol(symbol: &SymbolRecord) -> bool {
    matches!(
        symbol.kind,
        SageSymbolKind::Function | SageSymbolKind::Class | SageSymbolKind::CythonDeclaration
    )
}

fn push_incoming_call(
    calls: &mut Vec<CallHierarchyIncomingCall>,
    from: CallHierarchyItem,
    from_range: Range,
) {
    if let Some(existing) = calls
        .iter_mut()
        .find(|call| same_call_hierarchy_item(&call.from, &from))
    {
        existing.from_ranges.push(from_range);
        return;
    }
    calls.push(CallHierarchyIncomingCall {
        from,
        from_ranges: vec![from_range],
    });
}

fn push_outgoing_call(
    calls: &mut Vec<CallHierarchyOutgoingCall>,
    to: CallHierarchyItem,
    from_range: Range,
) {
    if let Some(existing) = calls
        .iter_mut()
        .find(|call| same_call_hierarchy_item(&call.to, &to))
    {
        existing.from_ranges.push(from_range);
        return;
    }
    calls.push(CallHierarchyOutgoingCall {
        to,
        from_ranges: vec![from_range],
    });
}

fn same_call_hierarchy_item(left: &CallHierarchyItem, right: &CallHierarchyItem) -> bool {
    left.name == right.name
        && left.uri == right.uri
        && left.selection_range == right.selection_range
}

fn call_ranges_in_range(text: &str, range: Range) -> Vec<(String, Range)> {
    let lines: Vec<_> = text.lines().collect();
    if lines.is_empty() {
        return Vec::new();
    }
    let mut calls = Vec::new();
    let start_line = range.start.line.min(lines.len().saturating_sub(1) as u32) as usize;
    let end_line = range.end.line.min(lines.len().saturating_sub(1) as u32) as usize;
    for line_number in start_line..=end_line {
        let Some(line) = lines.get(line_number) else {
            continue;
        };
        let scan_start = if line_number == start_line {
            range.start.character as usize
        } else {
            0
        }
        .min(line.len());
        let scan_end = if line_number == end_line {
            range.end.character as usize
        } else {
            line.len()
        }
        .min(line.len());
        calls.extend(call_ranges_in_line(
            line,
            line_number as u32,
            scan_start,
            scan_end,
        ));
    }
    calls
}

fn call_ranges_in_line(
    line: &str,
    line_number: u32,
    scan_start: usize,
    scan_end: usize,
) -> Vec<(String, Range)> {
    let bytes = line.as_bytes();
    let mut calls = Vec::new();
    let mut quote: Option<u8> = None;
    let mut escaped = false;
    let mut index = 0;
    while index < scan_end {
        let byte = bytes[index];
        if escaped {
            escaped = false;
            index += 1;
            continue;
        }
        if byte == b'\\' {
            escaped = true;
            index += 1;
            continue;
        }
        if let Some(active) = quote {
            if byte == active {
                quote = None;
            }
            index += 1;
            continue;
        }
        if byte == b'#' {
            break;
        }
        if byte == b'\'' || byte == b'"' {
            quote = Some(byte);
            index += 1;
            continue;
        }
        if index >= scan_start
            && is_identifier_start(byte)
            && (index == 0 || !is_word_byte(bytes[index - 1]))
        {
            let start = index;
            let mut end = index + 1;
            while end < scan_end && is_word_byte(bytes[end]) {
                end += 1;
            }
            let mut cursor = end;
            while cursor < scan_end && bytes[cursor].is_ascii_whitespace() {
                cursor += 1;
            }
            let name = &line[start..end];
            if cursor < scan_end
                && bytes[cursor] == b'('
                && !is_call_hierarchy_keyword(name)
                && !is_declaration_identifier(line, start)
                && previous_non_whitespace_byte(line, start) != Some(b'.')
            {
                calls.push((
                    name.to_string(),
                    Range::new(
                        Position::new(line_number, start as u32),
                        Position::new(line_number, end as u32),
                    ),
                ));
            }
            index = end;
            continue;
        }
        index += 1;
    }
    calls
}

fn is_identifier_start(byte: u8) -> bool {
    byte == b'_' || byte.is_ascii_alphabetic()
}

fn previous_non_whitespace_byte(line: &str, start: usize) -> Option<u8> {
    line.as_bytes()
        .get(..start)?
        .iter()
        .rev()
        .find(|byte| !byte.is_ascii_whitespace())
        .copied()
}

fn is_declaration_identifier(line: &str, start: usize) -> bool {
    line.get(..start)
        .unwrap_or_default()
        .split_whitespace()
        .last()
        .is_some_and(|token| matches!(token, "def" | "class" | "cdef" | "cpdef"))
}

fn is_call_hierarchy_keyword(name: &str) -> bool {
    matches!(
        name,
        "if" | "elif"
            | "for"
            | "while"
            | "with"
            | "except"
            | "return"
            | "yield"
            | "assert"
            | "lambda"
            | "def"
            | "class"
            | "cdef"
            | "cpdef"
    )
}

fn contains_position(range: &Range, position: Position) -> bool {
    position_leq(range.start, position) && position_leq(position, range.end)
}

fn module_name_for_path(path: &Path) -> &str {
    path.file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("document")
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
        .any(|reference| lsp_range(&reference.range) == target_range)
    {
        return Vec::new();
    }
    references
        .into_iter()
        .map(|reference| DocumentHighlight {
            range: lsp_range(&reference.range),
            kind: Some(DocumentHighlightKind::TEXT),
        })
        .collect()
}

fn is_code_reference_range(_path: &Path, text: &str, word: &str, target_range: Range) -> bool {
    sage_index::is_code_reference_at_range(text, word, &source_range_from_lsp(target_range))
}

fn local_rename_target_for_source(
    path: &Path,
    text: &str,
    word: &str,
    target_range: Range,
) -> bool {
    if !is_code_reference_range(path, text, word, target_range) {
        return false;
    }
    parse_source(module_name_for_path(path), path, text)
        .symbols
        .into_iter()
        .any(|symbol| {
            symbol.name == word
                && symbol.path == path
                && matches!(
                    symbol.kind,
                    SageSymbolKind::Class
                        | SageSymbolKind::Function
                        | SageSymbolKind::Variable
                        | SageSymbolKind::CythonDeclaration
                        | SageSymbolKind::PreparserGenerator
                )
        })
}

fn source_range_from_lsp(range: Range) -> sage_index::SourceRange {
    sage_index::SourceRange {
        start_line: range.start.line,
        start_character: range.start.character,
        end_line: range.end.line,
        end_character: range.end.character,
    }
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

fn symbol_kind(kind: &SageSymbolKind) -> SymbolKindLsp {
    match kind {
        SageSymbolKind::Class => SymbolKindLsp::CLASS,
        SageSymbolKind::Function | SageSymbolKind::CythonDeclaration => SymbolKindLsp::FUNCTION,
        SageSymbolKind::Module => SymbolKindLsp::MODULE,
        SageSymbolKind::Variable | SageSymbolKind::PreparserGenerator => SymbolKindLsp::VARIABLE,
        SageSymbolKind::Import => SymbolKindLsp::NAMESPACE,
    }
}

type SymbolKindLsp = tower_lsp::lsp_types::SymbolKind;

async fn prewarm_linked_documents(index: Arc<RwLock<WorkspaceIndex>>, uri: Url, text: &str) {
    let Some(path) = uri_to_path(&uri) else {
        return;
    };
    let mut targets: Vec<PathBuf> = sage_document_links(text, &path)
        .into_iter()
        .filter_map(|link| link.target)
        .filter_map(|target| uri_to_path(&target))
        .filter(|target| target != &path)
        .take(16)
        .collect();
    let import_modules = import_modules_for_prewarm(&path, text);
    if targets.is_empty() && import_modules.is_empty() {
        return;
    }
    let roots = {
        let index = index.read().await;
        for module in import_modules.into_iter().take(16) {
            if let Some(target) = index.source_path_for_module(&module) {
                targets.push(target);
            }
        }
        index.options().roots.clone()
    };
    let parsed_files: Vec<_> = targets
        .iter()
        .filter_map(|target| parse_file_for_roots(target, &roots).ok())
        .collect();
    if parsed_files.is_empty() {
        return;
    }
    for _ in 0..12 {
        if let Ok(mut index) = index.try_write() {
            index.preload_indexed_files(parsed_files.clone());
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
}

fn import_modules_for_prewarm(path: &Path, text: &str) -> Vec<String> {
    let mut modules = BTreeSet::new();
    let parsed = parse_source(module_name_for_path(path), path, text);
    for symbol in parsed.symbols {
        let Some(import_from) = symbol.import_from.as_deref() else {
            continue;
        };
        let module = import_from
            .split_once("::")
            .map_or(import_from, |(module, _)| module);
        if !module.is_empty() {
            modules.insert(module.to_string());
        }
    }
    modules.into_iter().collect()
}

fn lsp_range(range: &sage_index::SourceRange) -> Range {
    Range {
        start: Position {
            line: range.start_line,
            character: range.start_character,
        },
        end: Position {
            line: range.end_line,
            character: range.end_character,
        },
    }
}

fn apply_text_document_change(
    text: &mut String,
    change: &TextDocumentContentChangeEvent,
) -> std::result::Result<(), String> {
    let Some(range) = change.range else {
        *text = change.text.clone();
        return Ok(());
    };
    let start = position_to_byte_index(text, range.start)
        .ok_or_else(|| format!("invalid start position {:?}", range.start))?;
    let end = position_to_byte_index(text, range.end)
        .ok_or_else(|| format!("invalid end position {:?}", range.end))?;
    if start > end {
        return Err(format!("range start {start} is after end {end}"));
    }
    text.replace_range(start..end, &change.text);
    Ok(())
}

fn position_to_byte_index(text: &str, position: Position) -> Option<usize> {
    let (line_start, line_end) = line_byte_bounds(text, position.line)?;
    let line = &text[line_start..line_end];
    utf16_character_to_byte_offset(line, position.character).map(|offset| line_start + offset)
}

fn line_byte_bounds(text: &str, target_line: u32) -> Option<(usize, usize)> {
    if text.is_empty() {
        return (target_line == 0).then_some((0, 0));
    }

    let mut line = 0u32;
    let mut start = 0usize;
    for segment in text.split_inclusive('\n') {
        let mut end = start + segment.len();
        if segment.ends_with('\n') {
            end = end.saturating_sub(1);
            if end > start && text.as_bytes().get(end - 1) == Some(&b'\r') {
                end -= 1;
            }
        }
        if line == target_line {
            return Some((start, end));
        }
        start += segment.len();
        line = line.saturating_add(1);
    }

    (text.ends_with('\n') && line == target_line).then_some((text.len(), text.len()))
}

fn utf16_character_to_byte_offset(line: &str, character: u32) -> Option<usize> {
    let mut utf16_offset = 0u32;
    for (byte_offset, ch) in line.char_indices() {
        if utf16_offset == character {
            return Some(byte_offset);
        }
        let next = utf16_offset.saturating_add(ch.len_utf16() as u32);
        if character < next {
            return None;
        }
        utf16_offset = next;
    }
    if character >= utf16_offset {
        Some(line.len())
    } else {
        None
    }
}

fn word_at_position(text: &str, position: Position) -> Option<(String, Range)> {
    let line = text.lines().nth(position.line as usize)?;
    let mut character = position.character.min(line.len() as u32) as usize;
    if character == line.len() && character > 0 {
        character -= 1;
    }
    let bytes = line.as_bytes();
    if character >= bytes.len() {
        return None;
    }
    if !is_word_byte(bytes[character]) && character > 0 && is_word_byte(bytes[character - 1]) {
        character -= 1;
    }
    if !is_word_byte(bytes[character]) {
        return None;
    }
    let mut start = character;
    let mut end = character + 1;
    while start > 0 && is_word_byte(bytes[start - 1]) {
        start -= 1;
    }
    while end < bytes.len() && is_word_byte(bytes[end]) {
        end += 1;
    }
    Some((
        line[start..end].to_string(),
        Range {
            start: Position {
                line: position.line,
                character: start as u32,
            },
            end: Position {
                line: position.line,
                character: end as u32,
            },
        },
    ))
}

fn source_range_is_declaration(text: &str, word: &str, range: &Range) -> bool {
    let Some(line) = text.lines().nth(range.start.line as usize) else {
        return false;
    };
    let start = (range.start.character as usize).min(line.len());
    let prefix = line[..start].trim_start();
    if prefix.starts_with("def ")
        || prefix.starts_with("class ")
        || prefix.starts_with("cdef ")
        || prefix.starts_with("cpdef ")
        || prefix.ends_with(" def ")
        || prefix.ends_with(" class ")
    {
        return true;
    }
    let before_word = &line[..start];
    let declaration_patterns = [
        format!("def {word}"),
        format!("class {word}"),
        format!("cdef {word}"),
        format!("cpdef {word}"),
        format!("cdef class {word}"),
        format!("cdef inline {word}"),
        format!("cpdef inline {word}"),
    ];
    declaration_patterns
        .iter()
        .any(|pattern| before_word.trim_start().ends_with(pattern))
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
    let character = position.character.min(line.len() as u32) as usize;
    let bytes = line.as_bytes();
    let mut start = character;
    while start > 0 && is_word_byte(bytes[start - 1]) {
        start -= 1;
    }
    Some(line[start..character].to_string())
}

fn is_word_byte(byte: u8) -> bool {
    byte == b'_' || byte.is_ascii_alphanumeric()
}

fn is_valid_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first == '_' || first.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

fn sage_selection_ranges(text: &str, positions: &[Position]) -> Vec<SelectionRange> {
    positions
        .iter()
        .copied()
        .map(|position| sage_selection_range(text, position))
        .collect()
}

fn sage_selection_range(text: &str, position: Position) -> SelectionRange {
    let mut ranges = Vec::new();
    let has_word = if let Some((_, word_range)) = word_at_position(text, position) {
        push_selection_range(&mut ranges, word_range);
        true
    } else {
        push_selection_range(&mut ranges, Range::new(position, position));
        false
    };
    if let Some(line_range) = line_selection_range(text, position.line, position.character) {
        push_selection_range(&mut ranges, line_range);
    }
    for block_range in block_selection_ranges(text, position) {
        push_selection_range(&mut ranges, block_range);
    }
    push_selection_range(&mut ranges, document_selection_range(text));
    if ranges.is_empty()
        || (!has_word && !contains_range(&ranges[0], &Range::new(position, position)))
    {
        push_selection_range(&mut ranges, Range::new(position, position));
    }
    selection_range_chain(ranges)
}

fn line_selection_range(text: &str, line_number: u32, character: u32) -> Option<Range> {
    let line = text.lines().nth(line_number as usize)?;
    let mut start = line.len().saturating_sub(line.trim_start().len());
    let mut end = line.trim_end().len();
    if (character as usize) < start || (character as usize) > end {
        start = 0;
        end = line.len();
    }
    let (start, end) = if start <= end {
        (start, end)
    } else {
        (0, line.len())
    };
    Some(Range::new(
        Position::new(line_number, start as u32),
        Position::new(line_number, end as u32),
    ))
}

fn block_selection_ranges(text: &str, position: Position) -> Vec<Range> {
    let mut ranges: Vec<_> = sage_folding_ranges(text)
        .into_iter()
        .filter(|fold| fold.start_line <= position.line && position.line <= fold.end_line)
        .map(|fold| {
            Range::new(
                Position::new(fold.start_line, 0),
                Position::new(fold.end_line, line_length(text, fold.end_line) as u32),
            )
        })
        .filter(|range| range.start != range.end)
        .collect();
    ranges.sort_by_key(|range| {
        (
            range.end.line.saturating_sub(range.start.line),
            range.end.character.saturating_sub(range.start.character),
            range.start.line,
            range.start.character,
        )
    });
    ranges.dedup_by(|left, right| left == right);
    ranges
        .into_iter()
        .filter(|range| {
            (
                range.end.line.saturating_sub(range.start.line),
                range.end.character.saturating_sub(range.start.character),
            ) != (0, 0)
        })
        .collect()
}

fn document_selection_range(text: &str) -> Range {
    let line_count = text.lines().count();
    if line_count == 0 {
        return Range::new(Position::new(0, 0), Position::new(0, 0));
    }
    let end_line = line_count.saturating_sub(1) as u32;
    Range::new(
        Position::new(0, 0),
        Position::new(end_line, line_length(text, end_line) as u32),
    )
}

fn line_length(text: &str, line_number: u32) -> usize {
    text.lines()
        .nth(line_number as usize)
        .map(str::len)
        .unwrap_or_default()
}

fn push_selection_range(ranges: &mut Vec<Range>, range: Range) {
    if ranges.last().is_some_and(|existing| *existing == range) {
        return;
    }
    if ranges
        .last()
        .is_none_or(|existing| contains_range(&range, existing))
    {
        ranges.push(range);
    }
}

fn contains_range(outer: &Range, inner: &Range) -> bool {
    position_leq(outer.start, inner.start) && position_leq(inner.end, outer.end)
}

fn position_leq(left: Position, right: Position) -> bool {
    left.line < right.line || (left.line == right.line && left.character <= right.character)
}

fn selection_range_chain(ranges: Vec<Range>) -> SelectionRange {
    let mut parent = None;
    for range in ranges.into_iter().rev() {
        parent = Some(Box::new(SelectionRange { range, parent }));
    }
    *parent.expect("selection range chain should contain at least one range")
}

fn sage_folding_ranges(text: &str) -> Vec<FoldingRange> {
    let lines: Vec<_> = text.lines().collect();
    let mut ranges = Vec::new();
    add_explicit_region_folds(&lines, &mut ranges);
    add_comment_block_folds(&lines, &mut ranges);
    add_indentation_folds(&lines, &mut ranges);
    dedupe_folding_ranges(ranges)
}

fn add_explicit_region_folds(lines: &[&str], ranges: &mut Vec<FoldingRange>) {
    let mut stack = Vec::new();
    for (line_number, line) in lines.iter().enumerate() {
        let normalized = line.trim_start().to_ascii_lowercase();
        if normalized.starts_with("# region") {
            stack.push(line_number);
        } else if normalized.starts_with("# endregion") {
            let Some(start_line) = stack.pop() else {
                continue;
            };
            if line_number > start_line {
                ranges.push(folding_range(
                    start_line,
                    line_number,
                    Some(FoldingRangeKind::Region),
                ));
            }
        }
    }
}

fn add_comment_block_folds(lines: &[&str], ranges: &mut Vec<FoldingRange>) {
    let mut start_line = None;
    for (line_number, line) in lines.iter().enumerate() {
        let normalized = line.trim_start().to_ascii_lowercase();
        let is_comment = normalized.starts_with('#')
            && !normalized.starts_with("# region")
            && !normalized.starts_with("# endregion");
        if is_comment {
            start_line.get_or_insert(line_number);
            continue;
        }
        if let Some(start) = start_line.take() {
            if line_number.saturating_sub(start) > 1 {
                ranges.push(folding_range(
                    start,
                    line_number - 1,
                    Some(FoldingRangeKind::Comment),
                ));
            }
        }
    }
    if let Some(start) = start_line {
        if lines.len().saturating_sub(start) > 1 {
            ranges.push(folding_range(
                start,
                lines.len() - 1,
                Some(FoldingRangeKind::Comment),
            ));
        }
    }
}

fn add_indentation_folds(lines: &[&str], ranges: &mut Vec<FoldingRange>) {
    for (line_number, line) in lines.iter().enumerate() {
        let code = code_before_comment(line).trim_end();
        if !is_foldable_block_header(code.trim_start()) {
            continue;
        }
        let start_indent = leading_indent_width(line);
        let mut last_inside_line = None;
        for (next_line_number, next_line) in lines.iter().enumerate().skip(line_number + 1) {
            let trimmed = next_line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let indent = leading_indent_width(next_line);
            if indent <= start_indent {
                break;
            }
            last_inside_line = Some(next_line_number);
        }
        let Some(end_line) = last_inside_line else {
            continue;
        };
        if end_line > line_number {
            ranges.push(folding_range(line_number, end_line, None));
        }
    }
}

fn is_foldable_block_header(trimmed_code: &str) -> bool {
    if !trimmed_code.ends_with(':') {
        return false;
    }
    let headers = [
        "def ",
        "async def ",
        "class ",
        "cdef ",
        "cpdef ",
        "if ",
        "elif ",
        "else:",
        "for ",
        "async for ",
        "while ",
        "with ",
        "async with ",
        "try:",
        "except",
        "finally:",
        "match ",
        "case ",
    ];
    headers
        .iter()
        .any(|header| trimmed_code.starts_with(header))
}

fn leading_indent_width(line: &str) -> usize {
    line.chars()
        .take_while(|ch| *ch == ' ' || *ch == '\t')
        .map(|ch| if ch == '\t' { 4 } else { 1 })
        .sum()
}

fn folding_range(
    start_line: usize,
    end_line: usize,
    kind: Option<FoldingRangeKind>,
) -> FoldingRange {
    FoldingRange {
        start_line: start_line as u32,
        start_character: None,
        end_line: end_line as u32,
        end_character: None,
        kind,
        collapsed_text: None,
    }
}

fn dedupe_folding_ranges(ranges: Vec<FoldingRange>) -> Vec<FoldingRange> {
    let mut seen = BTreeSet::new();
    let mut deduped = Vec::new();
    for range in ranges {
        let kind = match &range.kind {
            Some(FoldingRangeKind::Comment) => "comment",
            Some(FoldingRangeKind::Imports) => "imports",
            Some(FoldingRangeKind::Region) => "region",
            None => "code",
        };
        let key = (range.start_line, range.end_line, kind);
        if seen.insert(key) {
            deduped.push(range);
        }
    }
    deduped.sort_by_key(|range| (range.start_line, range.end_line));
    deduped
}

fn sage_inlay_hints(text: &str, range: Range) -> Vec<InlayHint> {
    text.lines()
        .enumerate()
        .filter(|(line_number, _)| {
            let line = *line_number as u32;
            line >= range.start.line && line <= range.end.line
        })
        .filter_map(|(line_number, line)| {
            let code = code_before_comment(line).trim_end();
            let assignment = sage_assignment_for_inlay(code)?;
            let label = infer_sage_inlay_label(assignment.rhs)?;
            Some(InlayHint {
                position: Position {
                    line: line_number as u32,
                    character: assignment.name_end as u32,
                },
                label: InlayHintLabel::String(format!(": {label}")),
                kind: Some(InlayHintKind::TYPE),
                text_edits: None,
                tooltip: Some(InlayHintTooltip::String(
                    "Sage static type hint inferred from the assignment expression.".to_string(),
                )),
                padding_left: Some(true),
                padding_right: Some(false),
                data: None,
            })
        })
        .collect()
}

#[derive(Clone, Copy, Debug)]
struct SageInlayAssignment<'a> {
    name_end: usize,
    rhs: &'a str,
}

fn sage_assignment_for_inlay(line: &str) -> Option<SageInlayAssignment<'_>> {
    static PREPARSER_ASSIGNMENT_RE: OnceLock<regex::Regex> = OnceLock::new();
    static SIMPLE_ASSIGNMENT_RE: OnceLock<regex::Regex> = OnceLock::new();
    let preparser = PREPARSER_ASSIGNMENT_RE.get_or_init(|| {
        regex::Regex::new(r"^\s*(?P<name>[A-Za-z_][A-Za-z0-9_]*)\s*\.<[^>]+>\s*=\s*(?P<rhs>.+)$")
            .expect("valid preparser assignment regex")
    });
    let simple = SIMPLE_ASSIGNMENT_RE.get_or_init(|| {
        regex::Regex::new(r"^\s*(?P<name>[A-Za-z_][A-Za-z0-9_]*)\s*=\s*(?P<rhs>.+)$")
            .expect("valid assignment regex")
    });
    let captures = preparser.captures(line).or_else(|| simple.captures(line))?;
    let name = captures.name("name")?;
    let rhs = captures.name("rhs")?.as_str().trim_start();
    if rhs.starts_with('=') {
        return None;
    }
    Some(SageInlayAssignment {
        name_end: name.end(),
        rhs,
    })
}

fn infer_sage_inlay_label(rhs: &str) -> Option<&'static str> {
    let normalized = rhs.trim_start();
    if starts_with_call(normalized, &["GF", "FiniteField"]) {
        return Some("Field");
    }
    if starts_with_call(normalized, &["PolynomialRing", "BooleanPolynomialRing"]) {
        return Some("PolynomialRing");
    }
    if starts_with_call(
        normalized,
        &[
            "matrix",
            "Matrix",
            "zero_matrix",
            "identity_matrix",
            "random_matrix",
        ],
    ) {
        return Some("Matrix");
    }
    if starts_with_call(normalized, &["vector", "zero_vector", "random_vector"]) {
        return Some("Vector");
    }
    if starts_with_call(normalized, &["Graph", "DiGraph"]) {
        return Some("Graph");
    }
    if starts_with_call(normalized, &["EllipticCurve"]) {
        return Some("EllipticCurve");
    }
    if starts_with_call(
        normalized,
        &["NumberField", "CyclotomicField", "QuadraticField"],
    ) {
        return Some("NumberField");
    }
    if normalized.contains(".ideal(") || starts_with_call(normalized, &["ideal"]) {
        return Some("Ideal");
    }
    if normalized.contains(".gen(") || normalized.contains(".gen()") {
        return Some("PolynomialElement");
    }
    None
}

fn starts_with_call(value: &str, names: &[&str]) -> bool {
    names.iter().any(|name| {
        value
            .strip_prefix(name)
            .is_some_and(|rest| rest.trim_start().starts_with('('))
    })
}

fn code_before_comment(line: &str) -> &str {
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for (offset, ch) in line.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        match quote {
            Some(active) if ch == active => quote = None,
            Some(_) => {}
            None if ch == '\'' || ch == '"' => quote = Some(ch),
            None if ch == '#' => return &line[..offset],
            None => {}
        }
    }
    line
}

fn reference_key(uri: &Url, range: &sage_index::SourceRange) -> String {
    format!(
        "{}:{}:{}:{}",
        uri, range.start_line, range.start_character, range.end_character
    )
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

fn location_for_query_definition(definition: &QueryDefinition) -> Option<Location> {
    Url::from_file_path(&definition.path)
        .ok()
        .map(|uri| Location {
            uri,
            range: lsp_range(&definition.range),
        })
}

fn encode_semantic_tokens(text: &str) -> Vec<SemanticToken> {
    encode_semantic_spans(semantic_spans(text))
}

fn encode_semantic_tokens_for_range(text: &str, range: Range) -> Vec<SemanticToken> {
    encode_semantic_spans(
        semantic_spans(text)
            .into_iter()
            .filter(|span| semantic_span_intersects_range(span, &range)),
    )
}

fn encode_semantic_spans<I>(spans: I) -> Vec<SemanticToken>
where
    I: IntoIterator<Item = sage_index::SemanticSpan>,
{
    let mut data = Vec::new();
    let mut previous_line = 0u32;
    let mut previous_start = 0u32;
    for span in spans {
        let delta_line = span.line.saturating_sub(previous_line);
        let delta_start = if delta_line == 0 {
            span.start.saturating_sub(previous_start)
        } else {
            span.start
        };
        data.push(SemanticToken {
            delta_line,
            delta_start,
            length: span.length,
            token_type: token_type_index(&span.token_type),
            token_modifiers_bitset: modifier_bitset(&span.modifiers),
        });
        previous_line = span.line;
        previous_start = span.start;
    }
    data
}

fn semantic_span_intersects_range(span: &sage_index::SemanticSpan, range: &Range) -> bool {
    if span.line < range.start.line || span.line > range.end.line {
        return false;
    }
    let span_end = span.start.saturating_add(span.length);
    if span.line == range.start.line && span_end <= range.start.character {
        return false;
    }
    if span.line == range.end.line && span.start >= range.end.character {
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

fn uri_to_path(uri: &Url) -> Option<PathBuf> {
    uri.to_file_path().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sage_inlay_hints_infer_common_constructor_assignments() {
        let source = [
            "F = GF(7)",
            "R = PolynomialRing(F, 'x')",
            "M = matrix(F, 2, 2)",
            "v = vector(F, [1, 2])",
            "G = Graph([(0, 1)])",
            "E = EllipticCurve(F, [0, 1])",
            "K = NumberField(x^2 + 1, 'a')",
            "I = R.ideal(x^2 + 1)",
            "g = R.gen()",
        ]
        .join("\n");
        let hints = sage_inlay_hints(&source, full_range());
        let labels: Vec<_> = hints.iter().filter_map(hint_label).collect();

        assert_eq!(
            labels,
            vec![
                ": Field",
                ": PolynomialRing",
                ": Matrix",
                ": Vector",
                ": Graph",
                ": EllipticCurve",
                ": NumberField",
                ": Ideal",
                ": PolynomialElement",
            ]
        );
        assert_eq!(hints[0].position, Position::new(0, 1));
        assert_eq!(hints[1].position, Position::new(1, 1));
    }

    #[test]
    fn sage_inlay_hints_cover_preparser_assignments_and_skip_comments_strings() {
        let source = [
            "R.<x, y> = PolynomialRing(QQ, 2)",
            "text = 'PolynomialRing(QQ)'",
            "# M = matrix(QQ, 2)",
            "comparison == matrix(QQ, 2)",
            "A = zero_matrix(QQ, 2) # matrix comment",
        ]
        .join("\n");
        let hints = sage_inlay_hints(&source, full_range());
        let labels: Vec<_> = hints.iter().filter_map(hint_label).collect();

        assert_eq!(labels, vec![": PolynomialRing", ": Matrix"]);
        assert_eq!(hints[0].position, Position::new(0, 1));
        assert_eq!(hints[1].position, Position::new(4, 1));
    }

    #[test]
    fn sage_inlay_hints_respect_requested_line_range() {
        let source = [
            "F = GF(7)",
            "R = PolynomialRing(F, 'x')",
            "M = matrix(F, 2, 2)",
        ]
        .join("\n");
        let hints = sage_inlay_hints(
            &source,
            Range::new(Position::new(1, 0), Position::new(1, 200)),
        );
        let labels: Vec<_> = hints.iter().filter_map(hint_label).collect();

        assert_eq!(labels, vec![": PolynomialRing"]);
        assert_eq!(hints[0].position, Position::new(1, 1));
    }

    #[test]
    fn code_actions_offer_sage_exponent_quick_fixes() {
        let uri = Url::parse("file:///demo.sage").unwrap();
        let diagnostic = Diagnostic {
            range: Range::new(Position::new(0, 9), Position::new(0, 10)),
            severity: Some(DiagnosticSeverity::ERROR),
            code: Some(NumberOrString::String("syntax-error".to_string())),
            source: Some("sage-ls".to_string()),
            message: "Syntax error: incomplete Sage exponentiation".to_string(),
            ..Diagnostic::default()
        };
        let actions = code_actions_for_diagnostics(uri.clone(), std::slice::from_ref(&diagnostic));

        assert_eq!(actions.len(), 2);
        let titles: Vec<_> = actions
            .iter()
            .filter_map(|action| match action {
                CodeActionOrCommand::CodeAction(action) => Some(action.title.as_str()),
                CodeActionOrCommand::Command(_) => None,
            })
            .collect();
        assert_eq!(
            titles,
            vec![
                "Remove incomplete Sage exponent operator",
                "Insert exponent placeholder",
            ]
        );

        let first_edit = match &actions[0] {
            CodeActionOrCommand::CodeAction(action) => action
                .edit
                .as_ref()
                .and_then(|edit| edit.changes.as_ref())
                .and_then(|changes| changes.get(&uri))
                .and_then(|edits| edits.first())
                .expect("first action should include a text edit"),
            CodeActionOrCommand::Command(_) => panic!("expected code action"),
        };
        assert_eq!(first_edit.range, diagnostic.range);
        assert_eq!(first_edit.new_text, "");

        let unrelated = Diagnostic {
            message: "Syntax error: source could not be parsed".to_string(),
            ..diagnostic
        };
        assert!(code_actions_for_diagnostics(uri, &[unrelated]).is_empty());
    }

    #[test]
    fn code_actions_replace_python_sage_caret_exponents() {
        let uri = Url::parse("file:///demo.py").unwrap();
        let diagnostic = Diagnostic {
            range: Range::new(Position::new(2, 9), Position::new(2, 10)),
            severity: Some(DiagnosticSeverity::WARNING),
            code: Some(NumberOrString::String(
                "sage-python-caret-exponent".to_string(),
            )),
            source: Some("sage-ls".to_string()),
            message:
                "Sage-style exponent operator `^` has Python XOR semantics in `.py`; use `**`."
                    .to_string(),
            ..Diagnostic::default()
        };
        let actions = code_actions_for_diagnostics(uri.clone(), std::slice::from_ref(&diagnostic));

        assert_eq!(actions.len(), 1);
        let action = match &actions[0] {
            CodeActionOrCommand::CodeAction(action) => action,
            CodeActionOrCommand::Command(_) => panic!("expected code action"),
        };
        assert_eq!(action.title, "Replace Sage-style ^ with Python exponent **");
        let edit = action
            .edit
            .as_ref()
            .and_then(|edit| edit.changes.as_ref())
            .and_then(|changes| changes.get(&uri))
            .and_then(|edits| edits.first())
            .expect("action should include a text edit");
        assert_eq!(edit.range, diagnostic.range);
        assert_eq!(edit.new_text, "**");
    }

    #[test]
    fn initialization_options_parse_diagnostics_switch() {
        let disabled = parse_initialization_options(Some(json!({
            "analysis": {
                "enableDiagnostics": false,
                "enableRuntimeIntrospection": false,
                "enablePyxParsing": false
            }
        })));
        assert!(!disabled.analysis.enable_diagnostics);
        assert!(!disabled.analysis.enable_runtime_introspection);
        assert!(!disabled.analysis.enable_pyx_parsing);

        let defaults = parse_initialization_options(None);
        assert!(defaults.analysis.enable_diagnostics);
        assert!(defaults.analysis.enable_runtime_introspection);
        assert!(defaults.analysis.enable_pyx_parsing);
    }

    #[test]
    fn initialization_options_parse_documentation_hover_switch() {
        let disabled = parse_initialization_options(Some(json!({
            "documentation": {
                "preferredSource": "runtime",
                "showOnHover": false
            }
        })));

        assert_eq!(disabled.documentation.preferred_source, "runtime");
        assert!(!disabled.documentation.show_on_hover);

        let defaults = parse_initialization_options(None);
        assert_eq!(defaults.documentation.preferred_source, "auto");
        assert!(defaults.documentation.show_on_hover);
    }

    #[test]
    fn documentation_preferred_source_parses_known_values() {
        assert_eq!(
            DocumentationPreferredSource::from_config("auto"),
            DocumentationPreferredSource::Auto
        );
        assert_eq!(
            DocumentationPreferredSource::from_config("workspace"),
            DocumentationPreferredSource::Workspace
        );
        assert_eq!(
            DocumentationPreferredSource::from_config("runtime"),
            DocumentationPreferredSource::Runtime
        );
        assert_eq!(
            DocumentationPreferredSource::from_config("reference"),
            DocumentationPreferredSource::Reference
        );
        assert_eq!(
            DocumentationPreferredSource::from_config("unexpected"),
            DocumentationPreferredSource::Auto
        );
    }

    #[test]
    fn documentation_source_position_covers_external_definition_files() {
        let path = PathBuf::from("/workspace/sage/combinat/combination.py");
        let source = [
            "def Combinations(mset, k=None, *, as_tuples=False):",
            "    \"\"\"",
            "    Return the combinatorial class of combinations of the multiset.",
            "",
            "    EXAMPLES::",
            "",
            "        sage: C = Combinations(range(4)); C",
            "    \"\"\"",
            "    return []",
        ]
        .join("\n");
        let index = WorkspaceIndex::new(IndexOptions {
            roots: vec![PathBuf::from("/workspace")],
            editable_roots: Vec::new(),
            exclude_globs: Vec::new(),
            cache_dir: PathBuf::from("/tmp/sage-ls-doc-position-test"),
            enable_pyx: true,
        });

        let record = documentation_record_for_source_position(
            &index,
            &path,
            &source,
            QueryPosition {
                line: 0,
                character: 8,
            },
        )
        .expect("definition source position should produce docs");

        assert_eq!(record.name, "Combinations");
        assert_eq!(
            record.summary,
            "Return the combinatorial class of combinations of the multiset."
        );
        assert!(record
            .docstring
            .as_deref()
            .is_some_and(|doc| doc.contains("EXAMPLES::")));
    }

    #[test]
    fn declaration_source_position_covers_external_definition_files() {
        let path = PathBuf::from("/workspace/sage/combinat/combination.py");
        let uri = Url::from_file_path(&path).unwrap();
        let source = [
            "from sage.misc.lazy_import import lazy_import",
            "",
            "def Combinations(mset, k=None, *, as_tuples=False):",
            "    return []",
        ]
        .join("\n");

        let location = declaration_location_for_source_position(
            &uri,
            &path,
            &source,
            "Combinations",
            Position::new(2, 8),
        )
        .expect("definition source position should be returned as declaration");

        assert_eq!(location.uri, uri);
        assert_eq!(
            location.range,
            Range::new(Position::new(2, 4), Position::new(2, 16))
        );
        assert!(
            declaration_location_for_source_position(
                &location.uri,
                &path,
                &source,
                "lazy_import",
                Position::new(0, 34),
            )
            .is_none(),
            "imported names should not become declarations"
        );
    }

    #[test]
    fn runtime_docs_query_policy_respects_preferred_source() {
        let placeholder = DocumentationRecord {
            name: "PolynomialRing".to_string(),
            docstring: Some(
                "Known Sage symbol. Runtime documentation worker can provide details.".to_string(),
            ),
            ..DocumentationRecord::default()
        };
        let strong_static = DocumentationRecord {
            name: "PolynomialRing".to_string(),
            docstring: Some("Construct a polynomial ring.".to_string()),
            ..DocumentationRecord::default()
        };
        let query = QueryResult {
            target: Some(sage_index::QueryTarget {
                symbol: "PolynomialRing".to_string(),
                dotted_symbol: Some("sage.all.PolynomialRing".to_string()),
                range: sage_index::SourceRange::default(),
            }),
            documentation: Some(placeholder),
            ..QueryResult::default()
        };
        assert_eq!(
            runtime_docs_symbol_for_query(&query, DocumentationPreferredSource::Auto),
            Some("sage.all.PolynomialRing")
        );
        assert_eq!(
            runtime_docs_symbol_for_query(&query, DocumentationPreferredSource::Runtime),
            Some("sage.all.PolynomialRing")
        );
        assert_eq!(
            runtime_docs_symbol_for_query(&query, DocumentationPreferredSource::Workspace),
            None
        );
        assert_eq!(
            runtime_docs_symbol_for_query(&query, DocumentationPreferredSource::Reference),
            None
        );

        let strong_query = QueryResult {
            documentation: Some(strong_static),
            ..query
        };
        assert_eq!(
            runtime_docs_symbol_for_query(&strong_query, DocumentationPreferredSource::Auto),
            None
        );
        assert_eq!(
            runtime_docs_symbol_for_query(&strong_query, DocumentationPreferredSource::Runtime),
            Some("sage.all.PolynomialRing")
        );
    }

    #[test]
    fn hover_markdown_respects_documentation_preview_setting() {
        let markdown = [
            "```sage",
            "PolynomialRing(base_ring, names)",
            "```",
            "",
            "Module: `sage.rings.polynomial.polynomial_ring_constructor`",
            "",
            "Return a polynomial ring over the given base ring.",
        ]
        .join("\n");

        assert_eq!(hover_markdown_for_hover_setting(&markdown, true), markdown);

        let compact = hover_markdown_for_hover_setting(&markdown, false);
        assert!(compact.contains("PolynomialRing(base_ring, names)"));
        assert!(compact.contains("Module: `sage.rings.polynomial.polynomial_ring_constructor`"));
        assert!(!compact.contains("Return a polynomial ring"));
    }

    #[test]
    fn sage_folding_ranges_cover_python_sage_cython_and_comments() {
        let source = [
            "def kernel_columns(A):",
            "    if A.ncols() == 0:",
            "        return A",
            "    return A",
            "",
            "# region setup",
            "R = PolynomialRing(QQ, 'x')",
            "# endregion",
            "",
            "# first note",
            "# second note",
            "cdef class NativeThing:",
            "    cpdef rank(self):",
            "        return 1",
            "text = 'def fake():'",
        ]
        .join("\n");

        let ranges = sage_folding_ranges(&source);
        assert!(ranges
            .iter()
            .any(|range| range.start_line == 0 && range.end_line == 3 && range.kind.is_none()));
        assert!(ranges
            .iter()
            .any(|range| range.start_line == 1 && range.end_line == 2 && range.kind.is_none()));
        assert!(ranges.iter().any(|range| {
            range.start_line == 5
                && range.end_line == 7
                && range.kind == Some(FoldingRangeKind::Region)
        }));
        assert!(ranges.iter().any(|range| {
            range.start_line == 9
                && range.end_line == 10
                && range.kind == Some(FoldingRangeKind::Comment)
        }));
        assert!(ranges
            .iter()
            .any(|range| range.start_line == 11 && range.end_line == 13 && range.kind.is_none()));
        assert!(ranges
            .iter()
            .all(|range| range.start_line != 14 && range.end_line != 14));
    }

    #[test]
    fn sage_selection_ranges_expand_symbol_line_blocks_and_document() {
        let source = [
            "def kernel_columns(A):",
            "    if A.ncols() == 0:",
            "        return A",
            "    return A",
            "",
            "value = kernel_columns(M)",
        ]
        .join("\n");

        let chain = selection_chain_ranges(sage_selection_range(&source, Position::new(2, 15)));
        assert!(chain.len() >= 5, "{chain:?}");
        assert_eq!(
            chain[0],
            Range::new(Position::new(2, 15), Position::new(2, 16))
        );
        assert_eq!(
            chain[1],
            Range::new(Position::new(2, 8), Position::new(2, 16))
        );
        assert_eq!(
            chain[2],
            Range::new(Position::new(1, 0), Position::new(2, 16))
        );
        assert_eq!(
            chain[3],
            Range::new(Position::new(0, 0), Position::new(3, 12))
        );
        assert_eq!(
            chain.last().copied(),
            Some(Range::new(Position::new(0, 0), Position::new(5, 25)))
        );

        let leading_space_chain =
            selection_chain_ranges(sage_selection_range(&source, Position::new(1, 2)));
        assert_eq!(
            leading_space_chain[0],
            Range::new(Position::new(1, 2), Position::new(1, 2))
        );
        assert_eq!(
            leading_space_chain[1],
            Range::new(Position::new(1, 0), Position::new(1, 22))
        );
    }

    #[test]
    fn document_symbols_nest_classes_functions_and_locals() {
        let path = PathBuf::from("/workspace/demo.sage");
        let source = [
            "class Solver:",
            "    def build(self):",
            "        R = PolynomialRing(QQ, 'x')",
            "        return helper(R)",
            "",
            "def helper(R):",
            "    return R",
            "",
            "R.<x, y> = PolynomialRing(QQ, 2)",
        ]
        .join("\n");
        let parsed = parse_source(module_name_for_path(&path), &path, &source);
        let symbols = document_symbols_for_source(&source, &parsed.symbols);
        let names = symbols
            .iter()
            .map(|symbol| symbol.name.as_str())
            .collect::<Vec<_>>();

        assert!(names.contains(&"Solver"), "{names:?}");
        assert!(names.contains(&"helper"), "{names:?}");

        let solver = symbols
            .iter()
            .find(|symbol| symbol.name == "Solver")
            .expect("class should be a top-level outline entry");
        let children = solver
            .children
            .as_ref()
            .expect("class should contain nested method symbols");
        assert!(children.iter().any(|symbol| symbol.name == "build"));
        assert_eq!(
            solver.range,
            Range::new(Position::new(0, 0), Position::new(3, 24))
        );
        assert_eq!(
            solver.selection_range,
            Range::new(Position::new(0, 6), Position::new(0, 12))
        );
    }

    #[test]
    fn document_symbols_hide_module_and_import_metadata() {
        let path = PathBuf::from("/workspace/demo.py");
        let source = [
            "\"\"\"Module-level documentation.\"\"\"",
            "from sage.all import PolynomialRing",
            "",
            "def build_ring():",
            "    return PolynomialRing(QQ, 'x')",
        ]
        .join("\n");
        let parsed = parse_source(module_name_for_path(&path), &path, &source);
        let symbols = document_symbols_for_source(&source, &parsed.symbols);
        let names = symbols
            .iter()
            .map(|symbol| symbol.name.as_str())
            .collect::<Vec<_>>();

        assert_eq!(names, vec!["build_ring"]);
    }

    #[test]
    fn python_sage_import_items_are_detected_for_duplicate_navigation_suppression() {
        let path = PathBuf::from("/workspace/demo.py");
        let source = [
            "from sage.all import (",
            "    GF,",
            "    PolynomialRing,",
            ")",
            "",
            "R = PolynomialRing(GF(7), 'x')",
        ]
        .join("\n");
        let definition = QueryDefinition {
            name: "PolynomialRing".to_string(),
            path: PathBuf::from("/sage/sage/rings/polynomial/polynomial_ring_constructor.py"),
            range: sage_index::SourceRange {
                start_line: 60,
                start_character: 4,
                end_line: 60,
                end_character: 18,
            },
            detail: "def PolynomialRing(...)".to_string(),
            module: "sage.rings.polynomial.polynomial_ring_constructor".to_string(),
        };

        assert!(should_defer_python_import_definition_to_python_provider(
            &path,
            &source,
            Position::new(2, 8),
            &definition,
        ));
        assert!(!should_defer_python_import_definition_to_python_provider(
            &path,
            &source,
            Position::new(5, 6),
            &definition,
        ));
    }

    #[test]
    fn document_symbol_provider_has_visible_vscode_label() {
        assert_eq!(
            sage_document_symbol_options().label.as_deref(),
            Some("Sage")
        );
    }

    #[test]
    fn semantic_token_range_filters_to_requested_lines() {
        let source = [
            "# class IgnoredComment:",
            "class Solver:",
            "    def build(self):",
            "        R = PolynomialRing(QQ, 'x')",
            "text = 'def hidden():'",
        ]
        .join("\n");

        let class_tokens = encode_semantic_tokens_for_range(
            &source,
            Range::new(Position::new(1, 0), Position::new(2, 0)),
        );
        assert_eq!(class_tokens.len(), 1);
        assert_eq!(class_tokens[0].delta_line, 1);
        assert_eq!(class_tokens[0].delta_start, 6);
        assert_eq!(class_tokens[0].length, 6);
        assert_eq!(class_tokens[0].token_type, token_type_index("class"));

        let comment_tokens = encode_semantic_tokens_for_range(
            &source,
            Range::new(Position::new(0, 0), Position::new(1, 0)),
        );
        assert!(comment_tokens.is_empty());
    }

    #[test]
    fn incremental_text_change_applies_single_line_insert() {
        let mut source = "value = kernel_col\n".to_string();
        apply_text_document_change(
            &mut source,
            &TextDocumentContentChangeEvent {
                range: Some(Range::new(Position::new(0, 18), Position::new(0, 18))),
                range_length: None,
                text: "umns".to_string(),
            },
        )
        .unwrap();

        assert_eq!(source, "value = kernel_columns\n");
    }

    #[test]
    fn incremental_text_change_replaces_multiline_range() {
        let mut source = "def build():\n    pass\nvalue = 1\n".to_string();
        apply_text_document_change(
            &mut source,
            &TextDocumentContentChangeEvent {
                range: Some(Range::new(Position::new(1, 4), Position::new(2, 9))),
                range_length: None,
                text: "return 2".to_string(),
            },
        )
        .unwrap();

        assert_eq!(source, "def build():\n    return 2\n");
    }

    #[test]
    fn incremental_text_change_handles_utf16_positions() {
        let mut source = "text = \"😀\"\nvalue = π\n".to_string();
        apply_text_document_change(
            &mut source,
            &TextDocumentContentChangeEvent {
                range: Some(Range::new(Position::new(0, 8), Position::new(0, 10))),
                range_length: None,
                text: "theta".to_string(),
            },
        )
        .unwrap();
        apply_text_document_change(
            &mut source,
            &TextDocumentContentChangeEvent {
                range: Some(Range::new(Position::new(1, 8), Position::new(1, 9))),
                range_length: None,
                text: "pi".to_string(),
            },
        )
        .unwrap();

        assert_eq!(source, "text = \"theta\"\nvalue = pi\n");
    }

    #[test]
    fn incremental_text_change_rejects_split_surrogate_positions() {
        let mut source = "text = \"😀\"\n".to_string();
        let result = apply_text_document_change(
            &mut source,
            &TextDocumentContentChangeEvent {
                range: Some(Range::new(Position::new(0, 9), Position::new(0, 10))),
                range_length: None,
                text: "broken".to_string(),
            },
        );

        assert!(result.is_err());
        assert_eq!(source, "text = \"😀\"\n");
    }

    #[test]
    fn call_hierarchy_scanner_skips_declarations_methods_strings_and_comments() {
        let source = [
            "def main(A):",
            "    helper(A)",
            "    A.rank()",
            "    text = \"fake()\"",
            "    '''hidden()'''",
            "    # ignored()",
            "    R = PolynomialRing(QQ, 'x')",
            "    return zero_matrix(QQ, 1, 1)",
        ]
        .join("\n");
        let calls = call_ranges_in_range(
            &source,
            Range::new(Position::new(0, 0), Position::new(7, 35)),
        );
        let names = calls
            .iter()
            .map(|(name, _)| name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["helper", "PolynomialRing", "zero_matrix"]);
    }

    #[test]
    fn call_hierarchy_enclosing_item_finds_nearest_function_block() {
        let path = PathBuf::from("/workspace/demo.sage");
        let uri = Url::from_file_path(&path).unwrap();
        let source = [
            "def helper(A):",
            "    return A",
            "",
            "def main(A):",
            "    if A:",
            "        return helper(A)",
        ]
        .join("\n");

        let item =
            enclosing_call_hierarchy_item(&uri, &path, &source, Position::new(5, 18)).unwrap();
        assert_eq!(item.name, "main");
        assert_eq!(item.selection_range.start, Position::new(3, 4));
        assert_eq!(
            item.range,
            Range::new(Position::new(3, 0), Position::new(5, 24))
        );
    }

    #[test]
    fn call_hierarchy_prepare_prefers_open_document_local_definitions() {
        let path = PathBuf::from("/workspace/demo.py");
        let uri = Url::from_file_path(&path).unwrap();
        let source = [
            "from sage.all import zero_matrix",
            "",
            "def kernel_columns(A):",
            "    if A.ncols() == 0:",
            "        return zero_matrix(A.base_ring(), 0, 0)",
            "    return A",
            "",
            "def caller(M):",
            "    return kernel_columns(M)",
        ]
        .join("\n");

        let item = call_hierarchy_item_for_local_symbol_at_position(
            &uri,
            &path,
            &source,
            Position::new(8, 13),
        )
        .expect("local function reference should resolve before global index fallback");

        assert_eq!(item.name, "kernel_columns");
        assert_eq!(item.uri, uri);
        assert_eq!(
            item.selection_range,
            Range::new(Position::new(2, 4), Position::new(2, 18))
        );
        assert_eq!(
            item.range,
            Range::new(Position::new(2, 0), Position::new(5, 12))
        );
    }

    #[test]
    fn local_rename_fast_path_allows_editable_symbols_but_not_imports() {
        let path = PathBuf::from("/workspace/demo.py");
        let source = [
            "from sage.all import PolynomialRing",
            "",
            "def kernel_columns(A):",
            "    return A",
            "",
            "value = kernel_columns(M)",
        ]
        .join("\n");

        assert!(local_rename_target_for_source(
            &path,
            &source,
            "kernel_columns",
            Range::new(Position::new(5, 8), Position::new(5, 22)),
        ));
        assert!(!local_rename_target_for_source(
            &path,
            &source,
            "PolynomialRing",
            Range::new(Position::new(0, 21), Position::new(0, 35)),
        ));
    }

    #[test]
    fn sage_document_links_cover_load_attach_and_cython_include() {
        let path = PathBuf::from("/workspace/project/src/demo.sage");
        let source = [
            "load(\"helpers/setup.sage\")",
            "attach('../shared/tools.sage')",
            "# load('ignored.sage')",
            "text = \"load('ignored.sage')\"",
            "include \"native_include.pxi\"",
            "    include 'native_support.pxd'",
        ]
        .join("\n");

        let links = sage_document_links(&source, &path);
        let targets = links
            .iter()
            .filter_map(|link| link.target.as_ref())
            .map(|uri| uri.to_file_path().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(links.len(), 4);
        assert!(targets.contains(&PathBuf::from("/workspace/project/src/helpers/setup.sage")));
        assert!(targets.contains(&PathBuf::from("/workspace/project/shared/tools.sage")));
        assert!(targets.contains(&PathBuf::from("/workspace/project/src/native_include.pxi")));
        assert!(targets.contains(&PathBuf::from("/workspace/project/src/native_support.pxd")));
        assert_eq!(
            links[0].range,
            Range::new(Position::new(0, 6), Position::new(0, 24))
        );
    }

    #[test]
    fn import_modules_for_prewarm_extracts_local_targets() {
        let path = PathBuf::from("/workspace/project/src/demo.sage");
        let source = [
            "from local_docs import PolynomialNotebook",
            "from package_demo import named_polynomial, AffineNote",
            "from external_series import EXTERNAL_LABEL as label_value",
        ]
        .join("\n");

        assert_eq!(
            import_modules_for_prewarm(&path, &source),
            vec![
                "external_series".to_string(),
                "local_docs".to_string(),
                "package_demo".to_string(),
            ]
        );
    }

    #[test]
    fn document_highlights_cover_code_references_only() {
        let path = PathBuf::from("/workspace/demo.sage");
        let source = [
            "def kernel_columns(A):",
            "    return A",
            "N = kernel_columns(M)",
            "text = 'kernel_columns(M)'",
            "# kernel_columns(comment)",
            "K = kernel_columns(N)",
        ]
        .join("\n");
        let highlights = document_highlights_for_source(
            &path,
            &source,
            "kernel_columns",
            Range::new(Position::new(0, 4), Position::new(0, 18)),
        );

        assert_eq!(highlights.len(), 3);
        assert_eq!(
            highlights
                .iter()
                .map(|highlight| highlight.range.start.line)
                .collect::<Vec<_>>(),
            vec![0, 2, 5]
        );
        assert!(highlights
            .iter()
            .all(|highlight| highlight.kind == Some(DocumentHighlightKind::TEXT)));

        let comment_range = Range::new(Position::new(4, 2), Position::new(4, 16));
        assert!(
            document_highlights_for_source(&path, &source, "kernel_columns", comment_range,)
                .is_empty()
        );
    }

    #[test]
    fn signature_information_extracts_parameter_offsets() {
        let label = "trace_window(poly, base_ring=QQ, *, width=5, normalize=True)".to_string();
        let info = signature_information(label.clone(), Some("docs".to_string()), 1);
        assert_eq!(info.active_parameter, Some(1));
        assert_eq!(
            info.documentation,
            Some(Documentation::String("docs".to_string()))
        );
        let parameters = info.parameters.expect("parameters should be present");
        let labels = parameters
            .iter()
            .map(|parameter| match parameter.label {
                ParameterLabel::LabelOffsets([start, end]) => &label[start as usize..end as usize],
                ParameterLabel::Simple(_) => panic!("expected offset labels"),
            })
            .collect::<Vec<_>>();
        assert_eq!(
            labels,
            vec!["poly", "base_ring=QQ", "*", "width=5", "normalize=True"]
        );
    }

    #[test]
    fn signature_parameters_ignore_nested_commas_and_strings() {
        let label = "foo(a, data=(1, 2), names='x,y', options={\"k\": [1, 2]})";
        let labels = signature_parameter_offsets(label)
            .into_iter()
            .map(|[start, end]| &label[start as usize..end as usize])
            .collect::<Vec<_>>();
        assert_eq!(
            labels,
            vec!["a", "data=(1, 2)", "names='x,y'", "options={\"k\": [1, 2]}"]
        );
        assert!(signature_parameter_offsets("foo()").is_empty());
    }

    fn selection_chain_ranges(selection_range: SelectionRange) -> Vec<Range> {
        let mut ranges = Vec::new();
        let mut current = Some(selection_range);
        while let Some(selection) = current {
            ranges.push(selection.range);
            current = selection.parent.map(|parent| *parent);
        }
        ranges
    }

    fn full_range() -> Range {
        Range::new(Position::new(0, 0), Position::new(u32::MAX, u32::MAX))
    }

    fn hint_label(hint: &InlayHint) -> Option<&str> {
        match &hint.label {
            InlayHintLabel::String(label) => Some(label.as_str()),
            InlayHintLabel::LabelParts(_) => None,
        }
    }
}
