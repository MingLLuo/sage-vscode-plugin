from pathlib import Path

from lsprotocol.types import (
    ClientCapabilities,
    CompletionParams,
    DidCloseTextDocumentParams,
    DidChangeWatchedFilesParams,
    DidOpenTextDocumentParams,
    DidSaveTextDocumentParams,
    DefinitionParams,
    DocumentSymbolParams,
    FileChangeType,
    FileEvent,
    HoverParams,
    InitializeParams,
    Position,
    ReferenceContext,
    ReferenceParams,
    RenameParams,
    SemanticTokensParams,
    SignatureHelpParams,
    TextDocumentIdentifier,
    TextDocumentItem,
    TextDocumentSyncKind,
    WorkspaceSymbolParams,
    WorkspaceFolder,
)
from pygls.workspace import Workspace

from sage_lsp.runtime_introspection import RuntimeSymbolResult
from sage_lsp.server import _merge_documentation_with_runtime, create_server
from sage_lsp.index import DocumentationResult


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


def test_server_declares_semantic_tokens_and_encodes_sage_structures() -> None:
    server = create_server()
    server.protocol._workspace = Workspace(  # noqa: SLF001 - intentional test setup
        root_uri="file:///workspace",
        sync_kind=TextDocumentSyncKind.Incremental,
        workspace_folders=[WorkspaceFolder(uri="file:///workspace", name="workspace")],
    )
    initialize_handler = server.protocol.fm.features["initialize"]
    initialize_result = initialize_handler(
        InitializeParams(
            process_id=1,
            root_uri="file:///workspace",
            capabilities=ClientCapabilities(),
            workspace_folders=[WorkspaceFolder(uri="file:///workspace", name="workspace")],
            initialization_options={
                "workspace": {"sourceRoots": [str(FIXTURE_ROOT)]},
                "analysis": {"enablePyxParsing": True},
            },
        )
    )
    uri = Path("/workspace/semantic_tokens.sage").as_uri()
    source = (
        "from sage.misc.cachefunc import cached_method\n"
        "X = toric_varieties.P2()\n"
        "R.<x, y> = PolynomialRing(QQ, 2)\n"
        "class DemoFamily:\n"
        "    @cached_method\n"
        "    def invariant(self):\n"
        "        return matrix(QQ, [[1]])\n"
    )
    server.workspace.put_text_document(
        TextDocumentItem(uri=uri, language_id="sagemath", version=1, text=source)
    )

    semantic_provider = initialize_result.capabilities.semantic_tokens_provider
    assert semantic_provider is not None
    legend = semantic_provider.legend if hasattr(semantic_provider, "legend") else semantic_provider["legend"]  # type: ignore[index]
    token_types = legend.token_types if hasattr(legend, "token_types") else legend["tokenTypes"]  # type: ignore[index]
    token_modifiers = (
        legend.token_modifiers if hasattr(legend, "token_modifiers") else legend["tokenModifiers"]  # type: ignore[index]
    )
    assert token_types

    semantic_handler = server.protocol.fm.features["textDocument/semanticTokens/full"]
    semantic_tokens = semantic_handler(
        SemanticTokensParams(text_document=TextDocumentIdentifier(uri=uri))
    )

    decoded = decode_semantic_tokens(
        semantic_tokens.data,
        source,
        token_types,
        token_modifiers,
    )

    assert any(token["lexeme"] == "toric_varieties" and token["type"] == "namespace" for token in decoded)
    assert any(token["lexeme"] == "PolynomialRing" and token["type"] == "type" for token in decoded)
    assert any(token["lexeme"] == "QQ" and token["type"] == "variable" and "readonly" in token["modifiers"] for token in decoded)
    assert any(token["lexeme"] == "cached_method" and token["type"] == "decorator" for token in decoded)
    assert any(token["lexeme"] == "DemoFamily" and token["type"] == "class" and "declaration" in token["modifiers"] for token in decoded)
    assert any(token["lexeme"] == "invariant" and token["type"] == "method" and "declaration" in token["modifiers"] for token in decoded)
    assert any(token["lexeme"] == "x" and token["type"] == "parameter" and "declaration" in token["modifiers"] for token in decoded)


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


def test_server_drops_overlay_documents_on_close() -> None:
    server = _initialized_server()
    uri = Path("/workspace/example.sage").as_uri()
    source = "value = ZZ\nsqrt(4)\n"
    server.workspace.put_text_document(
        TextDocumentItem(uri=uri, language_id="sagemath", version=1, text=source)
    )

    open_handler = server.protocol.fm.features["textDocument/didOpen"]
    open_handler(
        DidOpenTextDocumentParams(
            text_document=TextDocumentItem(uri=uri, language_id="sagemath", version=1, text=source)
        )
    )
    assert server.workspace_index is not None
    assert uri in server.workspace_index._document_records  # noqa: SLF001 - intentional state verification

    close_handler = server.protocol.fm.features["textDocument/didClose"]
    close_handler(DidCloseTextDocumentParams(text_document=TextDocumentIdentifier(uri=uri)))

    assert uri not in server.workspace_index._document_records  # noqa: SLF001 - intentional state verification


