from __future__ import annotations

from lsprotocol.types import Hover, HoverParams, InitializeParams, MarkupContent, MarkupKind
from pygls.server import LanguageServer

from .environment import ServerSettings


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
        message = (
            "Sage LSP bootstrap server is running.\n\n"
            f"Trust mode: `{server.settings.workspace_trust_mode}`\n"
            f"Log level: `{server.settings.log_level}`"
        )
        return Hover(contents=MarkupContent(kind=MarkupKind.Markdown, value=message))

    return server

