from pathlib import Path

from sage_lsp.index import WorkspaceIndex, iter_identifier_ranges, path_from_uri
from sage_lsp.parser import parse_module


FIXTURE_ROOT = Path(__file__).parent / "fixtures" / "sage_src_lite" / "src"


def test_workspace_index_resolves_star_exports_and_lazy_imports() -> None:
    index = WorkspaceIndex([FIXTURE_ROOT], (), True)
    index.build()

    module = index.modules["sage.all_cmdline"]

    zz = index.resolve_symbol(module, "ZZ")
    assert zz is not None
    assert zz.file_path.name == "integer_ring.pyx"

    sqrt = index.resolve_symbol(module, "sqrt")
    assert sqrt is not None
    assert sqrt.file_path.name == "other.py"

    x_symbol = index.resolve_symbol(module, "x")
    assert x_symbol is not None
    assert x_symbol.file_path.name == "predefined.py"


def test_workspace_index_resolves_imports_lazily_without_full_build(tmp_path: Path) -> None:
    root = tmp_path / "src"
    _write_module(root, "pkg/__init__.py", "")
    _write_module(root, "pkg/helpers.py", "def helper(value):\n    return value\n")

    index = WorkspaceIndex([root], (), True)
    record = parse_module(
        "document::consumer",
        Path("consumer.py"),
        "from pkg.helpers import helper\n\nresult = helper(4)\n",
    )

    helper = index.resolve_symbol(record, "helper")

    assert helper is not None
    assert helper.file_path.name == "helpers.py"


def test_workspace_index_documentation_uses_source_docstring() -> None:
    index = WorkspaceIndex([FIXTURE_ROOT], (), True)
    index.build()

    module = index.modules["sage.all"]
    documentation = index.documentation_for_symbol(module, "sqrt")

    assert documentation is not None
    assert documentation.docstring is not None
    assert documentation.docstring.startswith("Return the principal square root.")
    assert documentation.summary == "Return the principal square root."
    assert documentation.sections == (
        {"title": "Details", "body": "Accepts a symbolic or numeric value in the reduced fixture corpus."},
    )
    assert "kind:function" in documentation.markers


def test_workspace_index_respects_pyx_disable_and_excludes() -> None:
    index = WorkspaceIndex([FIXTURE_ROOT], ("**/functions/**",), False)
    index.build()

    assert "sage.functions.other" not in index.modules
    assert "sage.rings.integer_ring" not in index.modules


def test_path_from_uri_handles_windows_drive_uris() -> None:
    resolved = path_from_uri("file:///C:/Users/example/project/example.sage")

    assert str(resolved).endswith("C:/Users/example/project/example.sage")


def test_workspace_index_merges_native_module_declarations_and_implementation(tmp_path: Path) -> None:
    root = tmp_path / "src"
    _write_module(root, "sage/__init__.py", "")
    _write_module(root, "sage/rings/__init__.py", "")
    _write_module(
        root,
        "sage/rings/native_support.pxd",
        '"""Native declarations."""\n\ncdef class NativeAccumulator:\n    pass\n\ncpdef int native_step(int value)\n',
    )
    _write_module(
        root,
        "sage/rings/native_support.pyx",
        '"""Native implementation."""\n\ncdef class NativeAccumulator:\n    pass\n\nZZ = NativeAccumulator()\n',
    )

    index = WorkspaceIndex([root], (), True)
    index.build()

    record = index.modules["sage.rings.native_support"]

    assert record.file_path.name == "native_support.pyx"
    assert record.docstring == "Native implementation."
    assert "NativeAccumulator" in record.symbols
    assert "native_step" in record.symbols
    assert "ZZ" in record.symbols


