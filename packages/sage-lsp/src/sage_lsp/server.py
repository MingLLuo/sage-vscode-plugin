from __future__ import annotations

from pathlib import Path
import re
import threading
from typing import Any, Dict, Optional, Tuple, TypedDict

from lsprotocol.types import (
    CodeAction,
    CodeActionKind,
    CodeActionParams,
    CompletionList,
    CompletionItem,
    CompletionParams,
    DidCloseTextDocumentParams,
    DidChangeWatchedFilesParams,
    Diagnostic,
    DiagnosticSeverity,
    DidChangeTextDocumentParams,
    DidChangeConfigurationParams,
    DidOpenTextDocumentParams,
    DidSaveTextDocumentParams,
    DefinitionParams,
    DocumentSymbolParams,
    FileChangeType,
    Hover,
    HoverParams,
    InitializeParams,
    InitializeResult,
    InitializedParams,
    Location,
    MarkupContent,
    MarkupKind,
    Position,
    PublishDiagnosticsParams,
    ReferenceParams,
    Range,
    RenameParams,
    ServerCapabilities,
    SignatureHelp,
    SignatureHelpParams,
    SignatureInformation,
    SaveOptions,
    SemanticTokens,
    SemanticTokensParams,
    TextDocumentSyncKind,
    TextDocumentSyncOptions,
    TextEdit,
    WorkspaceEdit,
    WorkspaceSymbolParams,
    ParameterInformation,
)
from pygls.lsp.server import LanguageServer

from .environment import SageEnvironment
from .index import DocumentationResult, WorkspaceIndex, iter_identifier_ranges, path_from_uri, split_docstring
from .model import ModuleRecord
from .runtime_introspection import RuntimeIntrospector, RuntimeSymbolResult


class SageLanguageServer(LanguageServer):
    def __init__(self) -> None:
        super().__init__("sage-lsp", "0.3.0")
        self.environment = SageEnvironment()
        self.workspace_index: Optional[WorkspaceIndex] = None
        self.runtime_introspector = RuntimeIntrospector(command=None, enabled=False)
        self.documentation_cache: dict[tuple[str, str, str], DocumentationResult] = {}
        self.definition_cache: dict[tuple[str, str, str], Optional[Location]] = {}
        self.index_generation = 0


class ResolvedRequestSymbol(TypedDict):
    record: ModuleRecord
    name: str
    static_name: Optional[str]
    runtime_name: Optional[str]
    range: Range


PREWARM_CALL_RE = re.compile(r"\b(?P<name>[A-Za-z_][A-Za-z0-9_]*(?:\.[A-Za-z_][A-Za-z0-9_]*)*)\s*\(")
MAX_PREWARM_SYMBOLS = 8
SEMANTIC_TOKEN_TYPES = [
    "namespace",
    "type",
    "class",
    "function",
    "method",
    "variable",
    "parameter",
    "keyword",
    "decorator",
]
SEMANTIC_TOKEN_MODIFIERS = ["declaration", "readonly", "defaultLibrary"]
SEMANTIC_TYPE_INDEX = {name: index for index, name in enumerate(SEMANTIC_TOKEN_TYPES)}
SEMANTIC_MODIFIER_INDEX = {name: index for index, name in enumerate(SEMANTIC_TOKEN_MODIFIERS)}
SEMANTIC_NAMESPACE_NAMES = {
    "graphs",
    "toric_varieties",
    "simplicial_complexes",
    "matroids",
    "mq",
    "interacts",
    "plot3d",
}
SEMANTIC_READONLY_NAMES = {
    "ZZ",
    "QQ",
    "RR",
    "CC",
    "SR",
    "GF",
    "QQbar",
    "AA",
    "CDF",
    "RDF",
    "IF",
    "CIF",
    "Integers",
    "Rationals",
    "ComplexField",
    "RealField",
    "FiniteField",
    "pi",
    "e",
    "I",
    "oo",
    "Infinity",
}
SEMANTIC_TYPE_NAMES = {
    "PolynomialRing",
    "PowerSeriesRing",
    "LaurentSeriesRing",
    "FractionField",
    "QuadraticField",
    "CyclotomicField",
    "NumberField",
    "OrePolynomialRing",
    "MatrixSpace",
    "EllipticCurve",
    "Graph",
    "DiGraph",
    "FreeModule",
    "VectorSpace",
    "SimplicialComplex",
    "FilteredSimplicialComplex",
    "ChowGroup",
    "FreeAbelianMonoid",
    "ToricVariety",
    "Cone",
    "Fan",
    "Polyhedron",
    "LatticePolytope",
    "ToricPlotter",
    "CPRFanoToricVariety",
    "BooleanFunction",
    "SBox",
    "Partitions",
    "Permutations",
    "SymmetricGroup",
    "Compositions",
    "Combinations",
    "Subsets",
}
SEMANTIC_FUNCTION_NAMES = {
    "matrix",
    "vector",
    "identity_matrix",
    "diagonal_matrix",
    "block_matrix",
    "random_matrix",
    "var",
    "function",
    "factor",
    "factorial",
    "expand",
    "simplify",
    "integrate",
    "diff",
    "limit",
    "solve",
    "sqrt",
    "sin",
    "cos",
    "tan",
    "exp",
    "log",
    "plot",
    "parametric_plot",
    "implicit_plot",
    "list_plot",
    "line",
    "point",
    "circle",
    "polygon",
    "density_plot",
    "text3d",
    "sigma",
    "euler_phi",
    "next_prime",
    "is_prime",
    "cyclotomic_polynomial",
    "lazy_import",
    "lazy_attribute",
    "cached_method",
    "cached_function",
    "cached_in_parent_method",
    "cached_property",
    "abstract_method",
    "PetersenGraph",
    "CompleteGraph",
    "CycleGraph",
    "PathGraph",
}
SEMANTIC_DECORATOR_RE = re.compile(r"^\s*@(?P<name>[A-Za-z_][A-Za-z0-9_\.]*)", re.MULTILINE)
SEMANTIC_PREPARSE_RE = re.compile(
    r"(?P<parent>\b[A-Za-z_][A-Za-z0-9_]*\b)\.\<(?P<symbols>[A-Za-z_][A-Za-z0-9_]*(?:\s*,\s*[A-Za-z_][A-Za-z0-9_]*)*)\>"
)
SEMANTIC_CLASS_DEF_RE = re.compile(
    r"^(?P<indent>\s*)(?:class|cdef\s+class|cpdef\s+class)\s+(?P<name>[A-Za-z_][A-Za-z0-9_]*)",
    re.MULTILINE,
)
SEMANTIC_FUNCTION_DEF_RE = re.compile(
    r"^(?P<indent>\s*)(?:async\s+def|def|cpdef|cdef)(?:\s+(?:inline|api|public|readonly|nogil|gil|except|const|unsigned|signed|long|short|char|int|float|double|void|object|bint|size_t|Py_ssize_t|[A-Za-z_][A-Za-z0-9_\.\*\[\]]*))*\s+(?P<name>[A-Za-z_][A-Za-z0-9_]*)\s*\(",
    re.MULTILINE,
)


