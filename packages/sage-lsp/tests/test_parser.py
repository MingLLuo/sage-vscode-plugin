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
