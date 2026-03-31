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


def test_parse_pyx_module_extracts_function_docstrings() -> None:
    source = '''
def matrix(*args, **kwds):
    """
    Create a matrix.
    """
    return args, kwds
'''
    record = parse_module("sage.matrix.constructor", Path("constructor.pyx"), source)

    assert record.symbols["matrix"].docstring == "Create a matrix."


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
R.<x, y> = PolynomialRing(QQ)
helper = factorial
"""
    record = parse_module("document::example", Path("example.sage"), source)

    assert "factorial" in record.bindings
    assert "R" in record.symbols
    assert "x" in record.symbols
    assert "y" in record.symbols
    assert "helper" in record.symbols


def test_parse_python_like_sage_module_uses_ast_path_and_tracks_members() -> None:
    source = """
from pkg.helpers import helper


class Solver:
    def compute(self, value):
        return helper(value^2)


result = Solver().compute(3)
"""
    record = parse_module("document::example", Path("example.sage"), source)

    assert record.language == "sage"
    assert "helper" in record.bindings
    assert "Solver" in record.symbols
    assert "result" in record.symbols
    assert record.member_symbols["Solver"]["compute"].kind == "function"
    assert record.diagnostics == []


def test_parse_preparser_sage_module_merges_loose_and_ast_results() -> None:
    source = """
pring.<x> = QQ[]


class PolyWorker:
    def square(self, value):
        return value^2


worker = PolyWorker()
"""
    record = parse_module("document::example", Path("example.sage"), source)

    assert record.language == "sage"
    assert "pring" in record.symbols
    assert "x" in record.symbols
    assert "PolyWorker" in record.symbols
    assert "worker" in record.symbols
    assert record.member_symbols["PolyWorker"]["square"].kind == "function"
    assert record.instance_types["worker"] == "PolyWorker"
    assert record.diagnostics == []


def test_parse_python_module_extracts_singleton_member_bindings() -> None:
    source = """
class GraphGenerators:
    from pkg import smallgraphs

    PetersenGraph = staticmethod(smallgraphs.PetersenGraph)

    def CycleGraph(self, n):
        return n

graphs = GraphGenerators()
"""
    record = parse_python_module("pkg.graph_generators", Path("graph_generators.py"), source)

    assert record.member_bindings["GraphGenerators"]["PetersenGraph"].module_name == "pkg.smallgraphs"
    assert record.member_bindings["GraphGenerators"]["PetersenGraph"].target_name == "PetersenGraph"
    assert record.member_symbols["GraphGenerators"]["CycleGraph"].kind == "function"
    assert record.instance_types["graphs"] == "GraphGenerators"