def create_server() -> SageLanguageServer:
    server = SageLanguageServer()

    @server.feature("initialize")
    def on_initialize(params: InitializeParams) -> InitializeResult:
        options = params.initialization_options if isinstance(params.initialization_options, dict) else None
        server.environment = SageEnvironment.from_initialize_options(options)
        _rebuild_index(server)
        return InitializeResult(
            server_info={"name": "sage-lsp", "version": "0.3.0"},
            capabilities=ServerCapabilities(
                text_document_sync=TextDocumentSyncOptions(
                    open_close=True,
                    change=TextDocumentSyncKind.Incremental,
                    save=SaveOptions(include_text=False),
                ),
                hover_provider=True,
                definition_provider=True,
                references_provider=True,
                rename_provider=True,
                document_symbol_provider=True,
                workspace_symbol_provider=True,
                code_action_provider=True,
                signature_help_provider={"triggerCharacters": ["(", ","]},
                completion_provider={"triggerCharacters": [".", "("], "resolveProvider": False},
                semantic_tokens_provider={
                    "legend": {
                        "tokenTypes": SEMANTIC_TOKEN_TYPES,
                        "tokenModifiers": SEMANTIC_TOKEN_MODIFIERS,
                    },
                    "full": True,
                },
            ),
        )

    @server.feature("initialized")
    def on_initialized(params: InitializedParams) -> None:
        del params

    @server.feature("workspace/didChangeConfiguration")
    def on_did_change_configuration(params: DidChangeConfigurationParams) -> None:
        del params

    @server.feature("workspace/didChangeWatchedFiles")
    def on_did_change_watched_files(params: DidChangeWatchedFilesParams) -> None:
        _apply_workspace_file_changes(
            server,
            [(change.uri, change.type) for change in params.changes],
        )
        _clear_all_request_caches(server)

    @server.feature("textDocument/hover")
    def on_hover(params: HoverParams) -> Optional[Hover]:
        resolved = _resolve_request_symbol(server, params.text_document.uri, params.position)
        if resolved is None:
            return None

        documentation = _documentation_for_resolved(server, resolved, uri=params.text_document.uri)
        if documentation is None:
            return None

        return Hover(
            contents=_hover_markup_content(server, documentation),
            range=resolved["range"],
        )

    @server.feature("textDocument/definition")
    def on_definition(params: DefinitionParams) -> Optional[Location]:
        resolved = _resolve_request_symbol(server, params.text_document.uri, params.position)
        if resolved is None:
            return None
        return _definition_for_resolved(server, resolved, uri=params.text_document.uri)

    @server.feature("textDocument/completion")
    def on_completion(params: CompletionParams) -> CompletionList:
        if server.workspace_index is None:
            return CompletionList(is_incomplete=False, items=[])

        record, text = _record_for_uri(server, params.text_document.uri)
        if record is None:
            return CompletionList(is_incomplete=False, items=[])

        dotted_target, prefix = completion_target_at_position(
            text or record.source,
            params.position.line,
            params.position.character,
        )
        if dotted_target and server.workspace_index is not None:
            items = [
                _as_completion_item(item)
                for item in server.workspace_index.member_completion_items(record, dotted_target, prefix)
            ]
            return CompletionList(is_incomplete=False, items=items)

        items = [_as_completion_item(item) for item in server.workspace_index.completion_items(record, prefix)]
        return CompletionList(is_incomplete=False, items=items)

    @server.feature("textDocument/signatureHelp")
    def on_signature_help(params: SignatureHelpParams) -> Optional[SignatureHelp]:
        record, text = _record_for_uri(server, params.text_document.uri)
        if record is None:
            return None

        call_name, active_parameter = call_expression_at_position(
            text or record.source,
            params.position.line,
            params.position.character,
        )
        if call_name is None:
            return None

        runtime_symbol = server.runtime_introspector.lookup(call_name)
        if runtime_symbol is None:
            return None

        signature = _signature_help_from_runtime_symbol(runtime_symbol, active_parameter)
        if signature is None:
            return None
        return signature

    @server.feature("textDocument/semanticTokens/full")
    def on_semantic_tokens(params: SemanticTokensParams) -> SemanticTokens:
        record, text = _record_for_uri(server, params.text_document.uri)
        if record is None:
            return SemanticTokens(data=[])
        return SemanticTokens(data=_encode_semantic_tokens(record, text or record.source))

    @server.feature("textDocument/documentSymbol")
    def on_document_symbol(params: DocumentSymbolParams) -> list[dict[str, object]]:
        if server.workspace_index is None:
            return []
        record, _ = _record_for_uri(server, params.text_document.uri)
        if record is None:
            return []
        return server.workspace_index.document_symbols(record)

    @server.feature("workspace/symbol")
    def on_workspace_symbol(params: WorkspaceSymbolParams) -> list[dict[str, object]]:
        if server.workspace_index is None:
            return []
        return server.workspace_index.workspace_symbols(params.query)

    @server.feature("textDocument/references")
    def on_references(params: ReferenceParams) -> list[Location]:
        if server.workspace_index is None:
            return []
        resolved = _resolve_request_symbol(server, params.text_document.uri, params.position)
        if resolved is None:
            return []
        locations = server.workspace_index.reference_locations(
            resolved["record"],
            resolved["name"],
            include_declaration=params.context.include_declaration,
        )
        return [
            Location(uri=location["uri"], range=_as_lsp_range(location["range"]))
            for location in locations
        ]

    @server.feature("textDocument/rename")
    def on_rename(params: RenameParams) -> Optional[WorkspaceEdit]:
        if server.workspace_index is None:
            return None
        resolved = _resolve_request_symbol(server, params.text_document.uri, params.position)
        if resolved is None:
            return None
        changes = server.workspace_index.rename_edits(
            resolved["record"],
            resolved["name"],
            params.new_name,
        )
        if not changes:
            return None
        return WorkspaceEdit(
            changes={
                uri: [
                    TextEdit(range=_as_lsp_range(edit["range"]), new_text=edit["newText"])
                    for edit in edits
                ]
                for uri, edits in changes.items()
            }
        )

    @server.feature("textDocument/codeAction")
    def on_code_action(params: CodeActionParams) -> list[CodeAction]:
        if server.workspace_index is None:
            return []
        return _code_actions_for_request(server, params)

    @server.feature("textDocument/didOpen")
    def on_did_open(params: DidOpenTextDocumentParams) -> None:
        _cache_document_overlay(
            server,
            params.text_document.uri,
            params.text_document.text,
            params.text_document.language_id,
        )
        _publish_diagnostics(server, params.text_document.uri)
        _prewarm_request_caches(server, params.text_document.uri)

    @server.feature("textDocument/didChange")
    def on_did_change(params: DidChangeTextDocumentParams) -> None:
        _refresh_workspace_document_overlay(server, params.text_document.uri)
        _publish_diagnostics(server, params.text_document.uri)
        _clear_request_caches(server, params.text_document.uri)

    @server.feature("textDocument/didSave")
    def on_did_save(params: DidSaveTextDocumentParams) -> None:
        _refresh_saved_indexed_document(server, params.text_document.uri)
        _publish_diagnostics(server, params.text_document.uri)
        _clear_all_request_caches(server)

    @server.feature("textDocument/didClose")
    def on_did_close(params: DidCloseTextDocumentParams) -> None:
        if server.workspace_index is not None:
            server.workspace_index.drop_document(params.text_document.uri)
        _clear_request_caches(server, params.text_document.uri)
        _publish_empty_diagnostics(server, params.text_document.uri)

    @server.feature("sage/getDocumentation")
    def on_get_documentation(params: Dict[str, Any]) -> Optional[dict[str, object]]:
        text_document = params.get("textDocument")
        if not isinstance(text_document, dict):
            return None
        uri = text_document.get("uri")
        if not isinstance(uri, str):
            return None

        record, text = _record_for_uri(server, uri)
        if record is None:
            return None

        name = params.get("symbol")
        runtime_name: Optional[str] = name if isinstance(name, str) else None
        if not isinstance(name, str):
            position = params.get("position")
            if not isinstance(position, dict):
                return None
            line = position.get("line")
            character = position.get("character")
            if not isinstance(line, int) or not isinstance(character, int):
                return None
            _, static_name, runtime_name, _ = symbol_at_position(text or record.source, line, character)
            name = static_name or runtime_name
            if name is None:
                return None

        documentation = _documentation_for_request(server, record, name, runtime_name or name, uri=uri)
        return documentation.to_payload() if documentation is not None else None

    return server