def test_workspace_index_resolves_cimported_symbols_from_pxd(tmp_path: Path) -> None:
    root = tmp_path / "src"
    _write_module(root, "sage/__init__.py", "")
    _write_module(root, "sage/rings/__init__.py", "")
    _write_module(
        root,
        "sage/rings/native_support.pxd",
        "cdef class NativeAccumulator:\n    pass\n\ncpdef int native_step(int value)\n",
    )
    _write_module(
        root,
        "sage/rings/native_consumer.pyx",
        "from sage.rings.native_support cimport NativeAccumulator, native_step\n\ncpdef int use_native(int value):\n    return native_step(value)\n",
    )

    index = WorkspaceIndex([root], (), True)
    index.build()

    consumer = index.modules["sage.rings.native_consumer"]
    accumulator = index.resolve_symbol(consumer, "NativeAccumulator")
    native_step = index.resolve_symbol(consumer, "native_step")

    assert accumulator is not None
    assert accumulator.file_path.name == "native_support.pxd"
    assert native_step is not None
    assert native_step.file_path.name == "native_support.pxd"


def test_workspace_index_workspace_symbols_and_references_follow_resolved_symbol(tmp_path: Path) -> None:
    root = tmp_path / "src"
    _write_module(root, "pkg/__init__.py", "")
    _write_module(root, "pkg/helpers.py", "def helper(value):\n    return value\n")
    _write_module(root, "pkg/consumer.py", "from pkg.helpers import helper\n\nresult = helper(4)\n")

    index = WorkspaceIndex([root], (), True)
    index.build()

    symbols = index.workspace_symbols("helper")
    assert any(item["name"] == "helper" and str(item["location"]["uri"]).endswith("helpers.py") for item in symbols)

    consumer = index.modules["pkg.consumer"]
    references = index.reference_locations(consumer, "helper", include_declaration=True)
    uris = {str(location["uri"]) for location in references}
    assert any(uri.endswith("helpers.py") for uri in uris)
    assert any(uri.endswith("consumer.py") for uri in uris)

    rename_edits = index.rename_edits(consumer, "helper", "renamed_helper")
    assert any(uri.endswith("helpers.py") for uri in rename_edits)
    assert any(uri.endswith("consumer.py") for uri in rename_edits)


def test_workspace_index_builds_python_like_sage_modules_and_resolves_imports(tmp_path: Path) -> None:
    root = tmp_path / "src"
    _write_module(root, "pkg/__init__.py", "")
    _write_module(root, "pkg/helpers.py", 'def helper(value):\n    """Return a scaled helper value."""\n    return value * 2\n')
    _write_module(
        root,
        "pkg/consumer.sage",
        "from pkg.helpers import helper\n\nclass Solver:\n    def compute(self, value):\n        return helper(value^2)\n\nresult = helper(4)\n",
    )

    index = WorkspaceIndex([root], (), True)
    index.build()

    consumer = index.modules["pkg.consumer"]
    helper = index.resolve_symbol(consumer, "helper")
    documentation = index.documentation_for_symbol(consumer, "helper")
    symbols = index.document_symbols(consumer)

    assert helper is not None
    assert helper.file_path.name == "helpers.py"
    assert documentation is not None
    assert documentation.summary == "Return a scaled helper value."
    assert any(item["name"] == "Solver" for item in symbols)
    assert any(item["name"] == "result" for item in symbols)


def test_workspace_index_injects_default_sage_imports_without_full_build() -> None:
    index = WorkspaceIndex([FIXTURE_ROOT], (), True)
    record = parse_module("document::example", Path("example.sage"), "value = sqrt(4)\n")

    index._inject_default_sage_imports(record)
    resolved = index.resolve_symbol(record, "sqrt")

    assert "sage.all" in record.star_imports
    assert resolved is not None
    assert resolved.file_path.name == "other.py"


