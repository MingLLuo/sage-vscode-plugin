"""
Highlighting-focused Sage examples for structure-heavy code.
"""

from sage.misc.cachefunc import cached_method
from sage.misc.lazy_import import lazy_import
from sage.schemes.toric.chow_group import ChowGroup

lazy_import("sage.plot.plot", "plot")

X = toric_varieties.P2()
A = ChowGroup(X)

R.<u, v> = PolynomialRing(QQ, 2)
S.<t> = NumberField(u^2 + 1)
M = FreeModule(ZZ, 3)
W = FilteredSimplicialComplex([[0], [1], [0, 1]])
theta = var("theta")
curve_plot = plot(sin(theta), (theta, 0, 2*pi), color="steelblue", legend_label="sin(theta)")
binary_code = codes.HammingCode(GF(2), 3)
root_lattice = RootSystem(["A", 2]).ambient_space()
square_table = [n^2 for n in [1..5]]


@interact
def explore_degree(bound=slider(1, 8, default=3), normalized=checkbox(default=True)):
    return bound if normalized else -bound


class DemoInvariantFamily:
    @cached_method
    def hilbert_data(self):
        return [
            A.degree(),
            M.rank(),
            W.f_vector(),
        ]


highlight_targets = [
    X,
    A,
    R,
    S,
    M,
    W,
    curve_plot,
    binary_code,
    root_lattice,
    square_table,
    DemoInvariantFamily().hilbert_data(),
    explore_degree,
]