def _rebuild_index(server: SageLanguageServer) -> None:
    source_roots = [coerce_path(entry) for entry in server.environment.workspace.source_roots if entry]
    source_roots.extend(coerce_path(entry) for entry in server.environment.analysis.extra_paths if entry)
    deduped_roots = list(dict.fromkeys(root.resolve() for root in source_roots))
    server.index_generation += 1
    generation = server.index_generation
    server.runtime_introspector = RuntimeIntrospector.from_environment(server.environment)

    workspace_index = WorkspaceIndex(
        source_roots=deduped_roots,
        excluded_globs=server.environment.workspace.excluded_globs,
        enable_pyx=server.environment.analysis.enable_pyx_parsing,
    )
    if workspace_index.hydrate_from_cache():
        server.workspace_index = workspace_index
        _clear_all_request_caches(server)
        _start_background_index_reconcile(server, deduped_roots, generation)
        return

    workspace_index.build()
    if server.index_generation == generation:
        server.workspace_index = workspace_index


def _start_background_index_reconcile(
    server: SageLanguageServer,
    source_roots: list[Path],
    generation: int,
) -> None:
    excluded_globs = server.environment.workspace.excluded_globs
    enable_pyx = server.environment.analysis.enable_pyx_parsing

    def _reconcile() -> None:
        refreshed_index = WorkspaceIndex(
            source_roots=source_roots,
            excluded_globs=excluded_globs,
            enable_pyx=enable_pyx,
        )
        refreshed_index.build()
        if server.index_generation != generation:
            return
        server.workspace_index = refreshed_index
        _clear_all_request_caches(server)

    threading.Thread(
        target=_reconcile,
        name="sage-lsp-index-reconcile",
        daemon=True,
    ).start()


