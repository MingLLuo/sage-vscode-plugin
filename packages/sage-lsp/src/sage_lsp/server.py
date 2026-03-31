from __future__ import annotations

from pathlib import Path
from typing import Any, Dict, Optional, Tuple

from lsprotocol.types import (
    CompletionList,
    CompletionItem,
    CompletionParams,
    Diagnostic,
    DiagnosticSeverity,
    DidChangeTextDocumentParams,
    DidChangeConfigurationParams,
    DidOpenTextDocumentParams,
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
    PublishDiagnosticsParams,
    ReferenceParams,
    Range,
    RenameParams,
    ServerCapabilities,
    SignatureHelp,
    SignatureHelpParams,
    SignatureInformation,
    TextDocumentSyncKind,
    TextEdit,
    WorkspaceEdit,
    WorkspaceSymbolParams,
    ParameterInformation,
)
from pygls.lsp.server import LanguageServer

from .environment import SageEnvironment
from .index import DocumentationResult, WorkspaceIndex, path_from_uri, split_docstring
from .model import ModuleRecord
from .runtime_introspection import RuntimeIntrospector, RuntimeSymbolResult


class SageLanguageServer(LanguageServer):
    def __init__(self) -> None:
        super().__init__("sage-lsp", "0.3.0")
        self.environment = SageEnvironment()
        self.workspace_index: Optional[WorkspaceIndex] = None
        self.runtime_introspector = RuntimeIntrospector(command=None, enabled=False)


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
                references_provider=True,
                rename_provider=True,
                document_symbol_provider=True,
                workspace_symbol_provider=True,
                signature_help_provider={"triggerCharacters": ["(", ","]},
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
        if resolved is None:
            return None

        documentation = _documentation_for_request(
            server,
            resolved["record"],
            str(resolved.get("static_name") or resolved["name"]),
            str(resolved.get("runtime_name") or resolved["name"]),
        )
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
        if resolved is None:
            return None

        symbol = (
            server.workspace_index.resolve_symbol(
                resolved["record"],
                str(resolved.get("static_name") or resolved["name"]),
            )
            if server.workspace_index is not None
            else None
        )
        if symbol is None:
            runtime_symbol = server.runtime_introspector.lookup(
                str(resolved.get("runtime_name") or resolved["name"])
            )
            if runtime_symbol is None or runtime_symbol.file_path is None:
                return None
            return _location_from_runtime_symbol(runtime_symbol)

        return Location(uri=symbol.file_path.as_uri(), range=_as_lsp_range(symbol.source_range.to_lsp()))

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

    @server.feature("textDocument/didOpen")
    def on_did_open(params: DidOpenTextDocumentParams) -> None:
        _publish_diagnostics(server, params.text_document.uri)

    @server.feature("textDocument/didChange")
    def on_did_change(params: DidChangeTextDocumentParams) -> None:
        _publish_diagnostics(server, params.text_document.uri)

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

        documentation = _documentation_for_request(server, record, name, runtime_name or name)
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
    server.runtime_introspector = RuntimeIntrospector.from_environment(server.environment)


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


def _publish_diagnostics(server: SageLanguageServer, uri: str) -> None:
    if server.workspace_index is None or not server.environment.analysis.enable_diagnostics:
        server.text_document_publish_diagnostics(
            PublishDiagnosticsParams(uri=uri, diagnostics=[])
        )
        return

    record, _ = _record_for_uri(server, uri)
    if record is None:
        server.text_document_publish_diagnostics(
            PublishDiagnosticsParams(uri=uri, diagnostics=[])
        )
        return

    diagnostics = [
        Diagnostic(
            range=_as_lsp_range(entry["range"]),
            severity=DiagnosticSeverity(entry.get("severity", 2)),
            source=str(entry.get("source", "sage-lsp")),
            message=str(entry["message"]),
        )
        for entry in server.workspace_index.diagnostics_for_record(record)
    ]
    server.text_document_publish_diagnostics(
        PublishDiagnosticsParams(uri=uri, diagnostics=diagnostics)
    )


def _documentation_for_request(
    server: SageLanguageServer,
    record: ModuleRecord,
    name: str,
    runtime_name: Optional[str] = None,
) -> Optional[DocumentationResult]:
    documentation = (
        server.workspace_index.documentation_for_symbol(record, name)
        if server.workspace_index is not None
        else None
    )
    runtime_symbol = server.runtime_introspector.lookup(runtime_name or name)
    if documentation is None:
        if runtime_symbol is None:
            return None
        return _documentation_from_runtime_symbol(runtime_symbol)
    if runtime_symbol is None:
        return documentation
    return _merge_documentation_with_runtime(documentation, runtime_symbol)


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
) -> Optional[dict[str, object]]:
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
