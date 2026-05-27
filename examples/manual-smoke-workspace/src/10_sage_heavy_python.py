"""
Synthetic Sage-heavy Python fixture for editor smoke testing.

The file is intentionally small and artificial. It exercises common Sage
constructors, matrix methods, polynomial-ring methods, polynomial-element
methods, and ideal methods without copying private or third-party project code.
"""

from sage.all import (
    GF,
    PolynomialRing,
    matrix,
    vector,
    zero_matrix,
    zero_vector,
)


def build_full_rank_matrix(field, rows, cols, rank=None):
    """Build a deterministic matrix until it reaches the requested rank."""
    target_rank = min(rows, cols) if rank is None else int(rank)
    mat = matrix(field, rows, cols, lambda i, j: field(i + 2 * j + 1))
    while mat.rank() < target_rank:
        mat = matrix(field, rows, cols, lambda i, j: field(i * j + 3))
    return mat


def build_quadratic_equations(data, ring, z_polys, values):
    """Return toy quadratic equations over the supplied polynomial ring."""
    equations = []
    for qmat, value in zip(data["quadratic_forms"], values):
        poly = sum(
            ring(qmat[a, b]) * z_polys[a] * z_polys[b]
            for a in range(data["ambient_rank"])
            for b in range(data["ambient_rank"])
        )
        equations.append(poly - ring(value))
    return equations


def build_affine_system(data, values):
    """Create an affine slice, polynomial ring, equations, and Jacobian."""
    field, variables, ambient_rank = data["field"], data["variables"], data["ambient_rank"]
    codim = ambient_rank - variables
    constraint_matrix = (
        build_full_rank_matrix(field, codim, ambient_rank, rank=codim)
        if codim
        else zero_matrix(field, 0, ambient_rank)
    )
    target_vector = vector(field, values[:codim]) if codim else zero_vector(field, 0)
    base_point = (
        vector(field, constraint_matrix.solve_right(target_vector))
        if codim
        else zero_vector(field, ambient_rank)
    )
    kernel_vectors = [vector(field, row) for row in constraint_matrix.right_kernel().basis()] if codim else []
    direction_matrix = matrix(
        field,
        ambient_rank,
        variables,
        lambda i, j: kernel_vectors[j][i] if kernel_vectors else field(i == j),
    )
    ring = PolynomialRing(field, names=[f"x{i}" for i in range(variables)], order="degrevlex")
    xs = ring.gens()
    z_polys = [
        ring(base_point[i]) + sum(ring(direction_matrix[i, j]) * xs[j] for j in range(variables))
        for i in range(ambient_rank)
    ]
    equations = build_quadratic_equations(data, ring, z_polys, values)
    jacobian = matrix(ring, len(equations), variables, lambda i, j: equations[i].derivative(xs[j]))
    return {
        "ring": ring,
        "xs": xs,
        "equations": equations,
        "constraints": constraint_matrix,
        "target": target_vector,
        "directions": direction_matrix,
        "jacobian": jacobian,
        "determinant": jacobian.det() if variables else None,
    }


def polynomial_system_report(system):
    """Collect lightweight ideal diagnostics for increasing prefixes."""
    ring = system["ring"]
    rows = []
    for prefix in range(1, len(system["equations"]) + 1):
        ideal = ring.ideal(system["equations"][:prefix])
        rows.append(
            {
                "prefix": prefix,
                "dimension": ideal.dimension(),
                "generators": ideal.gens(),
            }
        )
    return rows


def resultant_projection(equations, variables):
    """Project a toy polynomial system with successive resultants."""
    if len(variables) <= 1:
        return equations
    pivot = equations[0]
    elimination = []
    for poly in equations[1:]:
        elimination.append(pivot.resultant(poly, variables[-1]))
    gcd_poly = elimination[0]
    for poly in elimination[1:]:
        gcd_poly = gcd_poly.gcd(poly)
    return [pivot.derivative(variables[0]), gcd_poly] + elimination


def solve_with_variety(system):
    """Use Sage's ideal variety path as a deterministic root backend."""
    ideal = system["ring"].ideal(system["equations"])
    return ideal.variety(algorithm="msolve", proof=False)


def solve_demo_system(system, seed=0, backend="projection"):
    """Top-level synthetic solver used by navigation and rename smoke tests."""
    report = polynomial_system_report(system)
    if backend == "variety":
        roots = solve_with_variety(system)
    else:
        roots = resultant_projection(system["equations"], system["xs"])
    return {
        "seed": seed,
        "report": report,
        "roots": roots,
        "base_ring": system["ring"].base_ring(),
        "rows": system["jacobian"].rows(),
        "transpose": system["jacobian"].transpose(),
    }


def demo_input():
    """Build a small deterministic Sage-heavy Python input."""
    field = GF(17)
    forms = [
        matrix(field, 4, 4, lambda i, j: field(i + j + offset))
        for offset in range(1, 4)
    ]
    return {
        "field": field,
        "variables": 2,
        "ambient_rank": 4,
        "quadratic_forms": forms,
    }