def _record_for_uri(server: SageLanguageServer, uri: str) -> Tuple[Optional[ModuleRecord], Optional[str]]:
    if server.workspace_index is None:
        return None, None

    try:
        document = server.workspace.get_text_document(uri)
    except KeyError:
        document = None

    if document is not None:
        language_id = getattr(document, "language_id", "python")
        return server.workspace_index.parse_document(uri, document.source, language_id), document.source

    record = server.workspace_index.module_for_path(path_from_uri(uri))
    return record, record.source if record is not None else None


def _cache_document_overlay(
    server: SageLanguageServer,
    uri: str,
    source: str,
    language_id: str,
) -> Optional[ModuleRecord]:
    if server.workspace_index is None:
        return None
    return server.workspace_index.parse_document(uri, source, language_id)


def _refresh_workspace_document_overlay(
    server: SageLanguageServer,
    uri: str,
) -> Optional[ModuleRecord]:
    try:
        document = server.workspace.get_text_document(uri)
    except KeyError:
        return None

    return _cache_document_overlay(
        server,
        uri,
        document.source,
        getattr(document, "language_id", "python"),
    )


def _refresh_saved_indexed_document(
    server: SageLanguageServer,
    uri: str,
) -> Optional[ModuleRecord]:
    if server.workspace_index is None:
        return None
    return server.workspace_index.refresh_path(path_from_uri(uri))


def _apply_workspace_file_change(
    server: SageLanguageServer,
    uri: str,
    change_type: FileChangeType,
) -> None:
    if server.workspace_index is None:
        return

    path = path_from_uri(uri)
    if change_type is FileChangeType.Deleted:
        server.workspace_index.remove_path(path)
        _clear_request_caches(server, uri)
        _publish_empty_diagnostics(server, uri)
        return

    server.workspace_index.refresh_path(path)
    _clear_request_caches(server, uri)


def _apply_workspace_file_changes(
    server: SageLanguageServer,
    changes: list[tuple[str, FileChangeType]],
) -> None:
    if server.workspace_index is None:
        return

    normalized_changes = [
        (path_from_uri(uri), change_type is FileChangeType.Deleted)
        for uri, change_type in changes
    ]
    results = server.workspace_index.refresh_or_remove_paths(normalized_changes)

    for path, record in results.items():
        uri = path.as_uri()
        if record is None:
            _publish_empty_diagnostics(server, uri)
            continue
        _publish_diagnostics(server, uri)
        _clear_request_caches(server, uri)


def _publish_empty_diagnostics(server: SageLanguageServer, uri: str) -> None:
    server.text_document_publish_diagnostics(PublishDiagnosticsParams(uri=uri, diagnostics=[]))


def _publish_diagnostics(server: SageLanguageServer, uri: str) -> None:
    if server.workspace_index is None or not server.environment.analysis.enable_diagnostics:
        _publish_empty_diagnostics(server, uri)
        return

    record, _ = _record_for_uri(server, uri)
    if record is None:
        _publish_empty_diagnostics(server, uri)
        return

    diagnostics = [_as_lsp_diagnostic(entry) for entry in server.workspace_index.diagnostics_for_record(record)]
    server.text_document_publish_diagnostics(
        PublishDiagnosticsParams(uri=uri, diagnostics=diagnostics)
    )


def _prewarm_request_caches(server: SageLanguageServer, uri: str) -> None:
    _clear_request_caches(server, uri)
    record, text = _record_for_uri(server, uri)
    if record is None:
        return

    for static_name, runtime_name in _documentation_prewarm_candidates(record, text or record.source):
        _documentation_for_request(server, record, static_name, runtime_name, uri=uri)
        _definition_for_request(server, record, static_name, runtime_name, uri=uri)


def _code_actions_for_request(
    server: SageLanguageServer,
    params: CodeActionParams,
) -> list[CodeAction]:
    record, _ = _record_for_uri(server, params.text_document.uri)
    if record is None or server.workspace_index is None:
        return []

    diagnostics = list(params.context.diagnostics)
    if not diagnostics:
        diagnostics = [
            _as_lsp_diagnostic(entry)
            for entry in server.workspace_index.diagnostics_for_record(record)
            if _ranges_overlap(entry["range"], _as_dict_range(params.range))
        ]

    actions: list[CodeAction] = []
    for diagnostic in diagnostics:
        actions.extend(
            _code_actions_for_diagnostic(
                server,
                params.text_document.uri,
                diagnostic,
            )
        )
    return actions


def _code_actions_for_diagnostic(
    server: SageLanguageServer,
    uri: str,
    diagnostic: Diagnostic,
) -> list[CodeAction]:
    if diagnostic.code != "unresolved-import-name" or not isinstance(diagnostic.data, dict):
        return []

    target_name = diagnostic.data.get("targetName")
    alias = diagnostic.data.get("alias")
    existing_module = diagnostic.data.get("moduleName")
    if not isinstance(target_name, str) or not isinstance(alias, str):
        return []

    candidates = server.workspace_index.import_candidates(target_name, exclude_module=existing_module if isinstance(existing_module, str) else None)
    if not candidates:
        return []

    actions: list[CodeAction] = []
    for index, module_name in enumerate(candidates[:5]):
        import_text = f"from {module_name} import {target_name}"
        if alias != target_name:
            import_text += f" as {alias}"
        actions.append(
            CodeAction(
                title=f"Import '{target_name}' from '{module_name}'",
                kind=CodeActionKind.QuickFix,
                diagnostics=[diagnostic],
                is_preferred=index == 0,
                edit=WorkspaceEdit(
                    changes={
                        uri: [
                            TextEdit(
                                range=diagnostic.range,
                                new_text=import_text,
                            )
                        ]
                    }
                ),
            )
        )
    return actions


