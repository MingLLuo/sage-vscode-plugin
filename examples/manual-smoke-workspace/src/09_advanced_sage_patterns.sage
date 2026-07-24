"""
Advanced Sage patterns collected from algebra, coding theory, and
arithmetic-geometry workflows.
"""

from sage.misc.cachefunc import cached_function

P.<w> = PolynomialRing(QQ)
K.<i> = NumberField(w^2 + 1, embedding=CC.gen())

R.<x, y, z> = PolynomialRing(GF(127), 3, order="degrevlex")
preparser_ring_generator = R.gen()
preparser_polynomial_degree = x.degree()
I = ideal([
    x^2 + y*z - 1,
    y^2 - z + x,
    z^3 + x*y - 3,
])
groebner_data = I.groebner_basis()
quotient_ring = R.quotient(I, names=("xb", "yb", "zb"))

F.<a> = GF(2^8, name="a")
preparser_field_parent = a.parent()
linear_layer = matrix(F, 4, 4, lambda r, c: a^(r + 2*c))
branch_number_probe = min(
    vector(F, row).hamming_weight() + (linear_layer * vector(F, row)).hamming_weight()
    for row in cartesian_product([F.list()[1:8]] * 4)
)

E = EllipticCurve(GF(431), [0, 1])
torsion_profile = [
    P.order()
    for P in E.points()
    if P != E(0) and P.order().divides(430)
]

Kfun.<t> = FunctionField(GF(5))
S.<Y> = Kfun[]
preparser_series_generator = S.gen()
preparser_series_degree = Y.degree()
cover_polynomial = Y^3 + t*Y + 1
cover_invariants = {
    "disc": cover_polynomial.discriminant(),
    "degree": cover_polynomial.degree(),
    "monic": cover_polynomial.is_monic(),
}

P2.<X, Y, Z> = ProjectiveSpace(QQ, 2)
curve_equation = X^3 + Y^3 + Z^3 - 3*X*Y*Z
singular_locus = ideal([
    curve_equation.derivative(X),
    curve_equation.derivative(Y),
    curve_equation.derivative(Z),
]).groebner_basis()

C = codes.HammingCode(GF(2), 3)
parity_checks = matrix(GF(2), C.parity_check_matrix())
dual_dimension = parity_checks.right_kernel().dimension()


@cached_function
def trace_window(poly, base_ring=QQ, *, width=5, normalize=True):
    values = vector(base_ring, [poly(k) for k in [1..width]])
    return values / values.norm() if normalize else values


advanced_sage_targets = [
    groebner_data,
    quotient_ring.gens(),
    branch_number_probe,
    torsion_profile,
    cover_invariants,
    singular_locus,
    dual_dimension,
    trace_window(w^2 + 3*w + 1, width=7),
]

number_field_generator_polynomial = i.polynomial()
