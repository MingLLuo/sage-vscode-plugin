"""
Advanced symbolic and combinatorics examples for manual smoke testing.
"""

var("a b t")
assume(a > 0)
assume(b > 0)

symbolic_integral = integrate(exp(-t^2), t)
series_result = sin(t).series(t, 6)
limit_result = limit((1 + 1 / t)^t, t=oo)

partitions_of_five = Partitions(5)
partition_cardinality = partitions_of_five.cardinality()

permutation_group = SymmetricGroup(4)
permutation_generators = permutation_group.gens()

Q.<z> = PolynomialRing(QQ)
cyclotomic_field = NumberField(z^4 + 1, "w")
field_generator = cyclotomic_field.gen()
field_norm = field_generator.norm()

advanced_symbolic_targets = [
    symbolic_integral,
    series_result,
    limit_result,
    partition_cardinality,
    permutation_generators,
    field_norm,
]