def _request_symbol_names(resolved: ResolvedRequestSymbol) -> tuple[str, str]:
    return (
        str(resolved.get("static_name") or resolved["name"]),
        str(resolved.get("runtime_name") or resolved["name"]),
    )


def _documentation_for_resolved(
    server: SageLanguageServer,
    resolved: ResolvedRequestSymbol,
    *,
    uri: Optional[str] = None,
) -> Optional[DocumentationResult]:
    static_name, runtime_name = _request_symbol_names(resolved)
    return _documentation_for_request(server, resolved["record"], static_name, runtime_name, uri=uri)


def _definition_for_resolved(
    server: SageLanguageServer,
    resolved: ResolvedRequestSymbol,
    *,
    uri: Optional[str] = None,
) -> Optional[Location]:
    static_name, runtime_name = _request_symbol_names(resolved)
    return _definition_for_request(server, resolved["record"], static_name, runtime_name, uri=uri)


def _hover_markup_content(
    server: SageLanguageServer,
    documentation: DocumentationResult,
) -> MarkupContent:
    parts = [documentation.detail]
    if server.environment.documentation.show_on_hover and documentation.docstring:
        parts.append(documentation.docstring)
    return MarkupContent(kind=MarkupKind.Markdown, value="\n\n".join(parts))


def _definition_for_request(
    server: SageLanguageServer,
    record: ModuleRecord,
    name: str,
    runtime_name: Optional[str] = None,
    uri: Optional[str] = None,
) -> Optional[Location]:
    cache_key = _request_cache_key(uri, name, runtime_name)
    if cache_key is not None and cache_key in server.definition_cache:
        return server.definition_cache[cache_key]

    symbol = (
        server.workspace_index.resolve_symbol(record, name)
        if server.workspace_index is not None
        else None
    )
    if symbol is not None:
        result = Location(uri=symbol.file_path.as_uri(), range=_as_lsp_range(symbol.source_range.to_lsp()))
        if cache_key is not None:
            server.definition_cache[cache_key] = result
        return result

    runtime_symbol = server.runtime_introspector.lookup(runtime_name or name)
    if runtime_symbol is None or runtime_symbol.file_path is None:
        if cache_key is not None:
            server.definition_cache[cache_key] = None
        return None

    result = _location_from_runtime_symbol(runtime_symbol)
    if cache_key is not None:
        server.definition_cache[cache_key] = result
    return result


def _documentation_for_request(
    server: SageLanguageServer,
    record: ModuleRecord,
    name: str,
    runtime_name: Optional[str] = None,
    uri: Optional[str] = None,
) -> Optional[DocumentationResult]:
    cache_key = _request_cache_key(uri, name, runtime_name)
    if cache_key is not None and cache_key in server.documentation_cache:
        return server.documentation_cache[cache_key]

    documentation = (
        server.workspace_index.documentation_for_symbol(record, name)
        if server.workspace_index is not None
        else None
    )
    runtime_symbol = server.runtime_introspector.lookup(runtime_name or name)
    if documentation is None:
        if runtime_symbol is None:
            return None
        result = _documentation_from_runtime_symbol(runtime_symbol)
        if cache_key is not None:
            server.documentation_cache[cache_key] = result
        return result
    if runtime_symbol is None:
        if cache_key is not None:
            server.documentation_cache[cache_key] = documentation
        return documentation
    result = _merge_documentation_with_runtime(documentation, runtime_symbol)
    if cache_key is not None:
        server.documentation_cache[cache_key] = result
    return result


def _documentation_from_runtime_symbol(symbol: RuntimeSymbolResult) -> DocumentationResult:
    summary, sections = split_docstring(symbol.docstring)
    uri = symbol.file_path.as_uri() if symbol.file_path is not None else ""
    source_marker = symbol.file_path.suffix.lstrip(".") if symbol.file_path is not None else "runtime"
    return DocumentationResult(
        name=symbol.name,
        kind=symbol.kind,
        module_name=symbol.module_name,
        uri=uri,
        detail=symbol.detail,
        summary=summary,
        docstring=symbol.docstring,
        markers=(
            f"kind:{symbol.kind}",
            f"module:{symbol.module_name}",
            f"source:{source_marker or 'runtime'}",
        ),
        sections=sections,
    )


def _merge_documentation_with_runtime(
    documentation: DocumentationResult,
    runtime_symbol: RuntimeSymbolResult,
) -> DocumentationResult:
    runtime_documentation = _documentation_from_runtime_symbol(runtime_symbol)
    detail = (
        runtime_documentation.detail
        if _should_prefer_runtime_detail(documentation, runtime_documentation)
        else documentation.detail
    )
    docstring = documentation.docstring or runtime_documentation.docstring
    summary, sections = split_docstring(docstring)

    return DocumentationResult(
        name=documentation.name,
        kind=_merged_symbol_kind(documentation.kind, runtime_documentation.kind),
        module_name=documentation.module_name or runtime_documentation.module_name,
        uri=documentation.uri or runtime_documentation.uri,
        detail=detail,
        summary=summary if docstring else documentation.summary or runtime_documentation.summary,
        docstring=docstring,
        markers=documentation.markers or runtime_documentation.markers,
        sections=sections if docstring else documentation.sections or runtime_documentation.sections,
    )


