"""
Advanced Sage runtime smoke cases for docs, definitions, and navigation.
"""

G = graphs.PetersenGraph()
chromatic_number_result = G.chromatic_number()
automorphism_group_result = G.automorphism_group()

E = EllipticCurve([0, 0, 1, -1, 0])
rank_result = E.rank()
torsion_points = E.torsion_subgroup()

R.<x, y> = PolynomialRing(QQ, 2)
unit_circle = ideal([x^2 + y^2 - 1, x^3 - y])
groebner_basis_result = unit_circle.groebner_basis()

factor_result = factor(x^8 - 1)
cyclotomic_result = cyclotomic_polynomial(12)
sum_of_divisors_result = sigma(60)

advanced_runtime_targets = [
    chromatic_number_result,
    automorphism_group_result,
    rank_result,
    torsion_points,
    groebner_basis_result,
    factor_result,
    cyclotomic_result,
    sum_of_divisors_result,
]
