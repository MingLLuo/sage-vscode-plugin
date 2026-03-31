from pathlib import Path

from lsprotocol.types import (
    ClientCapabilities,
    CompletionParams,
    DidOpenTextDocumentParams,
    DefinitionParams,
    DocumentSymbolParams,
    HoverParams,
    InitializeParams,
    Position,
    ReferenceContext,
    ReferenceParams,
    RenameParams,
    SignatureHelpParams,
    TextDocumentIdentifier,
    TextDocumentItem,
    TextDocumentSyncKind,
    WorkspaceSymbolParams,
    WorkspaceFolder,
)
from pygls.workspace import Workspace

from sage_lsp.runtime_introspection import RuntimeSymbolResult
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
    assert "workspace/didChangeConfiguration" in server.protocol.fm.features


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
    assert all(not isinstance(item, dict) for item in completion.items)
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

    server.runtime_introspector.lookup = lambda name: RuntimeSymbolResult(  # type: ignore[method-assign]
        name=name,
        kind="function",
        detail="sqrt(x)",
        module_name="sage.misc.functional",
        docstring="Return the principal square root.",
        file_path=Path("/runtime/sage/functions/other.py"),
        line=10,
    )

    signature_help_handler = server.protocol.fm.features["textDocument/signatureHelp"]
    signature_help = signature_help_handler(
        SignatureHelpParams(
            text_document=TextDocumentIdentifier(uri=uri),
            position=Position(line=1, character=5),
        )
    )
    assert signature_help is not None
    assert signature_help.signatures[0].label == "sqrt(x)"


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


def test_server_indexes_modules_from_analysis_extra_paths(tmp_path: Path) -> None:
    package_root = tmp_path / "vendor"
    package_root.mkdir()
    module_path = package_root / "helper_mod.py"
    module_path.write_text("answer = 42\n", encoding="utf-8")

    server = _initialized_server_with_options(
        {
            "workspace": {"sourceRoots": [str(FIXTURE_ROOT)]},
            "analysis": {
                "enablePyxParsing": True,
                "extraPaths": [str(package_root)],
            },
        }
    )

    assert server.workspace_index is not None
    assert "helper_mod" in server.workspace_index.modules


def test_server_resolves_native_cython_definitions(tmp_path: Path) -> None:
    source_root = tmp_path / "src"
    _write_module(source_root, "sage/__init__.py", "")
    _write_module(source_root, "sage/rings/__init__.py", "")
    _write_module(
        source_root,
        "sage/rings/native_support.pxd",
        '"""Typed declarations for native support."""\n\ncdef class NativeAccumulator:\n    pass\n\ncpdef int native_step(int value)\n',
    )

    server = _initialized_server_with_options(
        {
            "workspace": {"sourceRoots": [str(source_root)]},
            "analysis": {"enablePyxParsing": True},
        }
    )
    uri = Path("/workspace/native_consumer.pyx").as_uri()
    source = (
        "from sage.rings.native_support cimport NativeAccumulator, native_step\n"
        "result = native_step(4)\n"
        "bridge = NativeAccumulator\n"
    )
    server.workspace.put_text_document(
        TextDocumentItem(uri=uri, language_id="sagemath-cython", version=1, text=source)
    )

    hover_handler = server.protocol.fm.features["textDocument/hover"]
    hover = hover_handler(
        HoverParams(
            text_document=TextDocumentIdentifier(uri=uri),
            position=Position(line=1, character=10),
        )
    )
    assert hover is not None
    assert "function native_step" in hover.contents.value

    definition_handler = server.protocol.fm.features["textDocument/definition"]
    definition = definition_handler(
        DefinitionParams(
            text_document=TextDocumentIdentifier(uri=uri),
            position=Position(line=2, character=10),
        )
    )
    assert definition is not None
    assert definition.uri.endswith("native_support.pxd")