def test_workspace_index_keeps_method_structure_for_preparser_sage_modules(tmp_path: Path) -> None:
    root = tmp_path / "src"
    _write_module(root, "pkg/__init__.py", "")
    _write_module(
        root,
        "pkg/polys.sage",
        "pring.<x> = QQ[]\n\nclass PolyWorker:\n    def square(self, value):\n        return value^2\n\nworker = PolyWorker()\nresult = worker.square(x + 1)\n",
    )

    index = WorkspaceIndex([root], (), True)
    index.build()

    record = index.modules["pkg.polys"]
    symbols = index.document_symbols(record)
    completions = index.member_completion_items(record, "worker", "sq")

    assert any(item["name"] == "pring" for item in symbols)
    assert any(item["name"] == "x" for item in symbols)
    assert any(item["name"] == "PolyWorker" for item in symbols)
    assert any(item["name"] == "worker" for item in symbols)
    assert any(item["label"] == "square" for item in completions)


def test_workspace_index_reuses_persistent_cache_for_unchanged_modules(tmp_path: Path, monkeypatch) -> None:
    root = tmp_path / "src"
    cache_dir = tmp_path / "cache"
    _write_module(root, "pkg/__init__.py", "")
    _write_module(root, "pkg/helpers.py", "def helper(value):\n    return value\n")

    index = WorkspaceIndex([root], (), True, cache_dir=cache_dir)
    index.build()

    def _fail_parse(*args, **kwargs):
        raise AssertionError("parse_module should not run when the persistent cache is warm")

    monkeypatch.setattr("sage_lsp.index.parse_module", _fail_parse)

    cached_index = WorkspaceIndex([root], (), True, cache_dir=cache_dir)
    cached_index.build()

    assert "pkg.helpers" in cached_index.modules


def test_workspace_index_reuses_cached_source_for_unchanged_modules(tmp_path: Path, monkeypatch) -> None:
    root = tmp_path / "src"
    cache_dir = tmp_path / "cache"
    _write_module(root, "pkg/__init__.py", "")
    _write_module(root, "pkg/helpers.py", "def helper(value):\n    return value\n")

    index = WorkspaceIndex([root], (), True, cache_dir=cache_dir)
    index.build()

    def _fail_read(path: Path) -> str:
        raise AssertionError("module source should be reused from cache when the file fingerprint is unchanged")

    monkeypatch.setattr(WorkspaceIndex, "_read_module_source", lambda self, path: _fail_read(path))

    cached_index = WorkspaceIndex([root], (), True, cache_dir=cache_dir)
    cached_index.build()

    assert "pkg.helpers" in cached_index.modules


def test_workspace_index_hydrates_from_cache_without_scanning(tmp_path: Path, monkeypatch) -> None:
    root = tmp_path / "src"
    cache_dir = tmp_path / "cache"
    _write_module(root, "pkg/__init__.py", "")
    _write_module(root, "pkg/helpers.py", "def helper(value):\n    return value\n")

    index = WorkspaceIndex([root], (), True, cache_dir=cache_dir)
    index.build()

    def _fail_read(path: Path) -> str:
        raise AssertionError("hydrate_from_cache should not reread module source")

    def _fail_parse(*args, **kwargs):
        raise AssertionError("hydrate_from_cache should not reparse cached modules")

    monkeypatch.setattr(WorkspaceIndex, "_read_module_source", lambda self, path: _fail_read(path))
    monkeypatch.setattr("sage_lsp.index.parse_module", _fail_parse)

    hydrated_index = WorkspaceIndex([root], (), True, cache_dir=cache_dir)

    assert hydrated_index.hydrate_from_cache() is True
    assert "pkg.helpers" in hydrated_index.modules


