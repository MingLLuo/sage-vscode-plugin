//! Sage domain knowledge used by navigation, completion, and type inference.
//!
//! This module deliberately contains type/member catalogs and name hints only.
//! Source-flow inference lives in the neighboring inference modules.

use super::*;

pub fn sage_prewarm_modules_for_source(source: &str) -> Vec<&'static str> {
    let mut owner_types = BTreeSet::new();
    if source.contains("matrix(")
        || source.contains("Matrix(")
        || source.contains("zero_matrix")
        || source.contains(".rank(")
        || source.contains(".det(")
        || source.contains(".solve_right(")
        || source.contains(".right_kernel(")
    {
        owner_types.insert(SageOwnerType::Matrix);
        owner_types.insert(SageOwnerType::FreeModule);
    }
    if source.contains("PolynomialRing")
        || source.contains(".ideal(")
        || source.contains(".gens(")
        || source.contains(".gen(")
    {
        owner_types.insert(SageOwnerType::PolynomialRing);
        owner_types.insert(SageOwnerType::UnivariatePolynomialRing);
        owner_types.insert(SageOwnerType::MultivariatePolynomialRing);
        owner_types.insert(SageOwnerType::PolynomialElement);
        owner_types.insert(SageOwnerType::Ideal);
    }
    if source.contains("GF(")
        || source.contains("FiniteField(")
        || source.contains("NumberField(")
        || source.contains("CyclotomicField(")
        || source.contains("QuadraticField(")
    {
        owner_types.insert(SageOwnerType::Field);
        owner_types.insert(SageOwnerType::FieldElement);
        owner_types.insert(SageOwnerType::NumberField);
        owner_types.insert(SageOwnerType::NumberFieldElement);
    }
    if source.contains("vector(") || source.contains("zero_vector") {
        owner_types.insert(SageOwnerType::Vector);
    }
    if source.contains("Graph(")
        || source.contains("DiGraph(")
        || source.contains("graphs.")
        || source.contains("graphs_")
    {
        owner_types.insert(SageOwnerType::Graph);
    }
    if source.contains("EllipticCurve(") {
        owner_types.insert(SageOwnerType::EllipticCurve);
    }
    if source.contains("Polyhedron(") {
        owner_types.insert(SageOwnerType::Polyhedron);
    }
    if owner_types.is_empty() {
        return Vec::new();
    }
    let mut modules = BTreeSet::new();
    for spec in SAGE_METHOD_SPECS {
        if owner_types.contains(&spec.owner_type) {
            modules.insert(spec.module);
        }
    }
    for spec in SAGE_METHOD_ALIAS_SPECS {
        if owner_types.contains(&spec.owner_type) {
            modules.insert(spec.module);
        }
    }
    modules.into_iter().collect()
}

pub(crate) fn is_known_sage_method(member: &str) -> bool {
    SAGE_METHOD_SPECS.iter().any(|spec| spec.member == member)
        || SAGE_METHOD_ALIAS_SPECS
            .iter()
            .any(|spec| spec.member == member)
}

pub(crate) fn infer_owner_type_from_member_hint(member: &str) -> Option<SageOwnerType> {
    if is_matrix_member(member) {
        return Some(SageOwnerType::Matrix);
    }
    if is_free_module_member(member) {
        return Some(SageOwnerType::FreeModule);
    }
    if is_unique_graph_member(member) {
        return Some(SageOwnerType::Graph);
    }
    None
}

fn is_matrix_member(member: &str) -> bool {
    matches!(
        member,
        "adjugate"
            | "augment"
            | "change_ring"
            | "charpoly"
            | "column"
            | "column_space"
            | "det"
            | "dimensions"
            | "inverse"
            | "matrix_from_columns"
            | "matrix_from_rows"
            | "matrix_from_rows_and_columns"
            | "ncols"
            | "nrows"
            | "pivots"
            | "rank"
            | "right_kernel"
            | "row"
            | "rows"
            | "solve_right"
            | "subs"
            | "transpose"
    )
}

fn is_free_module_member(member: &str) -> bool {
    matches!(member, "basis" | "basis_matrix" | "dimension")
}

