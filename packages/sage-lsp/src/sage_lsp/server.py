from __future__ import annotations

from pathlib import Path
from typing import Any, Dict, Optional, Tuple

from lsprotocol.types import (
    CompletionList,
    CompletionParams,
    DidChangeConfigurationParams,
    DefinitionParams,
    DocumentSymbolParams,
    Hover,
    HoverParams,
    InitializeParams,
    InitializeResult,
    InitializedParams,
    Location,
    MarkupContent,
    MarkupKind,
    Position,
    Range,
    ServerCapabilities,
    TextDocumentSyncKind,
)
from pygls.lsp.server import LanguageServer

from .environment import SageEnvironment
from .index import WorkspaceIndex, path_from_uri
from .model import ModuleRecord


class SageLanguageServer(LanguageServer):
    def __init__(self) -> None:
        super().__init__("sage-lsp", "0.3.0")
        self.environment = SageEnvironment()
        self.workspace_index: Optional[WorkspaceIndex] = None


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
                text_document_sync=TextDocumentSyncKind.Incremental,
                hover_provider=True,
                definition_provider=True,
                document_symbol_provider=True,
                completion_provider={"triggerCharacters": [".", "("], "resolveProvider": False},
            ),
        )

    @server.feature("initialized")
    def on_initialized(params: InitializedParams) -> None:
        del params

    @server.feature("workspace/didChangeConfiguration")
    def on_did_change_configuration(params: DidChangeConfigurationParams) -> None:
        del params

    @server.feature("textDocument/hover")
    def on_hover(params: HoverParams) -> Optional[Hover]:
        resolved = _resolve_request_symbol(server, params.text_document.uri, params.position)
        if resolved is None or server.workspace_index is None:
            return None

        documentation = server.workspace_index.documentation_for_symbol(resolved["record"], resolved["name"])
        if documentation is None:
            return None

        parts = [documentation.detail]
        if server.environment.documentation.show_on_hover and documentation.docstring:
            parts.append(documentation.docstring)

        return Hover(
            contents=MarkupContent(kind=MarkupKind.Markdown, value="\n\n".join(parts)),
            range=resolved["range"],
        )

    @server.feature("textDocument/definition")
    def on_definition(params: DefinitionParams) -> Optional[Location]:
        resolved = _resolve_request_symbol(server, params.text_document.uri, params.position)
        if resolved is None or server.workspace_index is None:
            return None

        symbol = server.workspace_index.resolve_symbol(resolved["record"], resolved["name"])
        if symbol is None:
            return None

        return Location(uri=symbol.file_path.as_uri(), range=_as_lsp_range(symbol.source_range.to_lsp()))

    @server.feature("textDocument/completion")
    def on_completion(params: CompletionParams) -> CompletionList:
        if server.workspace_index is None:
            return CompletionList(is_incomplete=False, items=[])

        record, text = _record_for_uri(server, params.text_document.uri)
        if record is None:
            return CompletionList(is_incomplete=False, items=[])

        prefix = current_prefix(text or record.source, params.position.line, params.position.character)
        items = server.workspace_index.completion_items(record, prefix)
        return CompletionList(is_incomplete=False, items=items)

    @server.feature("textDocument/documentSymbol")
    def on_document_symbol(params: DocumentSymbolParams) -> list[dict[str, object]]:
        if server.workspace_index is None:
            return []
        record, _ = _record_for_uri(server, params.text_document.uri)
        if record is None:
            return []
        return server.workspace_index.document_symbols(record)

    @server.feature("sage/getDocumentation")
    def on_get_documentation(params: Dict[str, Any]) -> Optional[dict[str, object]]:
        if server.workspace_index is None:
            return None

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
        if not isinstance(name, str):
            position = params.get("position")
            if not isinstance(position, dict):
                return None
            line = position.get("line")
            character = position.get("character")
            if not isinstance(line, int) or not isinstance(character, int):
                return None
            name, _ = word_at_position(text or record.source, line, character)
            if name is None:
                return None

        documentation = server.workspace_index.documentation_for_symbol(record, name)
        return documentation.to_payload() if documentation is not None else None

    return server


def _rebuild_index(server: SageLanguageServer) -> None:
    source_roots = [coerce_path(entry) for entry in server.environment.workspace.source_roots if entry]
    source_roots.extend(coerce_path(entry) for entry in server.environment.analysis.extra_paths if entry)
    deduped_roots = list(dict.fromkeys(root.resolve() for root in source_roots))
    server.workspace_index = WorkspaceIndex(
        source_roots=deduped_roots,
        excluded_globs=server.environment.workspace.excluded_globs,
        enable_pyx=server.environment.analysis.enable_pyx_parsing,
    )
    server.workspace_index.build()


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


def _resolve_request_symbol(
    server: SageLanguageServer,
    uri: str,
    position: Position,
) -> Optional[dict[str, object]]:
    record, text = _record_for_uri(server, uri)
    if record is None:
        return None

    name, source_range = word_at_position(text or record.source, position.line, position.character)
    if not name or source_range is None:
        return None

    return {
        "record": record,
        "name": name,
        "range": source_range,
    }


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


def _is_word_char(value: str) -> bool:
    return value.isalnum() or value == "_"


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