def test_workspace_index_ensure_full_index_loads_only_missing_roots(tmp_path: Path, monkeypatch) -> None:
    first_root = tmp_path / "first"
    second_root = tmp_path / "second"
    cache_dir = tmp_path / "cache"
    _write_module(first_root, "pkg/__init__.py", "")
    _write_module(first_root, "pkg/helpers.py", "def helper(value):\n    return value\n")
    _write_module(second_root, "lib/__init__.py", "")
    _write_module(second_root, "lib/other.py", "def other(value):\n    return value + 1\n")

    index = WorkspaceIndex([first_root, second_root], (), True, cache_dir=cache_dir)
    index.load_roots([first_root])

    def _fail_build() -> None:
        raise AssertionError("ensure_full_index should load only missing roots instead of rebuilding everything")

    monkeypatch.setattr(index, "build", _fail_build)

    index.ensure_full_index()

    assert "pkg.helpers" in index.modules
    assert "lib.other" in index.modules

    hydrated_index = WorkspaceIndex([first_root, second_root], (), True, cache_dir=cache_dir)
    assert hydrated_index.hydrate_from_cache() is True
    assert "pkg.helpers" in hydrated_index.modules
    assert "lib.other" in hydrated_index.modules


def test_workspace_symbols_query_loads_only_matching_deferred_modules(tmp_path: Path, monkeypatch) -> None:
    first_root = tmp_path / "first"
    second_root = tmp_path / "second"
    _write_module(first_root, "seed.py", "value = 1\n")
    _write_module(second_root, "pkg/__init__.py", "")
    _write_module(second_root, "pkg/helpers.py", "def helper(value):\n    return value\n")
    _write_module(second_root, "pkg/other.py", "def unrelated(value):\n    return value + 1\n")

    index = WorkspaceIndex([first_root, second_root], (), True)
    index.load_roots([first_root])

    def _fail_build() -> None:
        raise AssertionError("workspace_symbols(query) should use targeted deferred search instead of full rebuild")

    monkeypatch.setattr(index, "build", _fail_build)

    symbols = index.workspace_symbols("helper")

    assert any(item["name"] == "helper" for item in symbols)
    assert "pkg.helpers" in index.modules
    assert "pkg.other" not in index.modules


def test_workspace_index_does_not_persist_partial_lazy_snapshot(tmp_path: Path) -> None:
    root = tmp_path / "src"
    cache_dir = tmp_path / "cache"
    _write_module(root, "pkg/__init__.py", "")
    _write_module(root, "pkg/helpers.py", "def helper(value):\n    return value\n")

    index = WorkspaceIndex([root], (), True, cache_dir=cache_dir)
    record = parse_module(
        "document::consumer",
        Path("consumer.py"),
        "from pkg.helpers import helper\n\nvalue = helper(4)\n",
    )

    helper = index.resolve_symbol(record, "helper")

    assert helper is not None
    hydrated_index = WorkspaceIndex([root], (), True, cache_dir=cache_dir)
    assert hydrated_index.hydrate_from_cache() is False
    assert "pkg.helpers" not in hydrated_index.modules


def test_import_candidates_query_loads_only_matching_deferred_modules(tmp_path: Path, monkeypatch) -> None:
    first_root = tmp_path / "first"
    second_root = tmp_path / "second"
    _write_module(first_root, "seed.py", "value = 1\n")
    _write_module(second_root, "pkg/__init__.py", "")
    _write_module(second_root, "pkg/helpers.py", "def helper(value):\n    return value\n")
    _write_module(second_root, "pkg/other.py", "def unrelated(value):\n    return value + 1\n")

    index = WorkspaceIndex([first_root, second_root], (), True)
    index.load_roots([first_root])

    def _fail_build() -> None:
        raise AssertionError("import_candidates(name) should use targeted deferred search instead of full rebuild")

    monkeypatch.setattr(index, "build", _fail_build)

    candidates = index.import_candidates("helper")

    assert candidates == ["pkg.helpers"]
    assert "pkg.helpers" in index.modules
    assert "pkg.other" not in index.modules


def test_import_candidates_handles_cyclic_star_imports_during_targeted_search(tmp_path: Path) -> None:
    root = tmp_path / "src"
    _write_module(root, "pkg/__init__.py", "")
    _write_module(root, "pkg/a.py", "from pkg.b import *\n\ndef helper(value):\n    return value\n")
    _write_module(root, "pkg/b.py", "from pkg.a import *\n\ndef other(value):\n    return value + 1\n")

    index = WorkspaceIndex([root], (), True)

    candidates = index.import_candidates("helper")

    assert "pkg.a" in candidates


