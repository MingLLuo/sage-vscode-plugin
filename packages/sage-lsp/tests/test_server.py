from pathlib import Path

from lsprotocol.types import (
    ClientCapabilities,
    CompletionParams,
    DefinitionParams,
    DocumentSymbolParams,
    HoverParams,
    InitializeParams,
    Position,
    TextDocumentIdentifier,
    TextDocumentItem,
    TextDocumentSyncKind,
    WorkspaceFolder,
)
from pygls.workspace import Workspace

from sage_lsp.server import create_server


FIXTURE_ROOT = Path(__file__).parent / "fixtures" / "sage_src_lite" / "src"


def _initialized_server():
    return _initialized_server_with_options(
        {
            "workspace": {"sourceRoots": [str(FIXTURE_ROOT)]},
            "analysis": {"enablePyxParsing": True},
        }
    )


def _initialized_server_with_options(initialization_options: dict[str, object]):
    server = create_server()
    server.protocol._workspace = Workspace(  # noqa: SLF001 - intentional test setup
        root_uri="file:///workspace",
        sync_kind=TextDocumentSyncKind.Incremental,
        workspace_folders=[WorkspaceFolder(uri="file:///workspace", name="workspace")],
    )

    initialize = server.protocol.fm.features["initialize"]
    initialize(
        InitializeParams(
            process_id=1,
            root_uri="file:///workspace",
            capabilities=ClientCapabilities(),
            workspace_folders=[WorkspaceFolder(uri="file:///workspace", name="workspace")],
            initialization_options=initialization_options,
        )
    )
    return server


def test_server_imports_and_initializes_with_capabilities() -> None:
    server = _initialized_server()

    assert server.workspace_index is not None
    assert "sage.all" in server.workspace_index.modules


def test_server_handlers_resolve_hover_definition_completion_and_documentation() -> None:
    server = _initialized_server()
    uri = Path("/workspace/example.sage").as_uri()
    source = "value = ZZ\nsqrt(4)\n\n"
    server.workspace.put_text_document(
        TextDocumentItem(uri=uri, language_id="sagemath", version=1, text=source)
    )

    hover_handler = server.protocol.fm.features["textDocument/hover"]
    hover = hover_handler(
        HoverParams(
            text_document=TextDocumentIdentifier(uri=uri),
            position=Position(line=1, character=2),
        )
    )
    assert hover is not None
    assert "Return the principal square root." in hover.contents.value

    definition_handler = server.protocol.fm.features["textDocument/definition"]
    definition = definition_handler(
        DefinitionParams(
            text_document=TextDocumentIdentifier(uri=uri),
            position=Position(line=0, character=8),
        )
    )
    assert definition is not None
    assert definition.uri.endswith("integer_ring.pyx")

    completion_handler = server.protocol.fm.features["textDocument/completion"]
    completion = completion_handler(
        CompletionParams(
            text_document=TextDocumentIdentifier(uri=uri),
            position=Position(line=2, character=0),
        )
    )
    labels = {item["label"] if isinstance(item, dict) else item.label for item in completion.items}
    assert "ZZ" in labels
    assert "x" in labels

    documentation_handler = server.protocol.fm.features["sage/getDocumentation"]
    documentation = documentation_handler(
        {
            "textDocument": {"uri": uri},
            "position": {"line": 1, "character": 2},
        }
    )
    assert documentation is not None
    assert documentation["name"] == "sqrt"
    assert documentation["summary"] == "Return the principal square root."


def test_server_document_symbols_track_open_document_contents() -> None:
    server = _initialized_server()
    uri = Path("/workspace/example.sage").as_uri()
    server.workspace.put_text_document(
        TextDocumentItem(
            uri=uri,
            language_id="sagemath",
            version=1,
            text="R.<x> = PolynomialRing(QQ)\nhelper = factorial\n",
        )
    )

    document_symbol_handler = server.protocol.fm.features["textDocument/documentSymbol"]
    symbols = document_symbol_handler(DocumentSymbolParams(text_document=TextDocumentIdentifier(uri=uri)))
    names = {item["name"] if isinstance(item, dict) else item.name for item in symbols}

    assert {"R", "x", "helper", "factorial"} <= names


def test_server_hover_omits_docstring_when_hover_docs_disabled() -> None:
    server = _initialized_server_with_options(
        {
            "workspace": {"sourceRoots": [str(FIXTURE_ROOT)]},
            "analysis": {"enablePyxParsing": True},
            "documentation": {"showOnHover": False},
        }
    )
    uri = Path("/workspace/example.sage").as_uri()
    server.workspace.put_text_document(
        TextDocumentItem(uri=uri, language_id="sagemath", version=1, text="sqrt(4)\n")
    )

    hover_handler = server.protocol.fm.features["textDocument/hover"]
    hover = hover_handler(
        HoverParams(
            text_document=TextDocumentIdentifier(uri=uri),
            position=Position(line=0, character=2),
        )
    )

    assert hover is not None
    assert hover.contents.value == "function sqrt"