def test_server_did_save_refreshes_indexed_modules(tmp_path: Path) -> None:
    root = tmp_path / "src"
    helpers_path = root / "pkg" / "helpers.py"
    helpers_path.parent.mkdir(parents=True, exist_ok=True)
    (root / "pkg" / "__init__.py").write_text("", encoding="utf-8")
    helpers_path.write_text("def helper(value):\n    return value\n", encoding="utf-8")

    server = _initialized_server_with_options(
        {
            "workspace": {"sourceRoots": [str(root)]},
            "analysis": {"enablePyxParsing": True},
        }
    )

    uri = helpers_path.as_uri()
    source = 'def helper(value):\n    """Saved."""\n    return value + 1\n'
    server.workspace.put_text_document(
        TextDocumentItem(uri=uri, language_id="python", version=1, text=source)
    )
    helpers_path.write_text(source, encoding="utf-8")

    save_handler = server.protocol.fm.features["textDocument/didSave"]
    save_handler(DidSaveTextDocumentParams(text_document=TextDocumentIdentifier(uri=uri)))

    assert server.workspace_index is not None
    record = server.workspace_index.modules["pkg.helpers"]
    documentation = server.workspace_index.documentation_for_symbol(record, "helper")
    assert documentation is not None
    assert documentation.summary == "Saved."


def test_server_did_save_clears_cached_imported_hover_results(tmp_path: Path) -> None:
    root = tmp_path / "src"
    package_dir = root / "pkg"
    package_dir.mkdir(parents=True, exist_ok=True)
    (package_dir / "__init__.py").write_text("", encoding="utf-8")
    helper_path = package_dir / "helpers.py"
    helper_path.write_text(
        'def helper(value):\n    """Original helper summary."""\n    return value\n',
        encoding="utf-8",
    )
    consumer_path = package_dir / "consumer.sage"
    consumer_source = "from pkg.helpers import helper\n\nvalue = helper(4)\n"
    consumer_path.write_text(consumer_source, encoding="utf-8")

    server = _initialized_server_with_options(
        {
            "workspace": {"sourceRoots": [str(root)]},
            "analysis": {"enablePyxParsing": True},
        }
    )

    consumer_uri = consumer_path.as_uri()
    server.workspace.put_text_document(
        TextDocumentItem(uri=consumer_uri, language_id="sagemath", version=1, text=consumer_source)
    )

    hover_handler = server.protocol.fm.features["textDocument/hover"]
    hover = hover_handler(
        HoverParams(
            text_document=TextDocumentIdentifier(uri=consumer_uri),
            position=Position(line=2, character=8),
        )
    )
    assert hover is not None
    assert "Original helper summary." in hover.contents.value

    updated_helper_source = 'def helper(value):\n    """Updated helper summary."""\n    return value + 1\n'
    server.workspace.put_text_document(
        TextDocumentItem(uri=helper_path.as_uri(), language_id="python", version=1, text=updated_helper_source)
    )
    helper_path.write_text(updated_helper_source, encoding="utf-8")

    save_handler = server.protocol.fm.features["textDocument/didSave"]
    save_handler(DidSaveTextDocumentParams(text_document=TextDocumentIdentifier(uri=helper_path.as_uri())))

    updated_hover = hover_handler(
        HoverParams(
            text_document=TextDocumentIdentifier(uri=consumer_uri),
            position=Position(line=2, character=8),
        )
    )
    assert updated_hover is not None
    assert "Updated helper summary." in updated_hover.contents.value


def test_server_watched_file_changes_refresh_workspace_index(tmp_path: Path) -> None:
    root = tmp_path / "src"
    package_dir = root / "pkg"
    package_dir.mkdir(parents=True, exist_ok=True)
    (package_dir / "__init__.py").write_text("", encoding="utf-8")

    server = _initialized_server_with_options(
        {
            "workspace": {"sourceRoots": [str(root)]},
            "analysis": {"enablePyxParsing": True},
        }
    )
    assert server.workspace_index is not None

    helper_path = package_dir / "dynamic_helper.py"
    helper_path.write_text("def dynamic_helper(value):\n    return value\n", encoding="utf-8")

    watched_handler = server.protocol.fm.features["workspace/didChangeWatchedFiles"]
    watched_handler(
        DidChangeWatchedFilesParams(
            changes=[FileEvent(uri=helper_path.as_uri(), type=FileChangeType.Created)]
        )
    )

    symbols = server.workspace_index.workspace_symbols("dynamic_helper")
    assert any(item["name"] == "dynamic_helper" for item in symbols)

    helper_path.unlink()
    watched_handler(
        DidChangeWatchedFilesParams(
            changes=[FileEvent(uri=helper_path.as_uri(), type=FileChangeType.Deleted)]
        )
    )

    symbols_after_delete = server.workspace_index.workspace_symbols("dynamic_helper")
    assert all(item["name"] != "dynamic_helper" for item in symbols_after_delete)


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