def _merged_symbol_kind(static_kind: str, runtime_kind: str) -> str:
    if static_kind == "variable" and runtime_kind != "variable":
        return runtime_kind
    return static_kind or runtime_kind


def _should_prefer_runtime_detail(
    documentation: DocumentationResult,
    runtime_documentation: DocumentationResult,
) -> bool:
    static_detail = documentation.detail.strip()
    runtime_detail = runtime_documentation.detail.strip()
    if not runtime_detail:
        return False
    if not static_detail:
        return True

    static_name = documentation.name.split(".")[-1]
    if static_detail in {
        f"{documentation.kind} {documentation.name}",
        f"{documentation.kind} {static_name}",
    }:
        return True

    if "(" in runtime_detail and "(" not in static_detail:
        return True
    return False


def _location_from_runtime_symbol(symbol: RuntimeSymbolResult) -> Location:
    line = max((symbol.line or 1) - 1, 0)
    return Location(
        uri=symbol.file_path.as_uri(),
        range=Range(
            start=Position(line=line, character=0),
            end=Position(line=line, character=0),
        ),
    )


def _signature_help_from_runtime_symbol(
    symbol: RuntimeSymbolResult,
    active_parameter: int,
) -> Optional[SignatureHelp]:
    label = symbol.detail.strip()
    if not label or "(" not in label or ")" not in label:
        return None

    parameter_labels = split_signature_parameters(label)
    parameters = [ParameterInformation(label=parameter) for parameter in parameter_labels] or None
    clamped_active_parameter = min(active_parameter, max(len(parameter_labels) - 1, 0))

    return SignatureHelp(
        signatures=[
            SignatureInformation(
                label=label,
                documentation=symbol.docstring,
                parameters=parameters,
            )
        ],
        active_signature=0,
        active_parameter=clamped_active_parameter,
    )


def _encode_semantic_tokens(record: ModuleRecord, text: str) -> list[int]:
    tokens: list[tuple[int, int, int, int, int]] = []
    seen: set[tuple[int, int, int]] = set()

    def push(
        line: int,
        character: int,
        length: int,
        token_type: str,
        modifiers: tuple[str, ...] = (),
    ) -> None:
        if length <= 0:
            return
        key = (line, character, length)
        if key in seen:
            return
        seen.add(key)
        tokens.append(
            (
                line,
                character,
                length,
                SEMANTIC_TYPE_INDEX[token_type],
                _semantic_modifier_mask(modifiers),
            )
        )

    for match in SEMANTIC_PREPARSE_RE.finditer(text):
        symbols_text = match.group("symbols")
        symbols_start = match.start("symbols")
        relative_offset = 0
        for name in [item.strip() for item in symbols_text.split(",") if item.strip()]:
            local_offset = symbols_text.find(name, relative_offset)
            if local_offset < 0:
                continue
            start = _offset_to_position(text, symbols_start + local_offset)
            push(start[0], start[1], len(name), "parameter", ("declaration",))
            relative_offset = local_offset + len(name)

    for match in SEMANTIC_CLASS_DEF_RE.finditer(text):
        start = _offset_to_position(text, match.start("name"))
        push(start[0], start[1], len(match.group("name")), "class", ("declaration",))

    for match in SEMANTIC_FUNCTION_DEF_RE.finditer(text):
        start = _offset_to_position(text, match.start("name"))
        token_type = "method" if match.group("indent") else "function"
        push(start[0], start[1], len(match.group("name")), token_type, ("declaration",))

    for symbol in record.symbols.values():
        token_type = _semantic_token_type_for_symbol_kind(symbol.kind)
        if token_type is None:
            continue
        push(
            symbol.source_range.start.line,
            symbol.source_range.start.character,
            len(symbol.name),
            token_type,
            ("declaration",),
        )

    for owner_symbols in record.member_symbols.values():
        for symbol in owner_symbols.values():
            token_type = _semantic_token_type_for_symbol_kind(symbol.kind, is_member=True)
            if token_type is None:
                continue
            push(
                symbol.source_range.start.line,
                symbol.source_range.start.character,
                len(symbol.name),
                token_type,
                ("declaration",),
            )

    for match in SEMANTIC_DECORATOR_RE.finditer(text):
        start = _offset_to_position(text, match.start("name"))
        push(start[0], start[1], len(match.group("name")), "decorator")

    for name in SEMANTIC_NAMESPACE_NAMES:
        for source_range in iter_identifier_ranges(text, name):
            push(
                source_range.start.line,
                source_range.start.character,
                len(name),
                "namespace",
                ("defaultLibrary",),
            )

    for name in SEMANTIC_READONLY_NAMES:
        for source_range in iter_identifier_ranges(text, name):
            push(
                source_range.start.line,
                source_range.start.character,
                len(name),
                "variable",
                ("readonly", "defaultLibrary"),
            )

    for name in SEMANTIC_TYPE_NAMES:
        for source_range in iter_identifier_ranges(text, name):
            push(
                source_range.start.line,
                source_range.start.character,
                len(name),
                "type",
                ("defaultLibrary",),
            )

    for name in SEMANTIC_FUNCTION_NAMES:
        for source_range in iter_identifier_ranges(text, name):
            push(
                source_range.start.line,
                source_range.start.character,
                len(name),
                "function",
                ("defaultLibrary",),
            )

    encoded: list[int] = []
    previous_line = 0
    previous_character = 0
    for line, character, length, token_type, modifier_mask in sorted(tokens):
        delta_line = line - previous_line
        delta_character = character - previous_character if delta_line == 0 else character
        encoded.extend([delta_line, delta_character, length, token_type, modifier_mask])
        previous_line = line
        previous_character = character
    return encoded