fn is_polynomial_element_member(member: &str) -> bool {
    matches!(
        member,
        "base_ring"
            | "constant_coefficient"
            | "degree"
            | "dict"
            | "factor"
            | "gcd"
            | "is_constant"
            | "is_zero"
            | "list"
            | "map_coefficients"
            | "monic"
            | "monomial_coefficient"
            | "parent"
            | "resultant"
            | "roots"
            | "subs"
            | "total_degree"
    )
}

fn is_vector_member(member: &str) -> bool {
    matches!(
        member,
        "base_ring" | "change_ring" | "column" | "list" | "row"
    )
}

pub(super) fn is_field_member(member: &str) -> bool {
    matches!(member, "from_integer" | "order" | "random_element")
}

fn is_field_element_member(member: &str) -> bool {
    matches!(
        member,
        "integer_representation"
            | "parent"
            | "polynomial"
            | "to_integer"
            | "_integer_representation"
    )
}

fn is_graph_member(member: &str) -> bool {
    matches!(
        member,
        "adjacency_matrix"
            | "degree"
            | "edges"
            | "is_connected"
            | "neighbors"
            | "plot"
            | "shortest_path"
            | "vertices"
    )
}

fn is_unique_graph_member(member: &str) -> bool {
    matches!(
        member,
        "adjacency_matrix" | "edges" | "is_connected" | "neighbors" | "shortest_path" | "vertices"
    )
}

fn is_elliptic_curve_member(member: &str) -> bool {
    matches!(
        member,
        "base_ring"
            | "cardinality"
            | "gens"
            | "integral_points"
            | "order"
            | "plot"
            | "points"
            | "rank"
            | "torsion_subgroup"
    )
}

fn is_number_field_member(member: &str) -> bool {
    matches!(
        member,
        "absolute_degree"
            | "base_ring"
            | "class_group"
            | "degree"
            | "discriminant"
            | "embeddings"
            | "gen"
            | "gens"
            | "is_isomorphic"
            | "places"
            | "relative_degree"
            | "ring_of_integers"
            | "signature"
            | "unit_group"
    )
}

pub(super) fn is_matrix_context_member(member: &str) -> bool {
    is_matrix_member(member) || matches!(member, "base_ring" | "list")
}

pub(crate) fn is_sage_namespace_owner(owner: &str) -> bool {
    owner
        .trim()
        .rsplit('.')
        .next()
        .is_some_and(|name| SAGE_STATIC_NAV_NAMESPACES.contains(&name))
}

pub(super) fn sage_constructor_return_type(name: &str) -> Option<SageOwnerType> {
    match name {
        "matrix" | "zero_matrix" | "identity_matrix" | "random_matrix" | "block_matrix" => {
            Some(SageOwnerType::Matrix)
        }
        "vector" | "zero_vector" => Some(SageOwnerType::Vector),
        "GF" | "FiniteField" => Some(SageOwnerType::Field),
        "Graph" | "DiGraph" | "PetersenGraph" | "CompleteGraph" | "CycleGraph" => {
            Some(SageOwnerType::Graph)
        }
        "EllipticCurve" | "EllipticCurve_from_j" | "EllipticCurve_from_c4c6" => {
            Some(SageOwnerType::EllipticCurve)
        }
        "NumberField" | "CyclotomicField" | "QuadraticField" => Some(SageOwnerType::NumberField),
        "Polyhedron" => Some(SageOwnerType::Polyhedron),
        "PolynomialRing"
        | "LaurentPolynomialRing"
        | "PowerSeriesRing"
        | "BooleanPolynomialRing" => Some(SageOwnerType::PolynomialRing),
        _ => None,
    }
}