def test_workspace_index_invalidates_persistent_cache_when_source_changes(tmp_path: Path, monkeypatch) -> None:
    root = tmp_path / "src"
    cache_dir = tmp_path / "cache"
    _write_module(root, "pkg/__init__.py", "")
    helpers_path = _write_module(root, "pkg/helpers.py", "def helper(value):\n    return value\n")

    index = WorkspaceIndex([root], (), True, cache_dir=cache_dir)
    index.build()

    helpers_path.write_text('def helper(value):\n    """Updated."""\n    return value + 1\n', encoding="utf-8")

    observed_paths: list[str] = []
    original_parse_module = parse_module

    def _track_parse(*args, **kwargs):
        observed_paths.append(args[1].name)
        return original_parse_module(*args, **kwargs)

    monkeypatch.setattr("sage_lsp.index.parse_module", _track_parse)

    rebuilt_index = WorkspaceIndex([root], (), True, cache_dir=cache_dir)
    rebuilt_index.build()

    assert "helpers.py" in observed_paths
    documentation = rebuilt_index.documentation_for_symbol(rebuilt_index.modules["pkg.helpers"], "helper")
    assert documentation is not None
    assert documentation.summary == "Updated."


def test_workspace_index_refresh_path_updates_saved_module_without_rebuild(tmp_path: Path) -> None:
    root = tmp_path / "src"
    _write_module(root, "pkg/__init__.py", "")
    helpers_path = _write_module(root, "pkg/helpers.py", "def helper(value):\n    return value\n")

    index = WorkspaceIndex([root], (), True)
    index.build()

    helpers_path.write_text('def helper(value):\n    """Refreshed."""\n    return value + 1\n', encoding="utf-8")

    refreshed = index.refresh_path(helpers_path)

    assert refreshed is not None
    documentation = index.documentation_for_symbol(refreshed, "helper")
    assert documentation is not None
    assert documentation.summary == "Refreshed."


def test_workspace_index_remove_path_preserves_other_module_components(tmp_path: Path) -> None:
    root = tmp_path / "src"
    _write_module(root, "sage/__init__.py", "")
    _write_module(root, "sage/rings/__init__.py", "")
    native_support_pxd = _write_module(
        root,
        "sage/rings/native_support.pxd",
        '"""Native declarations."""\n\ncdef class NativeAccumulator:\n    pass\n\ncpdef int native_step(int value)\n',
    )
    native_support_pyx = _write_module(
        root,
        "sage/rings/native_support.pyx",
        '"""Native implementation."""\n\ncdef class NativeAccumulator:\n    pass\n\nZZ = NativeAccumulator()\n',
    )

    index = WorkspaceIndex([root], (), True)
    index.build()
    index.remove_path(native_support_pyx)

    record = index.module_for_path(native_support_pxd)

    assert record is not None
    assert record.file_path.name == "native_support.pxd"
    assert "NativeAccumulator" in record.symbols
    assert "native_step" in record.symbols
    assert "ZZ" not in record.symbols


def test_workspace_index_batches_cache_persistence_for_multiple_path_changes(tmp_path: Path, monkeypatch) -> None:
    root = tmp_path / "src"
    _write_module(root, "pkg/__init__.py", "")
    first_path = _write_module(root, "pkg/first.py", "def first():\n    return 1\n")
    second_path = _write_module(root, "pkg/second.py", "def second():\n    return 2\n")

    index = WorkspaceIndex([root], (), True)
    index.build()

    first_path.write_text("def first():\n    return 10\n", encoding="utf-8")
    second_path.unlink()

    write_calls: list[int] = []
    original_write_cached_entries = index._write_cached_entries  # noqa: SLF001 - intentional state verification

    def _track_write(entries):
        write_calls.append(len(entries))
        return original_write_cached_entries(entries)

    monkeypatch.setattr(index, "_write_cached_entries", _track_write)

    index.refresh_or_remove_paths(
        [
            (first_path, False),
            (second_path, True),
        ]
    )

    assert len(write_calls) == 1
    assert "pkg.first" in index.modules
    assert "pkg.second" not in index.modules