def test_server_resolves_static_dotted_members_and_member_completion(tmp_path: Path) -> None:
    source_root = tmp_path / "src"
    _write_module(source_root, "pkg/__init__.py", "")
    _write_module(
        source_root,
        "pkg/smallgraphs.py",
        'def PetersenGraph():\n    """Build the Petersen graph."""\n    return 10\n',
    )
    _write_module(
        source_root,
        "pkg/graph_generators.py",
        "class GraphGenerators:\n    from pkg import smallgraphs\n\n    PetersenGraph = staticmethod(smallgraphs.PetersenGraph)\n\n    def CycleGraph(self, n):\n        \"\"\"Build a cycle graph.\"\"\"\n        return n\n\ngraphs = GraphGenerators()\n",
    )

    server = _initialized_server_with_options(
        {
            "workspace": {"sourceRoots": [str(source_root)]},
            "analysis": {"enablePyxParsing": True},
        }
    )
    uri = Path("/workspace/consumer.sage").as_uri()
    source = "from pkg.graph_generators import graphs\n\nvalue = graphs.PetersenGraph()\nnext_value = graphs.Cy\n"
    server.workspace.put_text_document(
        TextDocumentItem(uri=uri, language_id="sagemath", version=1, text=source)
    )

    hover_handler = server.protocol.fm.features["textDocument/hover"]
    hover = hover_handler(
        HoverParams(
            text_document=TextDocumentIdentifier(uri=uri),
            position=Position(line=2, character=19),
        )
    )
    assert hover is not None
    assert "Build the Petersen graph." in hover.contents.value

    definition_handler = server.protocol.fm.features["textDocument/definition"]
    definition = definition_handler(
        DefinitionParams(
            text_document=TextDocumentIdentifier(uri=uri),
            position=Position(line=2, character=19),
        )
    )
    assert definition is not None
    assert definition.uri.endswith("smallgraphs.py")

    completion_handler = server.protocol.fm.features["textDocument/completion"]
    completion = completion_handler(
        CompletionParams(
            text_document=TextDocumentIdentifier(uri=uri),
            position=Position(line=3, character=22),
        )
    )
    assert all(not isinstance(item, dict) for item in completion.items)
    labels = {item["label"] if isinstance(item, dict) else item.label for item in completion.items}
    assert "CycleGraph" in labels


def test_server_provides_workspace_symbols_references_and_rename(tmp_path: Path) -> None:
    source_root = tmp_path / "src"
    _write_module(source_root, "pkg/__init__.py", "")
    _write_module(source_root, "pkg/helpers.py", "def helper(value):\n    return value\n")
    consumer_path = source_root / "pkg" / "consumer.py"
    _write_module(
        source_root,
        "pkg/consumer.py",
        "from pkg.helpers import helper\n\nresult = helper(4)\n",
    )

    server = _initialized_server_with_options(
        {
            "workspace": {"sourceRoots": [str(source_root)]},
            "analysis": {"enablePyxParsing": True},
        }
    )
    consumer_uri = consumer_path.as_uri()
    consumer_source = consumer_path.read_text(encoding="utf-8")
    server.workspace.put_text_document(
        TextDocumentItem(uri=consumer_uri, language_id="python", version=1, text=consumer_source)
    )

    workspace_symbol_handler = server.protocol.fm.features["workspace/symbol"]
    symbols = workspace_symbol_handler(WorkspaceSymbolParams(query="helper"))
    assert any(item["name"] == "helper" and str(item["location"]["uri"]).endswith("helpers.py") for item in symbols)

    references_handler = server.protocol.fm.features["textDocument/references"]
    references = references_handler(
        ReferenceParams(
            text_document=TextDocumentIdentifier(uri=consumer_uri),
            position=Position(line=2, character=9),
            context=ReferenceContext(include_declaration=True),
        )
    )
    assert any(location.uri.endswith("helpers.py") for location in references)
    assert any(location.uri.endswith("consumer.py") for location in references)

    rename_handler = server.protocol.fm.features["textDocument/rename"]
    rename = rename_handler(
        RenameParams(
            text_document=TextDocumentIdentifier(uri=consumer_uri),
            position=Position(line=2, character=9),
            new_name="renamed_helper",
        )
    )
    assert rename is not None
    assert any(uri.endswith("helpers.py") for uri in rename.changes)
    assert any(uri.endswith("consumer.py") for uri in rename.changes)


def test_server_publishes_import_diagnostics_on_open() -> None:
    server = _initialized_server()
    uri = Path("/workspace/broken.py").as_uri()
    source = "from missing.module import helper\n"
    server.workspace.put_text_document(
        TextDocumentItem(uri=uri, language_id="python", version=1, text=source)
    )

    published: list[object] = []
    server.text_document_publish_diagnostics = published.append  # type: ignore[assignment]

    did_open_handler = server.protocol.fm.features["textDocument/didOpen"]
    did_open_handler(
        DidOpenTextDocumentParams(
            text_document=TextDocumentItem(uri=uri, language_id="python", version=1, text=source)
        )
    )

    assert published
    assert published[0].uri == uri
    assert any("missing.module" in diagnostic.message for diagnostic in published[0].diagnostics)