pub(crate) fn sage_constructor_names_for_owner_type(
    owner_type: SageOwnerType,
) -> &'static [&'static str] {
    match owner_type {
        SageOwnerType::MatrixConstructor | SageOwnerType::Matrix => &[
            "matrix",
            "zero_matrix",
            "identity_matrix",
            "random_matrix",
            "block_matrix",
        ],
        SageOwnerType::Vector => &["vector", "zero_vector"],
        SageOwnerType::Field => &["GF", "FiniteField"],
        SageOwnerType::Graph => &[
            "Graph",
            "DiGraph",
            "PetersenGraph",
            "CompleteGraph",
            "CycleGraph",
        ],
        SageOwnerType::EllipticCurve => &[
            "EllipticCurve",
            "EllipticCurve_from_j",
            "EllipticCurve_from_c4c6",
        ],
        SageOwnerType::NumberField => &["NumberField", "CyclotomicField", "QuadraticField"],
        SageOwnerType::Polyhedron => &["Polyhedron"],
        SageOwnerType::PolynomialRing
        | SageOwnerType::UnivariatePolynomialRing
        | SageOwnerType::MultivariatePolynomialRing => &[
            "PolynomialRing",
            "LaurentPolynomialRing",
            "PowerSeriesRing",
            "BooleanPolynomialRing",
        ],
        SageOwnerType::FreeModule
        | SageOwnerType::PolynomialElement
        | SageOwnerType::Ideal
        | SageOwnerType::FieldElement
        | SageOwnerType::NumberFieldElement => &[],
    }
}

pub(crate) fn type_symbol_for_owner_type(owner_type: SageOwnerType) -> Option<&'static str> {
    match owner_type {
        SageOwnerType::Graph => Some("Graph"),
        SageOwnerType::EllipticCurve => Some("EllipticCurve"),
        SageOwnerType::NumberField => Some("NumberField"),
        SageOwnerType::Polyhedron => Some("Polyhedron"),
        SageOwnerType::PolynomialRing
        | SageOwnerType::UnivariatePolynomialRing
        | SageOwnerType::MultivariatePolynomialRing => Some("PolynomialRing"),
        SageOwnerType::Field => Some("GF"),
        SageOwnerType::MatrixConstructor => Some("matrix"),
        SageOwnerType::Matrix => Some("matrix"),
        SageOwnerType::Vector => Some("vector"),
        SageOwnerType::FreeModule
        | SageOwnerType::PolynomialElement
        | SageOwnerType::Ideal
        | SageOwnerType::FieldElement
        | SageOwnerType::NumberFieldElement => None,
    }
}

pub(crate) fn sage_method_return_type(
    receiver_type: SageOwnerType,
    member: &str,
) -> Option<SageOwnerType> {
    match (receiver_type, member) {
        (
            SageOwnerType::PolynomialRing
            | SageOwnerType::UnivariatePolynomialRing
            | SageOwnerType::MultivariatePolynomialRing,
            "ideal",
        ) => Some(SageOwnerType::Ideal),
        (
            SageOwnerType::Matrix,
            "adjugate"
            | "matrix_from_columns"
            | "matrix_from_rows"
            | "matrix_from_rows_and_columns"
            | "transpose",
        ) => Some(SageOwnerType::Matrix),
        (SageOwnerType::FreeModule, "basis_matrix") => Some(SageOwnerType::Matrix),
        (
            SageOwnerType::Matrix
            | SageOwnerType::Vector
            | SageOwnerType::FreeModule
            | SageOwnerType::PolynomialRing
            | SageOwnerType::UnivariatePolynomialRing
            | SageOwnerType::MultivariatePolynomialRing
            | SageOwnerType::PolynomialElement,
            "change_ring",
        ) => Some(receiver_type),
        (SageOwnerType::Matrix, "right_kernel" | "column_space" | "kernel") => {
            Some(SageOwnerType::FreeModule)
        }
        (SageOwnerType::Graph, "adjacency_matrix") => Some(SageOwnerType::Matrix),
        (SageOwnerType::Matrix, "charpoly") => Some(SageOwnerType::PolynomialElement),
        (SageOwnerType::PolynomialElement, "gcd" | "resultant" | "derivative") => {
            Some(SageOwnerType::PolynomialElement)
        }
        // A generic PolynomialRing does not distinguish Sage's univariate and
        // multivariate element hierarchies. Explicit multi-generator
        // preparser assignments carry the stronger ring type.
        (SageOwnerType::MultivariatePolynomialRing, "gen") => {
            Some(SageOwnerType::PolynomialElement)
        }
        (SageOwnerType::Field, "gen") => Some(SageOwnerType::FieldElement),
        (SageOwnerType::NumberField, "gen") => Some(SageOwnerType::NumberFieldElement),
        (SageOwnerType::PolynomialElement, "parent") => Some(SageOwnerType::PolynomialRing),
        (SageOwnerType::FieldElement, "polynomial") => Some(SageOwnerType::PolynomialElement),
        (SageOwnerType::Polyhedron, "intersection" | "polar") => Some(SageOwnerType::Polyhedron),
        _ => None,
    }
}