def test_workspace_index_reuses_open_document_overlay_for_unchanged_source(monkeypatch) -> None:
    index = WorkspaceIndex([], (), True)
    uri = Path("/workspace/example.sage").as_uri()
    source = "value = ZZ\nsqrt(4)\n"

    first = index.parse_document(uri, source, "sagemath")

    def _fail_parse(*args, **kwargs):
        raise AssertionError("parse_module should not run again for an unchanged open document")

    monkeypatch.setattr("sage_lsp.index.parse_module", _fail_parse)

    second = index.parse_document(uri, source, "sagemath")

    assert second is first


def test_workspace_index_invalidates_open_document_overlay_when_source_changes(monkeypatch) -> None:
    index = WorkspaceIndex([], (), True)
    uri = Path("/workspace/example.sage").as_uri()
    first_source = "value = ZZ\n"
    second_source = "result = QQ\n"

    index.parse_document(uri, first_source, "sagemath")

    observed_sources: list[str] = []
    original_parse_module = parse_module

    def _track_parse(*args, **kwargs):
        observed_sources.append(args[2])
        return original_parse_module(*args, **kwargs)

    monkeypatch.setattr("sage_lsp.index.parse_module", _track_parse)

    updated = index.parse_document(uri, second_source, "sagemath")

    assert second_source in observed_sources
    assert "result" in updated.symbols
    assert "value" not in updated.symbols


def test_workspace_index_diagnostics_report_unresolved_imports() -> None:
    index = WorkspaceIndex([], (), True)
    record = parse_module(
        "document::broken",
        Path("broken.py"),
        "from missing.module import helper\nimport also_missing\n",
    )

    diagnostics = index.diagnostics_for_record(record)

    assert any("missing.module" in entry["message"] for entry in diagnostics)
    assert any("also_missing" in entry["message"] for entry in diagnostics)


def test_workspace_index_import_candidates_prefer_defining_modules() -> None:
    index = WorkspaceIndex([FIXTURE_ROOT], (), True)
    index.build()

    candidates = index.import_candidates("sqrt")

    assert candidates[:2] == ["sage.functions.other", "sage.all"]


def test_workspace_index_diagnostics_report_python_syntax_errors() -> None:
    index = WorkspaceIndex([], (), True)
    record = parse_module(
        "document::broken_python",
        Path("broken.py"),
        "def broken(:\n    return 1\n",
    )

    diagnostics = index.diagnostics_for_record(record)

    assert any(entry["message"].startswith("Syntax error:") for entry in diagnostics)


def test_workspace_index_diagnostics_allow_valid_sage_preparser_syntax() -> None:
    index = WorkspaceIndex([], (), True)
    record = parse_module(
        "document::valid_sage",
        Path("valid.sage"),
        "R.<x> = PolynomialRing(QQ)\nvalue = x^2 + 1\n",
    )

    diagnostics = index.diagnostics_for_record(record)

    assert diagnostics == []


def test_workspace_index_diagnostics_report_invalid_sage_syntax() -> None:
    index = WorkspaceIndex([], (), True)
    record = parse_module(
        "document::invalid_sage",
        Path("invalid.sage"),
        "R.<x> = PolynomialRing(QQ)\nif True print(x)\n",
    )

    diagnostics = index.diagnostics_for_record(record)

    assert any(entry["message"].startswith("Syntax error:") for entry in diagnostics)


def test_iter_identifier_ranges_skips_comments_and_strings() -> None:
    ranges = iter_identifier_ranges(
        'helper = 1\nprint(helper)\n# helper\ntext = "helper"\n',
        "helper",
    )

    assert len(ranges) == 2


