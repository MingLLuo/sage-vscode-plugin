use crate::{SageMethodSpec, SageOwnerType};

pub(crate) const SAGE_METHOD_SPECS: &[SageMethodSpec] = &[
    SageMethodSpec {
        owner_type: SageOwnerType::Matrix,
        member: "rank",
        module: "sage.matrix.matrix0",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::Matrix,
        member: "base_ring",
        module: "sage.matrix.matrix0",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::Matrix,
        member: "dimensions",
        module: "sage.matrix.matrix0",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::Matrix,
        member: "list",
        module: "sage.matrix.matrix0",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::Matrix,
        member: "change_ring",
        module: "sage.matrix.matrix0",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::Matrix,
        member: "pivots",
        module: "sage.matrix.matrix0",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::Matrix,
        member: "rows",
        module: "sage.matrix.matrix1",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::Matrix,
        member: "row",
        module: "sage.matrix.matrix1",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::Matrix,
        member: "column",
        module: "sage.matrix.matrix1",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::Matrix,
        member: "augment",
        module: "sage.matrix.matrix1",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::Matrix,
        member: "matrix_from_columns",
        module: "sage.matrix.matrix1",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::Matrix,
        member: "matrix_from_rows",
        module: "sage.matrix.matrix1",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::Matrix,
        member: "matrix_from_rows_and_columns",
        module: "sage.matrix.matrix1",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::Matrix,
        member: "transpose",
        module: "sage.matrix.matrix_dense",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::Matrix,
        member: "solve_right",
        module: "sage.matrix.matrix2",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::Matrix,
        member: "subs",
        module: "sage.matrix.matrix2",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::Matrix,
        member: "right_kernel",
        module: "sage.matrix.matrix2",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::Matrix,
        member: "det",
        module: "sage.matrix.matrix2",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::Matrix,
        member: "inverse",
        module: "sage.matrix.matrix2",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::Matrix,
        member: "column_space",
        module: "sage.matrix.matrix2",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::Matrix,
        member: "charpoly",
        module: "sage.matrix.matrix2",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::Matrix,
        member: "adjugate",
        module: "sage.matrix.matrix2",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::Matrix,
        member: "nrows",
        module: "sage.matrix.matrix0",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::Matrix,
        member: "ncols",
        module: "sage.matrix.matrix0",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::FreeModule,
        member: "basis",
        module: "sage.modules.free_module",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::FreeModule,
        member: "basis_matrix",
        module: "sage.modules.free_module",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::FreeModule,
        member: "dimension",
        module: "sage.modules.free_module",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::FreeModule,
        member: "change_ring",
        module: "sage.modules.free_module",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::PolynomialRing,
        member: "gens",
        module: "sage.structure.parent_gens",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::PolynomialRing,
        member: "gen",
        module: "sage.structure.parent_gens",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::UnivariatePolynomialRing,
        member: "gen",
        module: "sage.rings.polynomial.polynomial_ring",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::MultivariatePolynomialRing,
        member: "gen",
        module: "sage.rings.polynomial.multi_polynomial_libsingular",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::PolynomialRing,
        member: "ideal",
        module: "sage.rings.polynomial.multi_polynomial_libsingular",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::PolynomialRing,
        member: "base_ring",
        module: "sage.structure.category_object",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::PolynomialRing,
        member: "hom",
        module: "sage.structure.parent",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::PolynomialRing,
        member: "lagrange_polynomial",
        module: "sage.rings.polynomial.polynomial_ring",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::PolynomialElement,
        member: "degree",
        module: "sage.rings.polynomial.multi_polynomial_element",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::PolynomialElement,
        member: "factor",
        module: "sage.rings.polynomial.polynomial_element",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::PolynomialElement,
        member: "monic",
        module: "sage.rings.polynomial.polynomial_element",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::PolynomialElement,
        member: "map_coefficients",
        module: "sage.rings.polynomial.polynomial_element",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::PolynomialElement,
        member: "monomial_coefficient",
        module: "sage.rings.polynomial.multi_polynomial_element",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::PolynomialElement,
        member: "constant_coefficient",
        module: "sage.rings.polynomial.multi_polynomial_element",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::PolynomialElement,
        member: "base_ring",
        module: "sage.rings.polynomial.polynomial_element",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::PolynomialElement,
        member: "change_ring",
        module: "sage.rings.polynomial.polynomial_element",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::PolynomialElement,
        member: "list",
        module: "sage.rings.polynomial.polynomial_element_generic",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::PolynomialElement,
        member: "is_zero",
        module: "sage.structure.element",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::PolynomialElement,
        member: "parent",
        module: "sage.structure.element",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::PolynomialElement,
        member: "dict",
        module: "sage.rings.polynomial.polydict",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::PolynomialElement,
        member: "subs",
        module: "sage.rings.polynomial.multi_polynomial_element",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::PolynomialElement,
        member: "total_degree",
        module: "sage.rings.polynomial.multi_polynomial_element",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::PolynomialElement,
        member: "is_constant",
        module: "sage.rings.polynomial.multi_polynomial_element",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::PolynomialElement,
        member: "gcd",
        module: "sage.rings.polynomial.multi_polynomial",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::PolynomialElement,
        member: "resultant",
        module: "sage.rings.polynomial.multi_polynomial_element",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::PolynomialElement,
        member: "derivative",
        module: "sage.rings.polynomial.multi_polynomial",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::PolynomialElement,
        member: "roots",
        module: "sage.rings.polynomial.polynomial_element",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::Ideal,
        member: "dimension",
        module: "sage.rings.polynomial.multi_polynomial_ideal",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::Ideal,
        member: "variety",
        module: "sage.rings.polynomial.multi_polynomial_ideal",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::Field,
        member: "random_element",
        module: "sage.rings.finite_rings.finite_field_base",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::Field,
        member: "order",
        module: "sage.rings.finite_rings.finite_field_base",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::Field,
        member: "from_integer",
        module: "sage.rings.finite_rings.finite_field_base",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::FieldElement,
        member: "parent",
        module: "sage.structure.element",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::FieldElement,
        member: "polynomial",
        module: "sage.rings.finite_rings.element_givaro",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::FieldElement,
        member: "to_integer",
        module: "sage.rings.finite_rings.element_base",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::Vector,
        member: "base_ring",
        module: "sage.structure.element",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::Vector,
        member: "change_ring",
        module: "sage.modules.free_module_element",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::Vector,
        member: "list",
        module: "sage.modules.free_module_element",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::Vector,
        member: "row",
        module: "sage.modules.free_module_element",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::Vector,
        member: "column",
        module: "sage.modules.free_module_element",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::Graph,
        member: "vertices",
        module: "sage.graphs.generic_graph",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::Graph,
        member: "edges",
        module: "sage.graphs.generic_graph",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::Graph,
        member: "neighbors",
        module: "sage.graphs.generic_graph",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::Graph,
        member: "degree",
        module: "sage.graphs.generic_graph",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::Graph,
        member: "shortest_path",
        module: "sage.graphs.generic_graph",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::Graph,
        member: "adjacency_matrix",
        module: "sage.graphs.generic_graph",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::Graph,
        member: "plot",
        module: "sage.graphs.generic_graph",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::Graph,
        member: "is_connected",
        module: "sage.graphs.base.c_graph",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::EllipticCurve,
        member: "base_ring",
        module: "sage.schemes.elliptic_curves.ell_generic",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::EllipticCurve,
        member: "gens",
        module: "sage.schemes.elliptic_curves.ell_generic",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::EllipticCurve,
        member: "plot",
        module: "sage.schemes.elliptic_curves.ell_generic",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::EllipticCurve,
        member: "points",
        module: "sage.schemes.elliptic_curves.ell_finite_field",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::EllipticCurve,
        member: "cardinality",
        module: "sage.schemes.elliptic_curves.ell_finite_field",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::EllipticCurve,
        member: "torsion_subgroup",
        module: "sage.schemes.elliptic_curves.ell_finite_field",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::EllipticCurve,
        member: "rank",
        module: "sage.schemes.elliptic_curves.ell_rational_field",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::EllipticCurve,
        member: "integral_points",
        module: "sage.schemes.elliptic_curves.ell_rational_field",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::NumberField,
        member: "ring_of_integers",
        module: "sage.rings.number_field.number_field_base",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::NumberField,
        member: "degree",
        module: "sage.rings.number_field.number_field",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::NumberField,
        member: "absolute_degree",
        module: "sage.rings.number_field.number_field",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::NumberField,
        member: "relative_degree",
        module: "sage.rings.number_field.number_field",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::NumberField,
        member: "discriminant",
        module: "sage.rings.number_field.number_field",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::NumberField,
        member: "gen",
        module: "sage.rings.number_field.number_field",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::NumberField,
        member: "gens",
        module: "sage.rings.number_field.number_field_rel",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::NumberField,
        member: "embeddings",
        module: "sage.rings.number_field.number_field",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::NumberField,
        member: "places",
        module: "sage.rings.number_field.number_field",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::NumberField,
        member: "signature",
        module: "sage.rings.number_field.number_field",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::NumberField,
        member: "class_group",
        module: "sage.rings.number_field.number_field",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::NumberField,
        member: "unit_group",
        module: "sage.rings.number_field.number_field",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::NumberField,
        member: "is_isomorphic",
        module: "sage.rings.number_field.number_field",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::NumberFieldElement,
        member: "parent",
        module: "sage.structure.element",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::NumberFieldElement,
        member: "polynomial",
        module: "sage.rings.number_field.number_field_element",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::Polyhedron,
        member: "vertices",
        module: "sage.geometry.polyhedron.base0",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::Polyhedron,
        member: "dim",
        module: "sage.geometry.polyhedron.base1",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::Polyhedron,
        member: "contains",
        module: "sage.geometry.polyhedron.base1",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::Polyhedron,
        member: "facets",
        module: "sage.geometry.polyhedron.base3",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::Polyhedron,
        member: "polar",
        module: "sage.geometry.polyhedron.base5",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::Polyhedron,
        member: "intersection",
        module: "sage.geometry.polyhedron.base5",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::Polyhedron,
        member: "plot",
        module: "sage.geometry.polyhedron.base6",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::Polyhedron,
        member: "volume",
        module: "sage.geometry.polyhedron.base7",
    },
];
