from pathlib import Path

from sage_lsp.parser import parse_module, parse_python_module


def test_parse_python_module_extracts_lazy_imports() -> None:
    source = """
from sage.misc.lazy_import import lazy_import

lazy_import("sage.functions.other", ["sqrt", "factorial"], ["root", "factorial"])
"""
    record = parse_python_module("sage.functions.all", Path("all.py"), source)

    assert "root" in record.bindings
    assert record.bindings["root"].module_name == "sage.functions.other"
    assert record.bindings["root"].target_name == "sqrt"
    assert record.bindings["factorial"].is_lazy is True


def test_parse_loose_sage_module_extracts_lazy_import_aliases() -> None:
    source = """
lazy_import("external_series", "alternating_square_sum", "alt_square_sum")
lazy_import("local_docs", ["PolynomialNotebook"], ["NotebookAlias"])
"""
    record = parse_module("document::example", Path("example.sage"), source)

    assert record.bindings["alt_square_sum"].module_name == "external_series"
    assert record.bindings["alt_square_sum"].target_name == "alternating_square_sum"
    assert record.bindings["NotebookAlias"].module_name == "local_docs"
    assert record.bindings["NotebookAlias"].target_name == "PolynomialNotebook"


def test_parse_pyx_module_extracts_top_level_symbols() -> None:
    source = '''
"""
Integer ring fixture.
"""

cdef class IntegerRing:
    pass

ZZ = IntegerRing()
'''
    record = parse_module("sage.rings.integer_ring", Path("integer_ring.pyx"), source)

    assert "IntegerRing" in record.symbols
    assert record.symbols["IntegerRing"].kind == "class"
    assert "ZZ" in record.symbols
    assert record.symbols["ZZ"].kind == "constant"


def test_parse_pyx_module_extracts_cimports_and_typed_functions() -> None:
    source = """
from sage.rings.integer_ring cimport IntegerRing, ZZ as INTEGER_RING
cimport sage.libs.gmp.all as gmp_all

cpdef int native_square(int value):
    return value * value

cdef inline long native_twice(long value):
    return value * 2
"""
    record = parse_module("sage.rings.native_support", Path("native_support.pyx"), source)

    assert record.bindings["IntegerRing"].module_name == "sage.rings.integer_ring"
    assert record.bindings["IntegerRing"].target_name == "IntegerRing"
    assert record.bindings["INTEGER_RING"].target_name == "ZZ"
    assert record.bindings["gmp_all"].target_name is None
    assert "native_square" in record.symbols
    assert "native_twice" in record.symbols


def test_parse_pxi_module_extracts_inline_symbols() -> None:
    source = """
DEF TRACE_LIMIT = 32

cdef inline int included_native_step(int value):
    return value + 1
"""
    record = parse_module("document::native_include", Path("native_include.pxi"), source)

    assert "included_native_step" in record.symbols
    assert record.symbols["included_native_step"].kind == "function"


def test_parse_loose_sage_module_extracts_preparse_assignments_and_imports() -> None:
    source = """
from sage.functions.all import factorial
R.<x> = PolynomialRing(QQ)
helper = factorial
"""
    record = parse_module("document::example", Path("example.sage"), source)

    assert "factorial" in record.bindings
    assert "R" in record.symbols
    assert "x" in record.symbols
    assert "helper" in record.symbols