def test_server_prewarms_runtime_documentation_for_open_documents() -> None:
    server = _initialized_server_with_options(
        {
            "workspace": {"sourceRoots": []},
            "analysis": {
                "enablePyxParsing": True,
                "enableRuntimeIntrospection": True,
            },
        }
    )
    uri = Path("/workspace/runtime_prewarm.sage").as_uri()
    source = "R = PolynomialRing(QQ, 2)\nE = EllipticCurve([0, 0, 1, -1, 0])\n"
    server.workspace.put_text_document(
        TextDocumentItem(uri=uri, language_id="sagemath", version=1, text=source)
    )

    requested_names: list[str] = []

    def _lookup(name: str) -> RuntimeSymbolResult:
        requested_names.append(name)
        return RuntimeSymbolResult(
            name=name,
            kind="function",
            detail=f"{name}(*args, **kwds)",
            module_name=f"sage.runtime.{name}",
            docstring=f"Runtime docs for {name}.",
            file_path=Path(f"/runtime/{name}.py"),
            line=12,
        )

    server.runtime_introspector.lookup = _lookup  # type: ignore[method-assign]

    did_open_handler = server.protocol.fm.features["textDocument/didOpen"]
    did_open_handler(
        DidOpenTextDocumentParams(
            text_document=TextDocumentItem(uri=uri, language_id="sagemath", version=1, text=source)
        )
    )

    assert requested_names == ["PolynomialRing", "PolynomialRing", "EllipticCurve", "EllipticCurve"]
    assert (uri, "PolynomialRing", "PolynomialRing") in server.documentation_cache
    assert (uri, "EllipticCurve", "EllipticCurve") in server.documentation_cache
    assert (uri, "PolynomialRing", "PolynomialRing") in server.definition_cache
    assert (uri, "EllipticCurve", "EllipticCurve") in server.definition_cache


def test_server_hover_uses_prewarmed_documentation_cache() -> None:
    server = _initialized_server_with_options(
        {
            "workspace": {"sourceRoots": []},
            "analysis": {
                "enablePyxParsing": True,
                "enableRuntimeIntrospection": True,
            },
        }
    )
    uri = Path("/workspace/runtime_cache_hover.sage").as_uri()
    source = "PolynomialRing(QQ, 2)\n"
    server.workspace.put_text_document(
        TextDocumentItem(uri=uri, language_id="sagemath", version=1, text=source)
    )

    requested_names: list[str] = []

    def _lookup(name: str) -> RuntimeSymbolResult:
        requested_names.append(name)
        return RuntimeSymbolResult(
            name=name,
            kind="function",
            detail=f"{name}(*args, **kwds)",
            module_name=f"sage.runtime.{name}",
            docstring=f"Runtime docs for {name}.",
            file_path=Path(f"/runtime/{name}.py"),
            line=12,
        )

    server.runtime_introspector.lookup = _lookup  # type: ignore[method-assign]

    did_open_handler = server.protocol.fm.features["textDocument/didOpen"]
    did_open_handler(
        DidOpenTextDocumentParams(
            text_document=TextDocumentItem(uri=uri, language_id="sagemath", version=1, text=source)
        )
    )

    hover_handler = server.protocol.fm.features["textDocument/hover"]
    hover = hover_handler(
        HoverParams(
            text_document=TextDocumentIdentifier(uri=uri),
            position=Position(line=0, character=2),
        )
    )

    assert hover is not None
    assert "Runtime docs for PolynomialRing." in hover.contents.value
    assert requested_names == ["PolynomialRing", "PolynomialRing"]