def test_workspace_index_resolves_singleton_dotted_members_and_member_completion(tmp_path: Path) -> None:
    root = tmp_path / "src"
    _write_module(root, "pkg/__init__.py", "")
    _write_module(
        root,
        "pkg/smallgraphs.py",
        'def PetersenGraph():\n    """Build the Petersen graph."""\n    return 10\n',
    )
    _write_module(
        root,
        "pkg/graph_generators.py",
        "class GraphGenerators:\n    from pkg import smallgraphs\n\n    PetersenGraph = staticmethod(smallgraphs.PetersenGraph)\n\n    def CycleGraph(self, n):\n        \"\"\"Build a cycle graph.\"\"\"\n        return n\n\ngraphs = GraphGenerators()\n",
    )
    _write_module(root, "pkg/consumer.py", "from pkg.graph_generators import graphs\n\nvalue = graphs.PetersenGraph()\n")

    index = WorkspaceIndex([root], (), True)
    index.build()

    consumer = index.modules["pkg.consumer"]
    resolved = index.resolve_symbol(consumer, "graphs.PetersenGraph")
    documentation = index.documentation_for_symbol(consumer, "graphs.PetersenGraph")
    completions = index.member_completion_items(consumer, "graphs", "Cy")
    symbols = index.workspace_symbols("PetersenGraph")

    assert resolved is not None
    assert resolved.file_path.name == "smallgraphs.py"
    assert documentation is not None
    assert documentation.name == "PetersenGraph"
    assert documentation.summary == "Build the Petersen graph."
    assert any(item["label"] == "CycleGraph" for item in completions)
    assert any(str(item["location"]["uri"]).endswith("smallgraphs.py") for item in symbols)


def test_workspace_index_uses_factory_class_docstrings_for_assigned_callables(tmp_path: Path) -> None:
    root = tmp_path / "src"
    _write_module(root, "pkg/__init__.py", "")
    _write_module(
        root,
        "pkg/constructors.py",
        'class EllipticCurveFactory:\n    """Construct an elliptic curve."""\n\n    def __call__(self, *args, **kwargs):\n        return args, kwargs\n\nEllipticCurve = EllipticCurveFactory()\n',
    )
    _write_module(root, "pkg/consumer.py", "from pkg.constructors import EllipticCurve\n\nvalue = EllipticCurve([0, 0, 1, -1, 0])\n")

    index = WorkspaceIndex([root], (), True)
    index.build()

    consumer = index.modules["pkg.consumer"]
    documentation = index.documentation_for_symbol(consumer, "EllipticCurve")
    definition = index.resolve_symbol(consumer, "EllipticCurve")

    assert documentation is not None
    assert documentation.docstring == "Construct an elliptic curve."
    assert documentation.summary == "Construct an elliptic curve."
    assert definition is not None
    assert definition.file_path.name == "constructors.py"


def test_workspace_index_uses_pyx_function_docstrings_for_documentation(tmp_path: Path) -> None:
    root = tmp_path / "src"
    _write_module(root, "sage/__init__.py", "")
    _write_module(root, "sage/matrix/__init__.py", "")
    _write_module(
        root,
        "sage/matrix/constructor.pyx",
        'def matrix(*args, **kwds):\n    """\n    Create a matrix.\n    """\n    return args, kwds\n',
    )
    _write_module(root, "sage/all.py", "from sage.matrix.constructor import matrix\n")

    index = WorkspaceIndex([root], (), True)
    index.build()

    module = index.modules["sage.all"]
    documentation = index.documentation_for_symbol(module, "matrix")

    assert documentation is not None
    assert documentation.summary == "Create a matrix."


def _write_module(root: Path, relative_path: str, contents: str) -> Path:
    module_path = root / relative_path
    module_path.parent.mkdir(parents=True, exist_ok=True)
    module_path.write_text(contents, encoding="utf-8")
    return module_path