def _semantic_modifier_mask(modifiers: tuple[str, ...]) -> int:
    mask = 0
    for modifier in modifiers:
        mask |= 1 << SEMANTIC_MODIFIER_INDEX[modifier]
    return mask


def _semantic_token_type_for_symbol_kind(kind: str, is_member: bool = False) -> Optional[str]:
    if kind == "class":
        return "class"
    if kind == "function":
        return "method" if is_member else "function"
    if kind in {"variable", "constant"}:
        return "variable"
    return None


def _offset_to_position(text: str, offset: int) -> tuple[int, int]:
    line = text.count("\n", 0, offset)
    line_start = text.rfind("\n", 0, offset)
    character = offset if line_start < 0 else offset - line_start - 1
    return (line, character)


def _as_completion_item(item: CompletionItem | dict[str, object]) -> CompletionItem:
    if isinstance(item, CompletionItem):
        return item

    return CompletionItem(
        label=str(item["label"]),
        kind=int(item["kind"]) if "kind" in item and item["kind"] is not None else None,
        detail=str(item["detail"]) if "detail" in item and item["detail"] is not None else None,
    )


def _resolve_request_symbol(
    server: SageLanguageServer,
    uri: str,
    position: Position,
) -> Optional[ResolvedRequestSymbol]:
    record, text = _record_for_uri(server, uri)
    if record is None:
        return None

    name, static_name, runtime_name, source_range = symbol_at_position(
        text or record.source,
        position.line,
        position.character,
    )
    if not name or source_range is None:
        return None

    return {
        "record": record,
        "name": name,
        "static_name": static_name,
        "runtime_name": runtime_name,
        "range": source_range,
    }


def _request_cache_key(
    uri: Optional[str],
    name: str,
    runtime_name: Optional[str],
) -> Optional[tuple[str, str, str]]:
    if uri is None:
        return None
    return (uri, name, runtime_name or name)


def _clear_request_caches(server: SageLanguageServer, uri: str) -> None:
    stale_keys = [key for key in server.documentation_cache if key[0] == uri]
    for key in stale_keys:
        server.documentation_cache.pop(key, None)
    stale_definition_keys = [key for key in server.definition_cache if key[0] == uri]
    for key in stale_definition_keys:
        server.definition_cache.pop(key, None)


def _clear_all_request_caches(server: SageLanguageServer) -> None:
    server.documentation_cache.clear()
    server.definition_cache.clear()


def _documentation_prewarm_candidates(
    record: ModuleRecord,
    text: str,
) -> list[tuple[str, str]]:
    candidates: list[tuple[str, str]] = []
    seen: set[tuple[str, str]] = set()

    def add(static_name: str, runtime_name: Optional[str] = None) -> None:
        candidate = (static_name, runtime_name or static_name)
        if candidate in seen:
            return
        seen.add(candidate)
        candidates.append(candidate)

    for name, symbol in sorted(record.symbols.items()):
        if symbol.kind in {"function", "class"} and not name.startswith("_"):
            add(name)
        if len(candidates) >= MAX_PREWARM_SYMBOLS:
            return candidates

    for match in PREWARM_CALL_RE.finditer(text):
        raw_name = match.group("name")
        static_name = raw_name if "." in raw_name else raw_name
        add(static_name, raw_name)
        if len(candidates) >= MAX_PREWARM_SYMBOLS:
            break

    return candidates


def symbol_at_position(
    text: str,
    line: int,
    character: int,
) -> Tuple[Optional[str], Optional[str], Optional[str], Optional[Range]]:
    dotted_name, dotted_range = dotted_word_at_position(text, line, character)
    if dotted_name is not None and dotted_range is not None:
        return dotted_name.split(".")[-1], dotted_name, dotted_name, dotted_range

    name, source_range = word_at_position(text, line, character)
    return name, name, name, source_range


def completion_target_at_position(
    text: str,
    line: int,
    character: int,
) -> Tuple[Optional[str], str]:
    lines = text.splitlines()
    if line < 0 or line >= len(lines):
        return None, ""

    prefix = lines[line][: max(character, 0)]
    if not prefix:
        return None, ""

    start = len(prefix)
    while start > 0 and _is_dotted_word_char(prefix[start - 1]):
        start -= 1

    candidate = prefix[start:]
    if "." not in candidate:
        return None, current_prefix(text, line, character)

    base, member_prefix = candidate.rsplit(".", maxsplit=1)
    if not base:
        return None, ""

    parts = base.split(".")
    if not all(part and _is_word_char(part[0]) and all(_is_word_char(char) for char in part) for part in parts):
        return None, ""
    if member_prefix and not all(_is_word_char(char) for char in member_prefix):
        return None, ""

    return base, member_prefix


def call_expression_at_position(
    text: str,
    line: int,
    character: int,
) -> Tuple[Optional[str], int]:
    lines = text.splitlines()
    if line < 0 or line >= len(lines):
        return None, 0

    prefix = lines[line][: max(character, 0)]
    if not prefix:
        return None, 0

    depth = 0
    active_parameter = 0
    open_index: Optional[int] = None

    for index in range(len(prefix) - 1, -1, -1):
        char = prefix[index]
        if char in ")]}":
            depth += 1
            continue
        if char in "([{":
            if depth == 0:
                if char != "(":
                    return None, 0
                open_index = index
                break
            depth -= 1
            continue
        if char == "," and depth == 0:
            active_parameter += 1

    if open_index is None:
        return None, 0

    end = open_index
    start = end
    while start > 0 and _is_dotted_word_char(prefix[start - 1]):
        start -= 1

    candidate = prefix[start:end].strip(".")
    if not candidate:
        return None, 0
    parts = candidate.split(".")
    if not all(part and _is_word_char(part[0]) and all(_is_word_char(char) for char in part) for part in parts):
        return None, 0

    return candidate, active_parameter


