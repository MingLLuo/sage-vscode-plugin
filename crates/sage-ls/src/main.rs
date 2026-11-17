#![allow(deprecated)]

mod analysis_mode;
mod call_hierarchy;
mod code_actions;
mod completion_items;
mod document_links;
mod documentation;
mod editor_features;
mod index_jobs;
mod initialization;
mod linked_document_prewarm;
mod memory;
mod navigation;
mod open_documents;
mod references;
mod request_trace;
mod runtime_docs;
mod semantic_tokens;
mod signature_help;
mod source_symbols;
mod text_positions;

use analysis_mode::AnalysisMode;
use call_hierarchy::{
    call_hierarchy_item_for_local_definition, call_hierarchy_item_for_local_symbol_at_position,
    call_hierarchy_item_from_definition, enclosing_call_hierarchy_item,
    enclosing_call_hierarchy_item_from_context, high_confidence_call_hierarchy_definition,
    is_identifier_start, push_incoming_call, push_outgoing_call, resolve_outgoing_calls,
    CallHierarchySourceContext,
};
use code_actions::{code_actions_for_diagnostics, diagnostic_severity};
use completion_items::{
    completion_documentation, completion_item, current_prefix, query_completion_item,
    CompletionResolveData,
};
use document_links::sage_document_links;
use documentation::{
    documentation_record_for_source_position, hover_markdown_for_documentation,
    hover_markdown_for_hover_setting, is_runtime_placeholder_documentation,
    runtime_docs_symbol_for_query,
};
#[cfg(test)]
use editor_features::sage_selection_range;
use editor_features::{sage_folding_ranges, sage_inlay_hints, sage_selection_ranges};
use index_jobs::{IndexActor, IndexHandle};
use initialization::{
    default_excludes, parse_initialization_options, source_roots_from_options,
    workspace_folders_from_options, DocumentationPreferredSource,
};
#[cfg(test)]
use linked_document_prewarm::import_modules_for_prewarm;
use linked_document_prewarm::LinkedDocumentPrewarmer;
pub(crate) use memory::release_freed_memory;
#[cfg(test)]
use navigation::{
    live_definition_range, navigation_response_for_links,
    should_defer_python_import_definition_to_python_provider, validated_disk_definition_location,
    NavigationQueryCacheKey, NavigationRequestKind,
};
use navigation::{
    navigation_query_cache_key, NavigationLinkSupport, NavigationQueryCache,
    NavigationQueryCacheFlavor,
};
#[cfg(test)]
use open_documents::source_text_fingerprint;
use open_documents::{
    live_document_for_path, live_document_for_uri_or_path, parse_uri, uri_to_path, DocumentStore,
    OpenDocument,
};
use references::ReferenceCollectionMode;
#[cfg(test)]
use references::{
    indexed_reference_locations, local_import_alias_rename_target,
    local_import_alias_rename_target_with_symbols, push_scoped_reference_location,
    reference_candidate_matches_target, reference_candidate_matches_target_with_symbols,
    reference_path_is_collectible, same_definition_identity, same_definition_owner_identity,
    ResolvedReferenceTarget,
};
use request_trace::RequestTrace;
use runtime_docs::{RuntimeDocsConfig, RuntimeDocsWorker};
use sage_index::{
    active_parameter_for_keyword, default_cache_dir, function_call_context_at_position,
    parse_source, semantic_spans, DocumentationRecord, IndexOptions, QueryCompletion,
    QueryDefinition, QueryFeatures, QueryPosition, QueryResult, SymbolKind as SageSymbolKind,
    SymbolRecord, WorkspaceIndex,
};
use semantic_tokens::{encode_semantic_tokens, tokens_for_range};
#[cfg(test)]
use semantic_tokens::{encode_semantic_tokens_for_range, token_type_index};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use signature_help::signature_information;
#[cfg(test)]
use signature_help::signature_parameter_offsets;
use source_symbols::{document_symbols_for_source, module_name_for_path};
#[cfg(test)]
use std::collections::BTreeSet;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;
use text_positions::{
    apply_text_document_change, byte_offset_to_utf16_character, is_word_byte,
    lsp_position_for_byte_column, lsp_range_for_text, query_position_from_lsp,
    utf16_character_to_byte_offset, word_at_position,
};
use tokio::sync::RwLock;
use tower_lsp_server::jsonrpc::Result;
use tower_lsp_server::ls_types::request::{
    GotoDeclarationParams, GotoDeclarationResponse, GotoImplementationParams,
    GotoImplementationResponse, GotoTypeDefinitionParams, GotoTypeDefinitionResponse,
};
use tower_lsp_server::ls_types::*;
use tower_lsp_server::{Client, LanguageServer, LspService, Server};

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

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

