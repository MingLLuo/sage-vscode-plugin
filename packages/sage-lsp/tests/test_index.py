from pathlib import Path

from sage_lsp.index import WorkspaceIndex


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