def current_prefix(text: str, line: int, character: int) -> str:
    lines = text.splitlines()
    if line < 0 or line >= len(lines):
        return ""
    source_line = lines[line]
    if character < 0:
        return ""
    bounded_character = min(character, len(source_line))
    start = bounded_character
    while start > 0 and _is_word_char(source_line[start - 1]):
        start -= 1
    return source_line[start:bounded_character]


def word_at_position(text: str, line: int, character: int) -> Tuple[Optional[str], Optional[Range]]:
    lines = text.splitlines()
    if line < 0 or line >= len(lines):
        return None, None
    source_line = lines[line]
    if not source_line:
        return None, None

    bounded_character = min(max(character, 0), max(len(source_line) - 1, 0))
    if not _is_word_char(source_line[bounded_character]) and bounded_character > 0 and _is_word_char(source_line[bounded_character - 1]):
        bounded_character -= 1

    if not _is_word_char(source_line[bounded_character]):
        return None, None

    start = bounded_character
    end = bounded_character + 1
    while start > 0 and _is_word_char(source_line[start - 1]):
        start -= 1
    while end < len(source_line) and _is_word_char(source_line[end]):
        end += 1

    return (
        source_line[start:end],
        Range(start=Position(line=line, character=start), end=Position(line=line, character=end)),
    )


def dotted_word_at_position(text: str, line: int, character: int) -> Tuple[Optional[str], Optional[Range]]:
    lines = text.splitlines()
    if line < 0 or line >= len(lines):
        return None, None
    source_line = lines[line]
    if not source_line:
        return None, None

    bounded_character = min(max(character, 0), max(len(source_line) - 1, 0))
    if (
        not _is_dotted_word_char(source_line[bounded_character])
        and bounded_character > 0
        and _is_dotted_word_char(source_line[bounded_character - 1])
    ):
        bounded_character -= 1

    if not _is_dotted_word_char(source_line[bounded_character]):
        return None, None

    start = bounded_character
    end = bounded_character + 1
    while start > 0 and _is_dotted_word_char(source_line[start - 1]):
        start -= 1
    while end < len(source_line) and _is_dotted_word_char(source_line[end]):
        end += 1

    candidate = source_line[start:end].strip(".")
    if "." not in candidate:
        return None, None
    parts = candidate.split(".")
    if not all(part and (_is_word_char(part[0])) and all(_is_word_char(char) for char in part) for part in parts):
        return None, None

    candidate_start = source_line[start:end].find(candidate) + start
    candidate_end = candidate_start + len(candidate)
    return (
        candidate,
        Range(
            start=Position(line=line, character=candidate_start),
            end=Position(line=line, character=candidate_end),
        ),
    )


def split_signature_parameters(signature_label: str) -> list[str]:
    start = signature_label.find("(")
    end = signature_label.rfind(")")
    if start == -1 or end <= start + 1:
        return []

    body = signature_label[start + 1 : end]
    parameters: list[str] = []
    current: list[str] = []
    depth = 0
    quote: Optional[str] = None
    escaped = False

    for char in body:
        if quote is not None:
            current.append(char)
            if escaped:
                escaped = False
            elif char == "\\":
                escaped = True
            elif char == quote:
                quote = None
            continue

        if char in {"'", '"'}:
            quote = char
            current.append(char)
            continue

        if char in "([{":
            depth += 1
            current.append(char)
            continue

        if char in ")]}":
            depth = max(depth - 1, 0)
            current.append(char)
            continue

        if char == "," and depth == 0:
            parameter = "".join(current).strip()
            if parameter:
                parameters.append(parameter)
            current = []
            continue

        current.append(char)

    tail = "".join(current).strip()
    if tail:
        parameters.append(tail)
    return parameters


def _is_word_char(value: str) -> bool:
    return value.isalnum() or value == "_"


def _is_dotted_word_char(value: str) -> bool:
    return _is_word_char(value) or value == "."


def _ranges_overlap(
    left: dict[str, dict[str, int]],
    right: dict[str, dict[str, int]],
) -> bool:
    return _range_sort_key(left["start"]) < _range_sort_key(right["end"]) and _range_sort_key(right["start"]) < _range_sort_key(left["end"])


def _range_sort_key(position: dict[str, int]) -> tuple[int, int]:
    return (int(position["line"]), int(position["character"]))


def _as_dict_range(raw_range: Range) -> dict[str, dict[str, int]]:
    return {
        "start": {"line": raw_range.start.line, "character": raw_range.start.character},
        "end": {"line": raw_range.end.line, "character": raw_range.end.character},
    }


def _as_lsp_diagnostic(entry: dict[str, object]) -> Diagnostic:
    return Diagnostic(
        range=_as_lsp_range(entry["range"]),
        severity=DiagnosticSeverity(entry.get("severity", 2)),
        code=entry.get("code"),
        source=str(entry.get("source", "sage-lsp")),
        message=str(entry["message"]),
        data=entry.get("data"),
    )


def _as_lsp_range(raw_range: dict[str, dict[str, int]]) -> Range:
    return Range(
        start=Position(
            line=raw_range["start"]["line"],
            character=raw_range["start"]["character"],
        ),
        end=Position(
            line=raw_range["end"]["line"],
            character=raw_range["end"]["character"],
        ),
    )


def coerce_path(value: str) -> Path:
    if value.startswith("file://"):
        return path_from_uri(value)
    return Path(value)