def test_server_definition_uses_prewarmed_definition_cache() -> None:
    server = _initialized_server_with_options(
        {
            "workspace": {"sourceRoots": []},
            "analysis": {
                "enablePyxParsing": True,
                "enableRuntimeIntrospection": True,
            },
        }
    )
    uri = Path("/workspace/runtime_cache_definition.sage").as_uri()
    source = "PolynomialRing(QQ, 2)\n"
    server.workspace.put_text_document(
        TextDocumentItem(uri=uri, language_id="sagemath", version=1, text=source)
    )

    requested_names: list[str] = []

    def _lookup(name: str) -> RuntimeSymbolResult:
        requested_names.append(name)
        return RuntimeSymbolResult(
            name=name,
            kind="function",
            detail=f"{name}(*args, **kwds)",
            module_name=f"sage.runtime.{name}",
            docstring=f"Runtime docs for {name}.",
            file_path=Path(f"/runtime/{name}.py"),
            line=12,
        )

    server.runtime_introspector.lookup = _lookup  # type: ignore[method-assign]

    did_open_handler = server.protocol.fm.features["textDocument/didOpen"]
    did_open_handler(
        DidOpenTextDocumentParams(
            text_document=TextDocumentItem(uri=uri, language_id="sagemath", version=1, text=source)
        )
    )

    definition_handler = server.protocol.fm.features["textDocument/definition"]
    definition = definition_handler(
        DefinitionParams(
            text_document=TextDocumentIdentifier(uri=uri),
            position=Position(line=0, character=2),
        )
    )

    assert definition is not None
    assert definition.uri.endswith("/runtime/PolynomialRing.py")
    assert requested_names == ["PolynomialRing", "PolynomialRing"]


def test_merge_documentation_with_runtime_prefers_runtime_signature_and_static_source() -> None:
    merged = _merge_documentation_with_runtime(
        DocumentationResult(
            name="PolynomialRing",
            kind="function",
            module_name="sage.rings.polynomial.polynomial_ring_constructor",
            uri=Path("/workspace/sage/rings/polynomial/polynomial_ring_constructor.py").as_uri(),
            detail="function PolynomialRing",
            summary="Static summary.",
            docstring="Static summary.\n\nMore static documentation.",
            markers=("kind:function",),
        ),
        RuntimeSymbolResult(
            name="PolynomialRing",
            kind="function",
            detail="PolynomialRing(base_ring, *args, **kwds)",
            module_name="sage.rings.polynomial.polynomial_ring_constructor",
            docstring="Return the globally unique univariate or multivariate polynomial ring.",
            file_path=Path("/runtime/sage/rings/polynomial/polynomial_ring_constructor.py"),
            line=60,
        ),
    )

    assert merged.detail == "PolynomialRing(base_ring, *args, **kwds)"
    assert merged.uri.endswith("workspace/sage/rings/polynomial/polynomial_ring_constructor.py")
    assert merged.docstring == "Static summary.\n\nMore static documentation."


def test_merge_documentation_with_runtime_uses_runtime_doc_for_weak_static_symbols() -> None:
    merged = _merge_documentation_with_runtime(
        DocumentationResult(
            name="EllipticCurve",
            kind="variable",
            module_name="sage.schemes.elliptic_curves.constructor",
            uri=Path("/workspace/sage/schemes/elliptic_curves/constructor.py").as_uri(),
            detail="variable EllipticCurve",
            summary=None,
            docstring=None,
            markers=("kind:variable",),
        ),
        RuntimeSymbolResult(
            name="EllipticCurve",
            kind="function",
            detail="EllipticCurve(*args, **kwds)",
            module_name="sage.schemes.elliptic_curves.constructor",
            docstring="Construct an elliptic curve.",
            file_path=Path("/runtime/sage/schemes/elliptic_curves/constructor.py"),
            line=42,
        ),
    )

    assert merged.kind == "function"
    assert merged.detail == "EllipticCurve(*args, **kwds)"
    assert merged.summary == "Construct an elliptic curve."
    assert merged.docstring == "Construct an elliptic curve."


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


def decode_semantic_tokens(
    data: list[int],
    source: str,
    token_types: list[str],
    token_modifiers: list[str],
) -> list[dict[str, object]]:
    decoded: list[dict[str, object]] = []
    current_line = 0
    current_character = 0
    lines = source.splitlines()

    for index in range(0, len(data), 5):
        delta_line, delta_character, length, token_type, modifier_mask = data[index:index + 5]
        current_line += delta_line
        current_character = current_character + delta_character if delta_line == 0 else delta_character
        lexeme = lines[current_line][current_character:current_character + length]
        modifiers = [
            modifier
            for bit, modifier in enumerate(token_modifiers)
            if modifier_mask & (1 << bit)
        ]
        decoded.append(
            {
                "line": current_line,
                "character": current_character,
                "lexeme": lexeme,
                "type": token_types[token_type],
                "modifiers": modifiers,
            }
        )

    return decoded


def _write_module(root: Path, relative_path: str, contents: str) -> None:
    module_path = root / relative_path
    module_path.parent.mkdir(parents=True, exist_ok=True)
    module_path.write_text(contents, encoding="utf-8")
