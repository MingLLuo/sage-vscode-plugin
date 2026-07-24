#![allow(deprecated)]

mod analysis_mode;
mod call_hierarchy;
mod document_links;
mod editor_features;
mod index_jobs;
mod initialization;
mod linked_document_prewarm;
mod navigation;
mod open_documents;
mod references;
mod runtime_docs;
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
use document_links::sage_document_links;
#[cfg(test)]
use editor_features::sage_selection_range;
use editor_features::{sage_folding_ranges, sage_inlay_hints, sage_selection_ranges};
#[cfg(test)]
use index_jobs::index_job_result_is_current;
use initialization::{
    default_excludes, parse_initialization_options, source_roots_from_options,
    workspace_folders_from_options, DocumentationPreferredSource,
};
#[cfg(test)]
use linked_document_prewarm::import_modules_for_prewarm;
use linked_document_prewarm::LinkedDocumentPrewarmer;
#[cfg(test)]
use navigation::{
    live_definition_range, navigation_response_for_links,
    should_defer_python_import_definition_to_python_provider, NavigationQueryCacheKey,
};
use navigation::{navigation_query_cache_key, NavigationLinkSupport, NavigationQueryCache};
#[cfg(test)]
use open_documents::source_text_fingerprint;
use open_documents::{
    live_document_for_path, live_document_for_uri_or_path, uri_to_path, OpenDocument,
    OpenDocumentMap,
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
use source_symbols::{document_symbols_for_source, module_name_for_path};
#[cfg(test)]
use std::collections::BTreeSet;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;
use text_positions::{
    apply_text_document_change, byte_offset_to_utf16_character, is_word_byte, line_byte_bounds,
    lsp_position_for_byte_column, lsp_range_for_text, query_position_from_lsp,
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

struct Backend {
    client: Client,
    index: Arc<RwLock<WorkspaceIndex>>,
    open_documents: Arc<RwLock<OpenDocumentMap>>,
    navigation_cache: Arc<RwLock<NavigationQueryCache>>,
    navigation_link_support: Arc<RwLock<NavigationLinkSupport>>,
    analysis_mode: Arc<RwLock<AnalysisMode>>,
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
        navigation_link_support: Arc::new(RwLock::new(NavigationLinkSupport::default())),
        analysis_mode: Arc::new(RwLock::new(AnalysisMode::default())),
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
        self.goto_definition_response(params).await
    }

    async fn goto_declaration(
        &self,
        params: GotoDeclarationParams,
    ) -> Result<Option<GotoDeclarationResponse>> {
        self.goto_declaration_response(params).await
    }

    async fn goto_type_definition(
        &self,
        params: GotoTypeDefinitionParams,
    ) -> Result<Option<GotoTypeDefinitionResponse>> {
        self.goto_type_definition_response(params).await
    }

    async fn goto_implementation(
        &self,
        params: GotoImplementationParams,
    ) -> Result<Option<GotoImplementationResponse>> {
        self.goto_implementation_response(params).await
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
        self.references_response(params).await
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
        self.rename_response(params).await
    }

    async fn prepare_rename(
        &self,
        params: TextDocumentPositionParams,
    ) -> Result<Option<PrepareRenameResponse>> {
        self.prepare_rename_response(params).await
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
        let Some(document) = self.document_for_uri(&params.item.uri).await else {
            return Ok(Some(Vec::new()));
        };
        let resolved_calls = {
            let index = self.index.read().await;
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
        let limit = self.analysis_mode.read().await.workspace_symbol_limit();
        Ok(Some(
            self.workspace_symbol_information(&params.query, limit)
                .await,
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

    async fn call_hierarchy_item_for_definition(
        &self,
        definition: &QueryDefinition,
    ) -> Option<CallHierarchyItem> {
        let documents = self.open_documents.read().await;
        if let Some(live) = live_document_for_path(&documents, &definition.path) {
            return call_hierarchy_item_for_local_definition(
                &live.uri,
                &live.path,
                &live.document.text,
                definition,
            );
        }
        drop(documents);
        let uri = Url::from_file_path(&definition.path).ok()?;
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
        let symbol = position_symbol.or(explicit_symbol.clone())?;
        let preferred_source = *self.docs_preferred_source.read().await;
        let mut record = if let Some(context) = &position_context {
            let index = self.index.read().await;
            documentation_record_for_source_position(
                &index,
                &context.path,
                &context.text,
                context.position,
            )
        } else {
            None
        };
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
            self.index.read().await.query_source_at_with_features(
                &path,
                &document.text,
                position,
                rename_to,
                features,
            )
        } else if let Some(symbol) = payload.get("symbol").and_then(Value::as_str) {
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
            return None;
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
        if query.definition.is_none() && !query.definition_candidates.is_empty() {
            return None;
        }
        let symbol = runtime_docs_symbol_for_query(query, preferred_source)?;
        let record = self.runtime_docs.cached(symbol);
        if record.is_none() {
            self.runtime_docs.prefetch(symbol);
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

fn may_fallback_to_enclosing_call_hierarchy(text: &str, position: Position) -> bool {
    word_at_position(text, position).is_none()
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