pub(super) fn infer_owner_type_from_name(name: &str) -> Option<SageOwnerType> {
    let lower = name.to_ascii_lowercase();
    if matches!(name, "R" | "S") || lower.ends_with("ring") {
        return Some(SageOwnerType::PolynomialRing);
    }
    if matches!(name, "I" | "ideal") || lower.ends_with("ideal") {
        return Some(SageOwnerType::Ideal);
    }
    if matches!(name, "F" | "K") || lower == "field" || lower.ends_with("_field") {
        return Some(SageOwnerType::Field);
    }
    if matches!(name, "E") || lower == "curve" || lower.ends_with("_curve") {
        return Some(SageOwnerType::EllipticCurve);
    }
    if lower == "graph" || lower.ends_with("_graph") || lower == "digraph" {
        return Some(SageOwnerType::Graph);
    }
    if lower == "number_field" || lower.ends_with("_number_field") {
        return Some(SageOwnerType::NumberField);
    }
    if matches!(lower.as_str(), "polyhedron" | "polytope")
        || lower.ends_with("_polyhedron")
        || lower.ends_with("_polytope")
    {
        return Some(SageOwnerType::Polyhedron);
    }
    if lower.starts_with("vec") || lower.ends_with("vec") || lower.ends_with("vector") {
        return Some(SageOwnerType::Vector);
    }
    if matches!(name, "jac") || lower.contains("mat") || lower.ends_with("matrix") {
        return Some(SageOwnerType::Matrix);
    }
    if matches!(name, "cp" | "f1" | "f2" | "fac" | "factor" | "pivot")
        || lower.contains("poly")
        || lower.ends_with("_factor")
        || lower.ends_with("_factors")
        || lower.ends_with("_polynomial")
    {
        return Some(SageOwnerType::PolynomialElement);
    }
    if matches!(name, "eq" | "q" | "g") || lower.ends_with("_poly") {
        return Some(SageOwnerType::PolynomialElement);
    }
    None
}

pub(super) fn infer_owner_type_from_name_for_member(
    name: &str,
    member: &str,
) -> Option<SageOwnerType> {
    if name == "matrix" {
        return Some(SageOwnerType::MatrixConstructor);
    }
    if (matches!(name, "G") || name.eq_ignore_ascii_case("graph")) && is_graph_member(member) {
        return Some(SageOwnerType::Graph);
    }
    if (matches!(name, "E") || name.eq_ignore_ascii_case("curve"))
        && is_elliptic_curve_member(member)
    {
        return Some(SageOwnerType::EllipticCurve);
    }
    if (matches!(name, "K") || name.eq_ignore_ascii_case("number_field"))
        && is_number_field_member(member)
    {
        return Some(SageOwnerType::NumberField);
    }
    if name == "f" && is_polynomial_element_member(member) {
        return Some(SageOwnerType::PolynomialElement);
    }
    if matches!(
        name,
        "A" | "G" | "P" | "Q" | "Q0" | "Q0inv" | "Qa" | "S1" | "T" | "base" | "base_inv"
    ) && is_matrix_context_member(member)
    {
        return Some(SageOwnerType::Matrix);
    }
    if matches!(name, "symbolic_obj" | "numeric_obj") && is_matrix_context_member(member) {
        return Some(SageOwnerType::Matrix);
    }
    if matches!(name, "u" | "v" | "target_u" | "u_candidate" | "vec") && is_vector_member(member) {
        return Some(SageOwnerType::Vector);
    }
    if matches!(name, "element" | "entry" | "root" | "value" | "x" | "y")
        && is_field_element_member(member)
    {
        return Some(SageOwnerType::FieldElement);
    }
    if matches!(name, "expr" | "equation" | "entry" | "poly" | "polynomial")
        && is_polynomial_element_member(member)
    {
        return Some(SageOwnerType::PolynomialElement);
    }
    infer_owner_type_from_name(name)
}