struct Backend {
    client: Client,
    index: IndexHandle,
    open_documents: Arc<DocumentStore>,
    navigation_cache: Arc<RwLock<NavigationQueryCache>>,
    navigation_link_support: Arc<RwLock<NavigationLinkSupport>>,
    versioned_workspace_edits: Arc<RwLock<bool>>,
    analysis_mode: Arc<RwLock<AnalysisMode>>,
    diagnostics_enabled: Arc<RwLock<bool>>,
    docs_on_hover_enabled: Arc<RwLock<bool>>,
    docs_preferred_source: Arc<RwLock<DocumentationPreferredSource>>,
    index_actor: IndexActor,
    shutting_down: Arc<AtomicBool>,
    linked_document_prewarmer: LinkedDocumentPrewarmer,
    runtime_docs: RuntimeDocsWorker,
    query_slots: Arc<tokio::sync::Semaphore>,
    reference_slots: Arc<tokio::sync::Semaphore>,
    diagnostic_tasks: std::sync::Mutex<HashMap<Uri, tokio::task::AbortHandle>>,
}

#[derive(Clone, Debug)]
struct DocumentationPositionContext {
    path: PathBuf,
    text: String,
    position: QueryPosition,
}

fn main() {
    if std::env::args()
        .skip(1)
        .any(|arg| arg == "--version" || arg == "-V")
    {
        println!("sage-ls {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    let threads = std::thread::available_parallelism()
        .map_or(2, usize::from)
        .min(4);
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(threads)
        .enable_all()
        .build()
        .expect("create LSP runtime");
    let orderly = runtime.block_on(async {
        let (service, socket) = LspService::new(Backend::new);
        let shutting_down = service.inner().shutting_down.clone();
        let runtime_docs = service.inner().runtime_docs.clone();
        Server::new(tokio::io::stdin(), tokio::io::stdout(), socket)
            .concurrency_level(32)
            .serve(service)
            .await;
        runtime_docs.shutdown();
        shutting_down.load(Ordering::Acquire)
    });
    // Tokio stdin uses a blocking read which cannot be cancelled if the client keeps its pipe open.
    runtime.shutdown_timeout(std::time::Duration::from_millis(100));
    std::process::exit(if orderly { 0 } else { 1 });
}

struct CancelOnDrop(Arc<AtomicBool>);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

async fn run_blocking<T: Send + 'static>(
    slots: Arc<tokio::sync::Semaphore>,
    work: impl FnOnce() -> T + Send + 'static,
) -> Result<T> {
    run_cancellable(slots, move |_| work()).await
}

async fn run_cancellable<T: Send + 'static>(
    slots: Arc<tokio::sync::Semaphore>,
    work: impl FnOnce(Arc<AtomicBool>) -> T + Send + 'static,
) -> Result<T> {
    let cancelled = Arc::new(AtomicBool::new(false));
    let _cancel_on_drop = CancelOnDrop(cancelled.clone());
    let permit = slots
        .acquire_owned()
        .await
        .map_err(|_| tower_lsp_server::jsonrpc::Error::internal_error())?;
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        if cancelled.load(Ordering::Acquire) {
            return Err(tower_lsp_server::jsonrpc::Error::request_cancelled());
        }
        Ok(work(cancelled))
    })
    .await
    .map_err(|_| tower_lsp_server::jsonrpc::Error::internal_error())?
}

impl Backend {
    fn new(client: Client) -> Self {
        let index = Arc::new(arc_swap::ArcSwap::from_pointee(WorkspaceIndex::default()));
        let shutting_down = Arc::new(AtomicBool::new(false));
        let index_actor =
            IndexActor::new(index.clone(), Some(client.clone()), shutting_down.clone());
        Self {
            client,
            index,
            open_documents: Arc::default(),
            navigation_cache: Arc::new(RwLock::new(NavigationQueryCache::default())),
            navigation_link_support: Arc::new(RwLock::new(NavigationLinkSupport::default())),
            versioned_workspace_edits: Arc::new(RwLock::new(false)),
            analysis_mode: Arc::new(RwLock::new(AnalysisMode::default())),
            diagnostics_enabled: Arc::new(RwLock::new(true)),
            docs_on_hover_enabled: Arc::new(RwLock::new(true)),
            docs_preferred_source: Arc::new(RwLock::new(DocumentationPreferredSource::Auto)),
            index_actor,
            shutting_down,
            linked_document_prewarmer: LinkedDocumentPrewarmer::default(),
            runtime_docs: RuntimeDocsWorker::default(),
            query_slots: Arc::new(tokio::sync::Semaphore::new(3)),
            reference_slots: Arc::new(tokio::sync::Semaphore::new(1)),
            diagnostic_tasks: std::sync::Mutex::default(),
        }
    }
}

