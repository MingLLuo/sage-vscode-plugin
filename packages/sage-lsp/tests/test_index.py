from pathlib import Path

from sage_lsp.index import WorkspaceIndex, path_from_uri


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


def _write_module(root: Path, relative_path: str, contents: str) -> None:
    module_path = root / relative_path
    module_path.parent.mkdir(parents=True, exist_ok=True)
    module_path.write_text(contents, encoding="utf-8")
