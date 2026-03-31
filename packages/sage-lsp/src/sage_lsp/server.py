from __future__ import annotations

from lsprotocol.types import Hover, HoverParams, InitializeParams, MarkupContent, MarkupKind
from pygls.server import LanguageServer

from .environment import ServerSettings
from .source_map import preprocess_document


class SageLanguageServer(LanguageServer):
    def __init__(self) -> None:
        super().__init__("sage-lsp", "0.1.0")
        self.settings = ServerSettings.from_initialization_options({})


def create_server() -> SageLanguageServer:
    server = SageLanguageServer()

    @server.feature("initialize")
    def on_initialize(params: InitializeParams) -> None:
        server.settings = ServerSettings.from_initialization_options(params.initialization_options)

    @server.feature("textDocument/hover")
    def on_hover(params: HoverParams) -> Hover:
        message = _build_hover_message(server, params)
        return Hover(contents=MarkupContent(kind=MarkupKind.Markdown, value=message))

    return server


def _build_hover_message(server: SageLanguageServer, params: HoverParams) -> str:
    message = [
        "Sage LSP bootstrap server is running.",
        "",
        f"Trust mode: `{server.settings.workspace_trust_mode}`",
        f"Log level: `{server.settings.log_level}`",
    ]

    try:
        document = server.workspace.get_text_document(params.text_document.uri)
    except KeyError:
        return "\n".join(message)

    preprocessed = preprocess_document(document.uri, document.source)
    if document.uri.lower().endswith(".sage"):
        mapped = preprocessed.map_source_to_generated(params.position.line, params.position.character)
        preview_line = preprocessed.line_maps[mapped.line].generated_line
        message.extend(
            [
                "",
                f"Preprocessed `.sage`: `{'yes' if preprocessed.changed else 'no change'}`",
                f"Mapped position: `{params.position.line}:{params.position.character}` -> `{mapped.line}:{mapped.character}`",
                f"Generated line preview: `{preview_line}`",
            ]
        )

    return "\n".join(message)