impl LanguageServer for Backend {
    async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {
        let initialize_started = Instant::now();
        *self.versioned_workspace_edits.write().await = params
            .capabilities
            .workspace
            .as_ref()
            .and_then(|workspace| workspace.workspace_edit.as_ref())
            .and_then(|edit| edit.document_changes)
            .unwrap_or(false);
        *self.navigation_link_support.write().await =
            NavigationLinkSupport::from_client_capabilities(&params.capabilities);
        let options = parse_initialization_options(params.initialization_options);
        trace_initialize_phase(initialize_started, "parse-options");
        let analysis_mode = options.analysis.mode.effective();
        *self.analysis_mode.write().await = analysis_mode;
        if let Some(invalid_mode) = options.analysis.mode.invalid_value() {
            self.client
                .log_message(
                    MessageType::WARNING,
                    format!(
                        "sage-ls received unsupported analysis mode {invalid_mode:?}; using default"
                    ),
                )
                .await;
        }
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
        self.index
            .store(Arc::new(WorkspaceIndex::new(index_options)));
        self.index_actor.reconcile();
        trace_initialize_phase(initialize_started, "spawn-reconcile");

        Ok(InitializeResult {
            offset_encoding: None,
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
                    // The extension routes these internal requests to its selected project
                    // client. Advertising them here makes each LanguageClient register the
                    // same global VS Code command IDs, so the second workspace cannot start.
                    commands: vec![],
                    ..ExecuteCommandOptions::default()
                }),
                ..ServerCapabilities::default()
            },
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        // Automatic cache cleanup removes index databases untouched for 30 days. A window can stay
        // open for months without writing to its database, so mark it used a few times a day.
        let index = self.index.clone();
        let shutting_down = self.shutting_down.clone();
        tokio::spawn(async move {
            let mut ticks = tokio::time::interval(std::time::Duration::from_secs(6 * 60 * 60));
            loop {
                ticks.tick().await;
                if shutting_down.load(Ordering::Acquire) {
                    return;
                }
                let path = index.load().db_path().to_path_buf();
                let _ = tokio::task::spawn_blocking(move || {
                    std::fs::File::options()
                        .write(true)
                        .open(path)
                        .and_then(|file| file.set_modified(std::time::SystemTime::now()))
                })
                .await;
            }
        });
        let analysis_mode = *self.analysis_mode.read().await;
        self.client
            .log_message(
                MessageType::INFO,
                format!(
                    "sage-ls v2 runtime initialized (analysis mode {}, workspace symbol limit {})",
                    analysis_mode.as_str(),
                    analysis_mode.workspace_symbol_limit(),
                ),
            )
            .await;
    }

    async fn shutdown(&self) -> Result<()> {
        self.shutting_down.store(true, Ordering::Release);
        self.linked_document_prewarmer.cancel_all();
        self.index_actor.stop();
        self.runtime_docs.shutdown();
        for (_, task) in self
            .diagnostic_tasks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .drain()
        {
            task.abort();
        }
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let uri = params.text_document.uri.clone();
        let text = params.text_document.text.clone();
        self.open_documents.write().insert(
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
            let mut documents = self.open_documents.write();
            let document = documents.entry(uri.clone()).or_insert_with(|| {
                OpenDocument::live(&uri, String::new(), params.text_document.version)
            });
            if params.text_document.version <= document.version {
                return;
            }
            let document = Arc::make_mut(document);
            document.version = params.text_document.version;
            document.content_fingerprint = None;
            for change in params.content_changes {
                if let Err(error) = apply_text_document_change(&mut document.text, &change) {
                    errors.push(error);
                }
            }
            document.lines = text_positions::LineIndex::new(&document.text);
            document.tokens.take();
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
        if let Some(task) = self
            .diagnostic_tasks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&params.text_document.uri)
        {
            task.abort();
        }
        self.linked_document_prewarmer
            .cancel(&params.text_document.uri);
        self.open_documents
            .write()
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
                .get_mut(&params.text_document.uri)
            {
                let document = Arc::make_mut(document);
                document.text = text.clone();
                document.content_fingerprint = None;
                document.lines = text_positions::LineIndex::new(&text);
                document.tokens.take();
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
                .get(&params.text_document.uri)
                .map_or(i32::MIN, |document| document.version);
            self.schedule_linked_document_prewarm(
                params.text_document.uri.clone(),
                text.clone(),
                version,
            );
        }
        if let Some(path) = uri_to_path(&params.text_document.uri) {
            self.index_actor.refresh(vec![path], Vec::new());
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
                FileChangeType::DELETED => {
                    changed.retain(|p| p != &path);
                    deleted.push(path);
                }
                FileChangeType::CREATED | FileChangeType::CHANGED => {
                    deleted.retain(|p| p != &path);
                    changed.push(path);
                }
                _ => {}
            }
        }
        if !changed.is_empty() || !deleted.is_empty() {
            self.navigation_cache.write().await.clear();
            self.index_actor.refresh(changed, deleted);
        }
    }

    async fn code_action(&self, params: CodeActionParams) -> Result<Option<CodeActionResponse>> {
        let _trace = RequestTrace::file("codeAction", &params.text_document.uri);
        let actions = code_actions_for_diagnostics(
            params.text_document.uri,
            params.context.diagnostics.as_slice(),
        );
        Ok((!actions.is_empty()).then_some(actions))
    }

    async fn document_link(&self, params: DocumentLinkParams) -> Result<Option<Vec<DocumentLink>>> {
        let _trace = RequestTrace::file("documentLink", &params.text_document.uri);
        let Some(document) = self.document_for_uri(&params.text_document.uri).await else {
            return Ok(Some(Vec::new()));
        };
        let Some(path) = uri_to_path(&params.text_document.uri) else {
            return Ok(Some(Vec::new()));
        };
        Ok(Some(sage_document_links(&document.text, &path)))
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let _trace = RequestTrace::at(
            "hover",
            &params.text_document_position_params.text_document.uri,
            params.text_document_position_params.position,
        );
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
        let index = self.index.load_full();
        let index_generation = index.generation();
        let key = navigation_query_cache_key(
            uri,
            &document,
            position,
            index_generation,
            NavigationQueryCacheFlavor::Hover,
        );
        let cached_query = self.navigation_cache.read().await.get(&key);
        let query = if let Some(query) = cached_query {
            drop(index);
            query
        } else {
            let snapshot = document.clone();
            let query = run_blocking(self.query_slots.clone(), move || {
                index.query_source_at_with_features(
                    &path,
                    &snapshot.text,
                    query_position,
                    None,
                    QueryFeatures::hover(),
                )
            })
            .await?;
            self.ensure_current_document(uri, &document)?;
            self.navigation_cache.write().await.insert(
                navigation_query_cache_key(
                    uri,
                    &document,
                    position,
                    index_generation,
                    NavigationQueryCacheFlavor::Hover,
                ),
                query.clone(),
            );
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
        let _trace = RequestTrace::at(
            "definition",
            &params.text_document_position_params.text_document.uri,
            params.text_document_position_params.position,
        );
        self.goto_definition_response(params).await
    }

    async fn goto_declaration(
        &self,
        params: GotoDeclarationParams,
    ) -> Result<Option<GotoDeclarationResponse>> {
        let _trace = RequestTrace::at(
            "declaration",
            &params.text_document_position_params.text_document.uri,
            params.text_document_position_params.position,
        );
        self.goto_declaration_response(params).await
    }

    async fn goto_type_definition(
        &self,
        params: GotoTypeDefinitionParams,
    ) -> Result<Option<GotoTypeDefinitionResponse>> {
        let _trace = RequestTrace::at(
            "typeDefinition",
            &params.text_document_position_params.text_document.uri,
            params.text_document_position_params.position,
        );
        self.goto_type_definition_response(params).await
    }

    async fn goto_implementation(
        &self,
        params: GotoImplementationParams,
    ) -> Result<Option<GotoImplementationResponse>> {
        let _trace = RequestTrace::at(
            "implementation",
            &params.text_document_position_params.text_document.uri,
            params.text_document_position_params.position,
        );
        self.goto_implementation_response(params).await
    }

    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        let _trace = RequestTrace::at(
            "completion",
            &params.text_document_position.text_document.uri,
            params.text_document_position.position,
        );
        let uri = &params.text_document_position.text_document.uri;
        if let (Some(document), Some(_path)) = (self.document_for_uri(uri).await, uri_to_path(uri))
        {
            let Some(query_position) =
                query_position_from_lsp(&document.text, params.text_document_position.position)
            else {
                return Ok(Some(CompletionResponse::Array(Vec::new())));
            };
            let index = self.index.load_full();
            let snapshot = document.clone();
            let items = run_blocking(self.query_slots.clone(), move || {
                index
                    .completion_items_at_source(&snapshot.text, query_position, 100)
                    .into_iter()
                    .map(query_completion_item)
                    .collect()
            })
            .await?;
            self.ensure_current_document(uri, &document)?;
            return Ok(Some(CompletionResponse::Array(items)));
        }
        let prefix = self
            .prefix_at_uri_position(
                &params.text_document_position.text_document.uri,
                params.text_document_position.position,
            )
            .await
            .unwrap_or_default();
        let index = self.index.load_full();
        let items = index
            .symbols_with_prefix(&prefix, 100)
            .into_iter()
            .map(completion_item)
            .collect();
        Ok(Some(CompletionResponse::Array(items)))
    }

    async fn completion_resolve(&self, mut item: CompletionItem) -> Result<CompletionItem> {
        let _trace = RequestTrace::start("completionItem/resolve", item.label.clone());
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
            .load_full()
            .documentation_for_symbol_with_module(name, module_hint);
        if let Some(documentation) = documentation {
            item.documentation = Some(completion_documentation(hover_markdown_for_documentation(
                &documentation,
            )));
        }
        Ok(item)
    }

    async fn references(&self, params: ReferenceParams) -> Result<Option<Vec<Location>>> {
        let _trace = RequestTrace::at(
            "references",
            &params.text_document_position.text_document.uri,
            params.text_document_position.position,
        );
        self.references_response(params).await
    }

    async fn document_highlight(
        &self,
        params: DocumentHighlightParams,
    ) -> Result<Option<Vec<DocumentHighlight>>> {
        let _trace = RequestTrace::at(
            "documentHighlight",
            &params.text_document_position_params.text_document.uri,
            params.text_document_position_params.position,
        );
        self.document_highlight_response(params).await
    }

    async fn rename(&self, params: RenameParams) -> Result<Option<WorkspaceEdit>> {
        let _trace = RequestTrace::at(
            "rename",
            &params.text_document_position.text_document.uri,
            params.text_document_position.position,
        );
        self.rename_response(params).await
    }

    async fn prepare_rename(
        &self,
        params: TextDocumentPositionParams,
    ) -> Result<Option<PrepareRenameResponse>> {
        let _trace = RequestTrace::at("prepareRename", &params.text_document.uri, params.position);
        self.prepare_rename_response(params).await
    }

    async fn signature_help(&self, params: SignatureHelpParams) -> Result<Option<SignatureHelp>> {
        let _trace = RequestTrace::at(
            "signatureHelp",
            &params.text_document_position_params.text_document.uri,
            params.text_document_position_params.position,
        );
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
            function_call_context_at_position(&document.text, position.line, position.character)
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
            let Some(call) = fallback else {
                return Ok(None);
            };
            let index = self.index.load_full();
            let target =
                index.query_source_at_navigation(&path, &document.text, call.name_position);
            let confirmed = (target.resolution_confidence.as_deref() == Some("high"))
                .then_some(target.signature)
                .flatten()
                .map(|signature| (signature.label, signature.documentation));
            let selected = confirmed.or_else(|| {
                if call.qualified {
                    return None;
                }
                let symbol = index.resolve_symbol(&call.name, Some(module_name_for_path(&path)))?;
                Some((symbol.signature.clone()?, symbol.docstring))
            });
            let Some((signature, documentation)) = selected else {
                return Ok(None);
            };
            let active_parameter = call
                .keyword
                .as_deref()
                .and_then(|keyword| active_parameter_for_keyword(&signature, keyword))
                .unwrap_or(call.active_parameter);
            return Ok(Some(SignatureHelp {
                signatures: vec![signature_information(
                    signature,
                    documentation,
                    active_parameter,
                )],
                active_signature: Some(0),
                active_parameter: Some(active_parameter),
            }));
        };
        if signature.label.is_empty() {
            return Ok(None);
        }
        let active_parameter = fallback
            .as_ref()
            .and_then(|call| call.keyword.as_deref())
            .and_then(|keyword| active_parameter_for_keyword(&signature.label, keyword))
            .unwrap_or(signature.active_parameter);
        Ok(Some(SignatureHelp {
            signatures: vec![signature_information(
                signature.label,
                signature.documentation,
                active_parameter,
            )],
            active_signature: Some(0),
            active_parameter: Some(active_parameter),
        }))
    }

    async fn prepare_call_hierarchy(
        &self,
        params: CallHierarchyPrepareParams,
    ) -> Result<Option<Vec<CallHierarchyItem>>> {
        let _trace = RequestTrace::at(
            "prepareCallHierarchy",
            &params.text_document_position_params.text_document.uri,
            params.text_document_position_params.position,
        );
        let uri = &params.text_document_position_params.text_document.uri;
        let Some(document) = self.document_for_uri(uri).await else {
            return Ok(Some(Vec::new()));
        };
        let Some(path) = uri_to_path(uri) else {
            return Ok(Some(Vec::new()));
        };
        let position = params.text_document_position_params.position;
        let may_fallback_to_enclosing =
            may_fallback_to_enclosing_call_hierarchy(&document.text, position);
        if let Some(item) =
            call_hierarchy_item_for_local_symbol_at_position(uri, &path, &document.text, position)
        {
            return Ok(Some(vec![item]));
        }
        let query = self
            .navigation_query_for_document(uri, &document, &path, position)
            .await;
        if let Some(definition) = high_confidence_call_hierarchy_definition(&query) {
            return Ok(Some(
                self.call_hierarchy_item_for_definition(definition)
                    .await
                    .into_iter()
                    .collect(),
            ));
        }
        if !may_fallback_to_enclosing {
            return Ok(Some(Vec::new()));
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
        let _trace = RequestTrace::start("incomingCalls", params.item.name.clone());
        let mut calls = Vec::new();
        let mut contexts: HashMap<Uri, CallHierarchySourceContext> = HashMap::new();
        let Some(target) = self
            .resolved_reference_target_at(&params.item.uri, params.item.selection_range.start)
            .await
        else {
            return Ok(Some(Vec::new()));
        };
        for location in self
            .reference_locations(&target, false, ReferenceCollectionMode::References)
            .await?
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
        let _trace = RequestTrace::start("outgoingCalls", params.item.name.clone());
        let Some(path) = uri_to_path(&params.item.uri) else {
            return Ok(Some(Vec::new()));
        };
        let Some(document) = self.document_for_uri(&params.item.uri).await else {
            return Ok(Some(Vec::new()));
        };
        let resolved_calls = {
            let index = self.index.load_full();
            resolve_outgoing_calls(
                &index,
                &path,
                &document.text,
                params.item.range,
                &params.item.name,
            )
        };
        let mut calls = Vec::new();
        for resolved in resolved_calls {
            let to = self
                .call_hierarchy_item_for_definition(&resolved.definition)
                .await;
            let Some(to) = to else {
                continue;
            };
            push_outgoing_call(&mut calls, to, resolved.from_range);
        }
        Ok(Some(calls))
    }

    async fn inlay_hint(&self, params: InlayHintParams) -> Result<Option<Vec<InlayHint>>> {
        let _trace = RequestTrace::file("inlayHint", &params.text_document.uri);
        let Some(document) = self
            .open_documents
            .read()
            .get(&params.text_document.uri)
            .cloned()
        else {
            return Ok(Some(Vec::new()));
        };
        Ok(Some(sage_inlay_hints(&document.text, params.range)))
    }

    async fn folding_range(&self, params: FoldingRangeParams) -> Result<Option<Vec<FoldingRange>>> {
        let _trace = RequestTrace::file("foldingRange", &params.text_document.uri);
        let Some(document) = self
            .open_documents
            .read()
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
        let _trace = RequestTrace::file("selectionRange", &params.text_document.uri);
        let Some(document) = self
            .open_documents
            .read()
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
        let _trace = RequestTrace::file("documentSymbol", &params.text_document.uri);
        let Some(document) = self
            .open_documents
            .read()
            .get(&params.text_document.uri)
            .cloned()
        else {
            return Ok(Some(DocumentSymbolResponse::Flat(Vec::new())));
        };
        let path = uri_to_path(&params.text_document.uri)
            .unwrap_or_else(|| PathBuf::from("document.sage"));
        let snapshot = document.clone();
        let symbols = run_blocking(self.query_slots.clone(), move || {
            let parsed = parse_source(module_name_for_path(&path), &path, &snapshot.text);
            document_symbols_for_source(&snapshot.text, &parsed.symbols)
        })
        .await?;
        self.ensure_current_document(&params.text_document.uri, &document)?;
        Ok(Some(DocumentSymbolResponse::Nested(symbols)))
    }

    async fn symbol(
        &self,
        params: WorkspaceSymbolParams,
    ) -> Result<Option<WorkspaceSymbolResponse>> {
        let _trace = RequestTrace::start("workspaceSymbol", params.query.clone());
        let limit = self.analysis_mode.read().await.workspace_symbol_limit();
        Ok(Some(WorkspaceSymbolResponse::Flat(
            self.workspace_symbol_information(&params.query, limit)
                .await,
        )))
    }

    async fn semantic_tokens_full(
        &self,
        params: SemanticTokensParams,
    ) -> Result<Option<SemanticTokensResult>> {
        let _trace = RequestTrace::file("semanticTokens/full", &params.text_document.uri);
        let Some(document) = self
            .open_documents
            .read()
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
            data: self
                .document_tokens(&params.text_document.uri, document)
                .await?,
        })))
    }

    async fn semantic_tokens_range(
        &self,
        params: SemanticTokensRangeParams,
    ) -> Result<Option<SemanticTokensRangeResult>> {
        let _trace = RequestTrace::file("semanticTokens/range", &params.text_document.uri);
        let Some(document) = self
            .open_documents
            .read()
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
            data: tokens_for_range(
                &self
                    .document_tokens(&params.text_document.uri, document)
                    .await?,
                params.range,
            ),
        })))
    }

    async fn execute_command(&self, params: ExecuteCommandParams) -> Result<Option<Value>> {
        let _trace = RequestTrace::start("executeCommand", params.command.clone());
        match params.command.as_str() {
            COMMAND_INDEX_STATUS => Ok(Some(self.index_status_payload().await)),
            COMMAND_DOCS_STATUS => Ok(Some(self.docs_status_payload().await)),
            COMMAND_REBUILD_INDEX => {
                self.index_actor.rebuild();
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
    async fn publish_diagnostics_for_text(&self, uri: Uri, _text: String) {
        let Some(path) = uri_to_path(&uri) else {
            return;
        };
        let Some(document) = self.open_documents.read().get(&uri).cloned() else {
            return;
        };
        let enabled = *self.diagnostics_enabled.read().await;
        let documents = self.open_documents.clone();
        let client = self.client.clone();
        let slots = self.query_slots.clone();
        let index = self.index.load_full();
        let stopped = self.shutting_down.clone();
        let task_uri = uri.clone();
        let task = tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            let snapshot = document.clone();
            let diagnostics = run_blocking(slots, move || {
                if !enabled {
                    return Vec::new();
                }
                index
                    .diagnostics_for_source(&path, &snapshot.text)
                    .into_iter()
                    .map(|diagnostic| Diagnostic {
                        range: lsp_range_for_text(&snapshot.text, &diagnostic.range),
                        severity: Some(diagnostic_severity(&diagnostic.severity)),
                        code: Some(NumberOrString::String(diagnostic.code)),
                        source: Some("sage-ls".into()),
                        message: diagnostic.message,
                        ..Diagnostic::default()
                    })
                    .collect()
            })
            .await;
            let current = documents
                .read()
                .get(&task_uri)
                .is_some_and(|current| Arc::ptr_eq(current, &document));
            if current && !stopped.load(Ordering::Acquire) {
                if let Ok(diagnostics) = diagnostics {
                    client
                        .publish_diagnostics(task_uri, diagnostics, Some(document.version))
                        .await;
                }
            }
        });
        let mut tasks = self
            .diagnostic_tasks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(previous) = tasks.insert(uri, task.abort_handle()) {
            previous.abort();
        }
    }

    fn ensure_current_document(&self, uri: &Uri, snapshot: &Arc<OpenDocument>) -> Result<()> {
        if snapshot.content_fingerprint.is_none()
            && !self
                .open_documents
                .read()
                .get(uri)
                .is_some_and(|current| Arc::ptr_eq(current, snapshot))
        {
            return Err(tower_lsp_server::jsonrpc::Error::content_modified());
        }
        Ok(())
    }

    async fn document_tokens(
        &self,
        uri: &Uri,
        document: Arc<OpenDocument>,
    ) -> Result<Vec<SemanticToken>> {
        let snapshot = document.clone();
        let tokens = run_blocking(self.query_slots.clone(), move || {
            snapshot
                .tokens
                .get_or_init(|| encode_semantic_tokens(&snapshot.text))
                .clone()
        })
        .await?;
        self.ensure_current_document(uri, &document)?;
        Ok(tokens)
    }

    async fn document_for_uri(&self, uri: &Uri) -> Option<Arc<OpenDocument>> {
        let documents = self.open_documents.read();
        if let Some(live) = live_document_for_uri_or_path(&documents, uri) {
            return Some(live.document);
        }
        drop(documents);
        let path = uri_to_path(uri)?;
        let text = std::fs::read_to_string(path).ok()?;
        Some(OpenDocument::on_disk(uri, text))
    }

    async fn call_hierarchy_item_for_definition(
        &self,
        definition: &QueryDefinition,
    ) -> Option<CallHierarchyItem> {
        let live = { live_document_for_path(&self.open_documents.read(), &definition.path) };
        if let Some(live) = live {
            return call_hierarchy_item_for_local_definition(
                &live.uri,
                &live.path,
                &live.document.text,
                definition,
            );
        }
        let uri = Uri::from_file_path(&definition.path)?;
        if let Ok(text) = std::fs::read_to_string(&definition.path) {
            if let Some(item) =
                call_hierarchy_item_for_local_definition(&uri, &definition.path, &text, definition)
            {
                return Some(item);
            }
        }
        let location = self.location_for_query_definition(definition).await?;
        Some(call_hierarchy_item_from_definition(
            definition,
            location.uri,
            location.range,
        ))
    }

    fn schedule_linked_document_prewarm(&self, uri: Uri, text: String, version: i32) {
        self.linked_document_prewarmer.schedule(
            Arc::clone(&self.index),
            self.index_actor.clone(),
            Arc::clone(&self.shutting_down),
            uri,
            text,
            version,
        );
    }

    async fn text_for_uri_or_file(&self, uri: &Uri) -> Option<String> {
        let documents = self.open_documents.read();
        if let Some(live) = live_document_for_uri_or_path(&documents, uri) {
            return Some(live.document.text.clone());
        }
        drop(documents);
        let path = uri_to_path(uri)?;
        std::fs::read_to_string(path).ok()
    }

    async fn prefix_at_uri_position(&self, uri: &Uri, position: Position) -> Option<String> {
        let document = self.document_for_uri(uri).await?;
        current_prefix(&document.text, position)
    }

    async fn documentation_payload(&self, payload: Value) -> Option<Value> {
        let explicit_symbol = payload
            .get("symbol")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|symbol| !symbol.is_empty())
            .map(str::to_string);
        let position_context = self.documentation_position_context(&payload).await;
        let position_symbol = position_context.as_ref().and_then(|context| {
            word_at_position(
                &context.text,
                lsp_position_for_byte_column(
                    &context.text,
                    context.position.line,
                    context.position.character,
                ),
            )
            .map(|(word, _)| word)
        });
        let symbol = explicit_symbol.clone().or(position_symbol)?;
        let preferred_source = *self.docs_preferred_source.read().await;
        let record = if let Some(context) = &position_context {
            let index = self.index.load_full();
            if let Some(selected) = &explicit_symbol {
                index
                    .query_source_symbol(
                        &context.path,
                        &context.text,
                        selected,
                        None,
                        None,
                        Vec::new(),
                    )
                    .documentation
            } else {
                documentation_record_for_source_position(
                    &index,
                    &context.path,
                    &context.text,
                    context.position,
                )
            }
        } else {
            None
        };
        // Resolve imported aliases using the indexed name while preserving the
        // position-specific documentation as the static fallback.
        let lookup_symbol = record
            .as_ref()
            .map_or(symbol.as_str(), |record| record.name.as_str())
            .to_string();
        let record = self
            .documentation_record_for_symbol(&lookup_symbol, preferred_source, record)
            .await?;
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
        let uri = parse_uri(uri)?;
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
        let uri = parse_uri(uri)?;
        let path = uri_to_path(&uri)?;
        let document = self.document_for_uri(&uri).await?;
        let rename_to = payload.get("renameTo").and_then(Value::as_str);
        let diagnostics_enabled = *self.diagnostics_enabled.read().await;
        let features = query_features_from_payload(&payload, diagnostics_enabled);
        let diagnostics = if features.diagnostics {
            let index = self.index.load_full();
            index.diagnostics_for_source(&path, &document.text)
        } else {
            Vec::new()
        };
        let position = payload
            .get("position")
            .and_then(|position| {
                Some(Position::new(
                    position.get("line")?.as_u64()? as u32,
                    position.get("character")?.as_u64()? as u32,
                ))
            })
            .and_then(|position| query_position_from_lsp(&document.text, position));
        let mut query = if let Some(position) = position {
            self.index.load_full().query_source_at_with_features(
                &path,
                &document.text,
                position,
                rename_to,
                features,
            )
        } else if let Some(symbol) = payload.get("symbol").and_then(Value::as_str) {
            self.index.load_full().query_source_symbol_with_options(
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
            return None;
        };
        self.enhance_query_with_runtime_docs(&mut query).await;
        serde_json::to_value(query).ok()
    }

    async fn docs_status_payload(&self) -> Value {
        let preferred_source = self.docs_preferred_source.read().await.as_str().to_string();
        let mut status = {
            let index = self.index.load_full();
            index.docs_status()
        };
        status.preferred_source = preferred_source;
        let status = self.runtime_docs.status(status).await;
        serde_json::to_value(status).unwrap_or_else(|_| json!({}))
    }

    async fn persist_runtime_documentation(&self, symbol: &str, record: &DocumentationRecord) {
        let index = self.index.load_full();
        let _ = index.write_runtime_documentation(symbol, record);
    }

    async fn documentation_record_for_symbol(
        &self,
        symbol: &str,
        preferred_source: DocumentationPreferredSource,
        source_record: Option<DocumentationRecord>,
    ) -> Option<DocumentationRecord> {
        let static_record = match source_record {
            Some(record) => Some(record),
            None => self.index.load_full().documentation_for_symbol(symbol),
        };
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
        if query.definition.is_none() && !query.definition_candidates.is_empty() {
            return None;
        }
        let symbol = runtime_docs_symbol_for_query(query, preferred_source)?;
        let record = self.runtime_docs.cached(symbol);
        if record.is_none() {
            self.runtime_docs.prefetch(symbol);
            if let Some(placeholder) = query
                .documentation
                .as_ref()
                .filter(|record| is_runtime_placeholder_documentation(record))
            {
                let mut pending = placeholder.clone();
                pending.docstring = Some(self.runtime_docs.hover_status_message());
                return Some(pending);
            }
        }
        record
    }

    async fn enhance_query_with_runtime_docs(&self, query: &mut QueryResult) {
        if query.definition.is_none() && !query.definition_candidates.is_empty() {
            return;
        }
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

fn may_fallback_to_enclosing_call_hierarchy(text: &str, position: Position) -> bool {
    word_at_position(text, position).is_none()
}

#[cfg(test)]
mod tests;

/// Absolute test path for `unix` (`/workspace/a.py`) that is also absolute on Windows (`C:\workspace\a.py`).
#[cfg(test)]
pub(crate) fn test_path(unix: &str) -> std::path::PathBuf {
    if cfg!(windows) {
        std::path::PathBuf::from(format!("C:{}", unix.replace('/', "\\")))
    } else {
        std::path::PathBuf::from(unix)
    }
}