def test_server_falls_back_to_runtime_docs_and_definition() -> None:
    server = _initialized_server_with_options(
        {
            "workspace": {"sourceRoots": []},
            "analysis": {
                "enablePyxParsing": True,
                "enableRuntimeIntrospection": True,
            },
        }
    )
    uri = Path("/workspace/runtime_only.sage").as_uri()
    source = "runtime_only\n"
    server.workspace.put_text_document(
        TextDocumentItem(uri=uri, language_id="sagemath", version=1, text=source)
    )
    server.runtime_introspector.lookup = lambda name: RuntimeSymbolResult(  # type: ignore[method-assign]
        name=name,
        kind="function",
        detail="runtime_only(x)",
        module_name="sage.runtime",
        docstring="Runtime fallback documentation.",
        file_path=Path("/runtime/sage/runtime_only.pyx"),
        line=42,
    )

    hover_handler = server.protocol.fm.features["textDocument/hover"]
    hover = hover_handler(
        HoverParams(
            text_document=TextDocumentIdentifier(uri=uri),
            position=Position(line=0, character=2),
        )
    )
    assert hover is not None
    assert "Runtime fallback documentation." in hover.contents.value

    definition_handler = server.protocol.fm.features["textDocument/definition"]
    definition = definition_handler(
        DefinitionParams(
            text_document=TextDocumentIdentifier(uri=uri),
            position=Position(line=0, character=2),
        )
    )
    assert definition is not None
    assert definition.uri.endswith("runtime_only.pyx")
    assert definition.range.start.line == 41

    documentation_handler = server.protocol.fm.features["sage/getDocumentation"]
    documentation = documentation_handler(
        {
            "textDocument": {"uri": uri},
            "position": {"line": 0, "character": 2},
        }
    )
    assert documentation is not None
    assert documentation["name"] == "runtime_only"
    assert documentation["summary"] == "Runtime fallback documentation."


def test_server_runtime_fallback_prefers_dotted_symbol_names() -> None:
    server = _initialized_server_with_options(
        {
            "workspace": {"sourceRoots": []},
            "analysis": {
                "enablePyxParsing": True,
                "enableRuntimeIntrospection": True,
            },
        }
    )
    uri = Path("/workspace/runtime_dotted.sage").as_uri()
    source = "graphs.PetersenGraph\n"
    server.workspace.put_text_document(
        TextDocumentItem(uri=uri, language_id="sagemath", version=1, text=source)
    )

    requested_names: list[str] = []

    def _lookup(name: str) -> RuntimeSymbolResult:
        requested_names.append(name)
        return RuntimeSymbolResult(
            name=name.split(".")[-1],
            kind="function",
            detail=f"{name}()",
            module_name="sage.graphs.graph_generators",
            docstring="Runtime fallback dotted lookup.",
            file_path=Path("/runtime/sage/graphs/graph_generators.py"),
            line=12,
        )

    server.runtime_introspector.lookup = _lookup  # type: ignore[method-assign]

    documentation_handler = server.protocol.fm.features["sage/getDocumentation"]
    documentation = documentation_handler(
        {
            "textDocument": {"uri": uri},
            "position": {"line": 0, "character": 10},
        }
    )

    assert documentation is not None
    assert requested_names == ["graphs.PetersenGraph"]
    assert documentation["name"] == "PetersenGraph"


def test_server_signature_help_uses_runtime_fallback_for_dotted_calls() -> None:
    server = _initialized_server_with_options(
        {
            "workspace": {"sourceRoots": []},
            "analysis": {
                "enablePyxParsing": True,
                "enableRuntimeIntrospection": True,
            },
        }
    )
    uri = Path("/workspace/runtime_signature.sage").as_uri()
    source = "graphs.PetersenGraph(order=10, immutable=True)\n"
    server.workspace.put_text_document(
        TextDocumentItem(uri=uri, language_id="sagemath", version=1, text=source)
    )

    requested_names: list[str] = []

    def _lookup(name: str) -> RuntimeSymbolResult:
        requested_names.append(name)
        return RuntimeSymbolResult(
            name="PetersenGraph",
            kind="function",
            detail="graphs.PetersenGraph(order=None, immutable=False)",
            module_name="sage.graphs.graph_generators",
            docstring="Build the Petersen graph.",
            file_path=Path("/runtime/sage/graphs/graph_generators.py"),
            line=12,
        )

    server.runtime_introspector.lookup = _lookup  # type: ignore[method-assign]

    signature_help_handler = server.protocol.fm.features["textDocument/signatureHelp"]
    signature_help = signature_help_handler(
        SignatureHelpParams(
            text_document=TextDocumentIdentifier(uri=uri),
            position=Position(line=0, character=40),
        )
    )

    assert signature_help is not None
    assert requested_names == ["graphs.PetersenGraph"]
    assert signature_help.signatures[0].label == "graphs.PetersenGraph(order=None, immutable=False)"
    assert signature_help.active_parameter == 1


def _write_module(root: Path, relative_path: str, contents: str) -> None:
    module_path = root / relative_path
    module_path.parent.mkdir(parents=True, exist_ok=True)
    module_path.write_text(contents, encoding="utf-8")
