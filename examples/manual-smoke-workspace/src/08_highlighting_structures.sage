"""
Highlighting-focused Sage examples for structure-heavy code.
"""

from sage.misc.cachefunc import cached_method
from sage.schemes.toric.chow_group import ChowGroup

X = toric_varieties.P2()
A = ChowGroup(X)

R.<u, v> = PolynomialRing(QQ, 2)
S.<t> = NumberField(u^2 + 1)
M = FreeModule(ZZ, 3)
W = FilteredSimplicialComplex([[0], [1], [0, 1]])


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
    DemoInvariantFamily().hilbert_data(),
]
