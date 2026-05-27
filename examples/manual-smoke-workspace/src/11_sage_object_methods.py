"""
Sage object-method patterns for editor smoke testing.

The examples here cover common Sage constructors whose methods are implemented
across several internal modules.  They are intentionally lightweight and are
not meant to be executed during tests.
"""

from sage.all import (
    DiGraph,
    EllipticCurve,
    GF,
    Graph,
    NumberField,
    PolynomialRing,
    QQ,
    QQbar,
)


def graph_walk_report():
    """Collect navigation-oriented graph facts."""
    graph = Graph([(0, 1), (1, 2), (2, 0)])
    digraph = DiGraph({0: [1], 1: [2], 2: [0]})
    return {
        "vertices": graph.vertices(),
        "neighbors": graph.neighbors(1),
        "edges": graph.edges(labels=False),
        "degree": graph.degree(1),
        "connected": graph.is_connected(),
        "path": digraph.shortest_path(0, 2),
        "adjacency": graph.adjacency_matrix(),
        "plot": graph.plot(),
    }


def elliptic_curve_report():
    """Exercise finite-field and rational elliptic curve methods."""
    finite_curve = EllipticCurve(GF(431), [0, 1])
    rational_curve = EllipticCurve([0, 0, 1, -7, 6])
    return {
        "base_ring": finite_curve.base_ring(),
        "points": finite_curve.points(),
        "cardinality": finite_curve.cardinality(),
        "order": finite_curve.order(),
        "torsion": finite_curve.torsion_subgroup(3),
        "rank": rational_curve.rank(),
        "gens": rational_curve.gens(),
        "integral_points": rational_curve.integral_points(),
        "plot": rational_curve.plot(),
    }


def number_field_report():
    """Exercise number-field methods used in algebraic computations."""
    ring = PolynomialRing(QQ, "x")
    x = ring.gen()
    field = NumberField(x**2 + 1, "a")
    return {
        "generator": field.gen(),
        "generators": field.gens(),
        "degree": field.degree(),
        "absolute_degree": field.absolute_degree(),
        "relative_degree": field.relative_degree(),
        "discriminant": field.discriminant(),
        "signature": field.signature(),
        "integers": field.ring_of_integers(),
        "embeddings": field.embeddings(QQbar),
        "places": field.places(),
        "class_group": field.class_group(),
        "unit_group": field.unit_group(),
    }
