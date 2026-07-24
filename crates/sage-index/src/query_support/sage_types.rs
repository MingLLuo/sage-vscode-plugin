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

fn is_field_member(member: &str) -> bool {
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

fn is_matrix_context_member(member: &str) -> bool {
    is_matrix_member(member) || matches!(member, "base_ring" | "list")
}

pub(crate) fn is_sage_namespace_owner(owner: &str) -> bool {
    owner
        .trim()
        .rsplit('.')
        .next()
        .is_some_and(|name| SAGE_STATIC_NAV_NAMESPACES.contains(&name))
}

pub(crate) fn infer_owner_type_before(
    source: &str,
    owner: &str,
    member: &str,
    max_line: u32,
) -> Option<SageOwnerType> {
    infer_owner_type_before_with_hints(source, owner, member, max_line, true)
}

pub(crate) fn infer_owner_type_before_strict(
    source: &str,
    owner: &str,
    member: &str,
    max_line: u32,
) -> Option<SageOwnerType> {
    infer_owner_type_before_with_hints(source, owner, member, max_line, false)
}

fn infer_owner_type_before_with_hints(
    source: &str,
    owner: &str,
    member: &str,
    max_line: u32,
    allow_name_hints: bool,
) -> Option<SageOwnerType> {
    let scope_map = LexicalScopeMap::new(source);
    let local_function_returns =
        infer_local_function_return_types(source, allow_name_hints, max_line, &scope_map);
    let mut known_types: HashMap<String, SageOwnerType> = HashMap::new();
    let target_functions = scope_map.enclosing_function_lines(max_line);
    let mut entered_functions = BTreeSet::new();
    let owner_base = owner_base_identifier(owner);
    if owner_base.is_some_and(|name| expression_locally_binds_name_on_line(source, name, max_line))
    {
        return None;
    }
    for (line_index, line) in source.lines().enumerate() {
        if line_index as u32 > max_line {
            break;
        }
        if !scope_map.is_code_line(line_index as u32) {
            continue;
        }
        let trimmed = line.trim_start();
        match scope_map.line_relation_to(line_index as u32, max_line) {
            InferenceLineRelation::Hidden => continue,
            InferenceLineRelation::Conditional => {
                known_types.retain(|name, _| !line_rebinds_name(trimmed, name));
                continue;
            }
            InferenceLineRelation::Dominates => {}
        }
        for function_line in scope_map.enclosing_function_lines(line_index as u32) {
            if target_functions.contains(&function_line) && entered_functions.insert(function_line)
            {
                known_types.retain(|name, _| {
                    !scope_map.function_statically_binds_name(source, function_line, name)
                });
            }
        }
        if let Some(parameters) =
            scope_map.enclosing_function_parameters_at_line(line_index as u32, max_line)
        {
            known_types.retain(|name, _| !parameters.contains(name));
        }
        let Some((name, rhs)) = parse_simple_assignment(trimmed) else {
            known_types.retain(|name, _| !line_rebinds_name(trimmed, name));
            continue;
        };
        known_types
            .retain(|known_name, _| known_name == name || !line_rebinds_name(trimmed, known_name));
        let inferred_type =
            infer_type_from_rhs(rhs, &known_types, &local_function_returns, allow_name_hints)
                .or_else(|| {
                    allow_name_hints
                        .then(|| infer_owner_type_from_name(name))
                        .flatten()
                });
        // The right-hand side sees the old binding, but every assignment then
        // creates a new one. Unknown values must clear stale constructor types.
        known_types.remove(name);
        if let Some(owner_type) = inferred_type {
            known_types.insert(name.to_string(), owner_type);
        }
    }
    let exact_owner_type = known_types.get(owner).copied();
    let expression_owner_type = if allow_name_hints {
        infer_owner_type_from_owner_expression(owner, member)
    } else {
        infer_owner_type_from_explicit_expression(owner, member)
    };
    let base_owner_type = owner_base.and_then(|name| known_types.get(name).copied());
    exact_owner_type
        .or_else(|| {
            owner_is_compound(owner)
                .then_some(expression_owner_type)
                .flatten()
        })
        .or(base_owner_type)
        .or(expression_owner_type)
        .or_else(|| {
            allow_name_hints
                .then(|| {
                    owner_base.and_then(|name| infer_owner_type_from_name_for_member(name, member))
                })
                .flatten()
        })
        .or_else(|| {
            allow_name_hints
                .then(|| infer_owner_type_from_name_for_member(owner, member))
                .flatten()
        })
}

pub(super) fn owner_base_identifier(owner: &str) -> Option<&str> {
    let owner = owner.trim_start();
    let bytes = owner.as_bytes();
    let mut end = 0usize;
    while end < bytes.len() && is_word_byte(bytes[end]) {
        end += 1;
    }
    (end > 0).then_some(&owner[..end])
}

fn owner_is_compound(owner: &str) -> bool {
    owner.contains('.') || owner.contains('(') || owner.contains('[')
}

pub(super) fn infer_owner_type_from_owner_expression(
    owner: &str,
    member: &str,
) -> Option<SageOwnerType> {
    let compact: String = owner.chars().filter(|ch| !ch.is_whitespace()).collect();
    if compact.contains("[\"R\"]")
        || compact.contains("['R']")
        || compact.contains("[\"ring\"]")
        || compact.contains("['ring']")
    {
        return Some(SageOwnerType::PolynomialRing);
    }
    if compact.contains("[\"Q\"]")
        || compact.contains("['Q']")
        || compact.contains("[\"g\"]")
        || compact.contains("['g']")
    {
        return Some(SageOwnerType::PolynomialElement);
    }
    if compact.contains("[\"A\"]")
        || compact.contains("['A']")
        || compact.contains("[\"W\"]")
        || compact.contains("['W']")
    {
        return Some(SageOwnerType::Matrix);
    }
    if compact.contains("[\"b\"]") || compact.contains("['b']") {
        return Some(SageOwnerType::Vector);
    }
    if compact.contains("Graph(")
        || compact.contains("DiGraph(")
        || compact.contains("PetersenGraph(")
        || compact.contains("CompleteGraph(")
        || compact.contains("CycleGraph(")
    {
        return Some(SageOwnerType::Graph);
    }
    if compact.contains("EllipticCurve(") {
        return Some(SageOwnerType::EllipticCurve);
    }
    if compact.contains("NumberField(")
        || compact.contains("CyclotomicField(")
        || compact.contains("QuadraticField(")
    {
        return Some(SageOwnerType::NumberField);
    }
    if compact.contains('[') {
        if let Some(base) = owner_base_identifier(owner) {
            let lower = base.to_ascii_lowercase();
            if matches!(
                lower.as_str(),
                "qs" | "qhs" | "mats" | "matrices" | "gs" | "gtildes" | "ordinary_target"
            ) || lower.ends_with("_mats")
                || lower.ends_with("_matrices")
                || lower.ends_with("_qs")
            {
                return Some(SageOwnerType::Matrix);
            }
            if matches!(lower.as_str(), "eqs" | "polys" | "vars" | "xs" | "z_polys")
                || lower.ends_with("_eqs")
                || lower.ends_with("_polys")
            {
                return Some(SageOwnerType::PolynomialElement);
            }
            if matches!(lower.as_str(), "kernel" | "rows") || lower.ends_with("_rows") {
                return Some(SageOwnerType::Vector);
            }
        }
    }
    if compact.contains(".ideal(") {
        return Some(SageOwnerType::Ideal);
    }
    if compact.contains(".base_ring(") {
        return Some(SageOwnerType::Field);
    }
    if compact.contains(".polynomial(") {
        return Some(SageOwnerType::PolynomialElement);
    }
    if compact.contains('*') && is_matrix_context_member(member) {
        return Some(SageOwnerType::Matrix);
    }
    if compact.contains(".parent(") {
        if is_field_member(member) {
            return Some(SageOwnerType::Field);
        }
        if matches!(
            member,
            "gen" | "gens" | "hom" | "ideal" | "lagrange_polynomial"
        ) {
            return Some(SageOwnerType::PolynomialRing);
        }
    }
    if compact.contains(".charpoly(") {
        return Some(SageOwnerType::PolynomialElement);
    }
    if compact.contains(".gen(") || compact.contains(".gens(") {
        return Some(SageOwnerType::PolynomialElement);
    }
    if compact.contains(".transpose(")
        || compact.contains(".solve_right(")
        || compact.contains(".matrix_from_rows(")
        || compact.contains(".matrix_from_columns(")
        || compact.contains(".matrix_from_rows_and_columns(")
        || compact.contains(".adjugate(")
    {
        return Some(SageOwnerType::Matrix);
    }
    if compact.contains(".right_kernel(")
        || compact.contains(".column_space(")
        || compact.contains(".kernel(")
    {
        return Some(SageOwnerType::FreeModule);
    }
    if compact.contains(".basis_matrix(") {
        return Some(SageOwnerType::Matrix);
    }
    if compact.contains(".adjacency_matrix(") {
        return Some(SageOwnerType::Matrix);
    }
    None
}

fn infer_owner_type_from_explicit_expression(owner: &str, member: &str) -> Option<SageOwnerType> {
    let compact: String = owner.chars().filter(|ch| !ch.is_whitespace()).collect();
    let has_explicit_type_evidence = [
        "Graph(",
        "DiGraph(",
        "PetersenGraph(",
        "CompleteGraph(",
        "CycleGraph(",
        "EllipticCurve(",
        "NumberField(",
        "CyclotomicField(",
        "QuadraticField(",
    ]
    .iter()
    .any(|evidence| compact.contains(evidence));
    has_explicit_type_evidence
        .then(|| infer_owner_type_from_owner_expression(owner, member))
        .flatten()
}

fn infer_local_function_return_types(
    source: &str,
    allow_name_hints: bool,
    max_line: u32,
    scope_map: &LexicalScopeMap,
) -> HashMap<String, SageOwnerType> {
    let mut returns: HashMap<String, SageOwnerType> = HashMap::new();
    let target_functions = scope_map.enclosing_function_lines(max_line);
    let mut entered_functions = BTreeSet::new();
    let lines: Vec<&str> = source.lines().collect();
    for (line_index, line) in lines.iter().enumerate().take(max_line as usize + 1) {
        if !scope_map.is_code_line(line_index as u32) {
            continue;
        }
        let trimmed = line.trim_start();
        match scope_map.line_relation_to(line_index as u32, max_line) {
            InferenceLineRelation::Hidden => continue,
            InferenceLineRelation::Conditional => {
                returns.retain(|name, _| !line_rebinds_name(trimmed, name));
                continue;
            }
            InferenceLineRelation::Dominates => {}
        }
        for function_line in scope_map.enclosing_function_lines(line_index as u32) {
            if target_functions.contains(&function_line) && entered_functions.insert(function_line)
            {
                returns.retain(|name, _| {
                    !scope_map.function_statically_binds_name(source, function_line, name)
                });
            }
        }
        if let Some(parameters) =
            scope_map.enclosing_function_parameters_at_line(line_index as u32, max_line)
        {
            returns.retain(|name, _| !parameters.contains(name));
        }
        let Some(captures) = function_header_re().captures(trimmed) else {
            returns.retain(|name, _| !line_rebinds_name(trimmed, name));
            continue;
        };
        let Some(name) = captures.name("name").map(|name| name.as_str()) else {
            continue;
        };
        let inferred = infer_local_function_return_type(
            &lines,
            line_index as u32,
            &returns,
            allow_name_hints,
            scope_map,
        );
        returns.remove(name);
        if let Some(owner_type) = inferred {
            returns.insert(name.to_string(), owner_type);
        }
    }
    returns
}

fn infer_local_function_return_type(
    lines: &[&str],
    function_line: u32,
    local_function_returns: &HashMap<String, SageOwnerType>,
    allow_name_hints: bool,
    scope_map: &LexicalScopeMap,
) -> Option<SageOwnerType> {
    let mut known_types: HashMap<String, SageOwnerType> = HashMap::new();
    let mut return_type = None;
    let mut saw_return = false;
    let mut saw_unconditional_return = false;
    let mut entered_function_scope = false;
    for (line_index, body_line) in lines.iter().enumerate().skip(function_line as usize + 1) {
        let line_index = line_index as u32;
        if scope_map.is_within_function_scope(line_index, function_line) {
            entered_function_scope = true;
        } else if entered_function_scope {
            break;
        } else {
            continue;
        }
        if !scope_map.is_code_line(line_index) {
            continue;
        }
        let body_trimmed = body_line.trim_start();
        let return_expression = if body_trimmed == "return" {
            Some("")
        } else {
            body_trimmed.strip_prefix("return ")
        };
        if !scope_map.is_unconditional_function_body_line(line_index, function_line) {
            if scope_map.is_direct_function_body_line(line_index, function_line) {
                if let Some(return_expression) = return_expression {
                    saw_return = true;
                    let owner_type = infer_type_from_rhs(
                        return_expression,
                        &known_types,
                        local_function_returns,
                        allow_name_hints,
                    )?;
                    if return_type.is_some_and(|known| known != owner_type) {
                        return None;
                    }
                    return_type = Some(owner_type);
                }
                known_types.retain(|name, _| !line_rebinds_name(body_trimmed, name));
            }
            continue;
        }
        if let Some((assigned, rhs)) = parse_simple_assignment(body_trimmed) {
            known_types.retain(|known_name, _| {
                known_name == assigned || !line_rebinds_name(body_trimmed, known_name)
            });
            let inferred =
                infer_type_from_rhs(rhs, &known_types, local_function_returns, allow_name_hints)
                    .or_else(|| {
                        allow_name_hints
                            .then(|| infer_owner_type_from_name(assigned))
                            .flatten()
                    });
            known_types.remove(assigned);
            if let Some(owner_type) = inferred {
                known_types.insert(assigned.to_string(), owner_type);
            }
        } else {
            known_types.retain(|name, _| !line_rebinds_name(body_trimmed, name));
        }

        let Some(return_expression) = return_expression else {
            continue;
        };
        saw_return = true;
        saw_unconditional_return = true;
        let owner_type = infer_type_from_rhs(
            return_expression,
            &known_types,
            local_function_returns,
            allow_name_hints,
        )?;
        if return_type.is_some_and(|known| known != owner_type) {
            return None;
        }
        return_type = Some(owner_type);
    }
    (saw_return && saw_unconditional_return)
        .then_some(return_type)
        .flatten()
}

fn parse_simple_assignment(line: &str) -> Option<(&str, &str)> {
    let captures = simple_assignment_re().captures(line)?;
    let name = captures.name("name")?.as_str();
    let rhs = captures.name("rhs")?.as_str();
    Some((name, rhs))
}

fn infer_type_from_rhs(
    rhs: &str,
    known_types: &HashMap<String, SageOwnerType>,
    local_function_returns: &HashMap<String, SageOwnerType>,
    allow_name_hints: bool,
) -> Option<SageOwnerType> {
    let value = rhs.trim();
    if value.is_empty() {
        return None;
    }
    if allow_name_hints && value.contains(".ideal(") {
        return Some(SageOwnerType::Ideal);
    }
    if allow_name_hints && (value.contains("[\"R\"]") || value.contains("['R']")) {
        return Some(SageOwnerType::PolynomialRing);
    }
    if allow_name_hints && (value.contains("[\"Q\"]") || value.contains("['Q']")) {
        return Some(SageOwnerType::PolynomialElement);
    }
    if let Some(product_type) = infer_product_type_from_rhs(value, known_types, allow_name_hints) {
        return Some(product_type);
    }
    let callee = assignment_call_re()
        .captures(value)
        .and_then(|captures| captures.name("callee"))
        .map(|callee| callee.as_str());
    if let Some(callee) = callee {
        let short = callee.rsplit('.').next().unwrap_or(callee);
        if !callee.contains('.') {
            if let Some(owner_type) = local_function_returns.get(short).copied() {
                return Some(owner_type);
            }
        }
        if let Some(owner_type) = sage_constructor_return_type(short) {
            return Some(owner_type);
        }
        if known_types
            .get(short)
            .is_some_and(|owner_type| *owner_type == SageOwnerType::PolynomialRing)
        {
            return Some(SageOwnerType::PolynomialElement);
        }
        if let Some(owner_type) = known_types.get(callee).copied() {
            return Some(owner_type);
        }
        if let Some((receiver, member)) = callee.rsplit_once('.') {
            if allow_name_hints || known_types.contains_key(receiver) {
                if let Some(owner_type) = sage_method_return_type(member) {
                    return Some(owner_type);
                }
            }
        }
    }
    if identifier_re().is_match(value) {
        return known_types.get(value).copied();
    }
    None
}

fn infer_product_type_from_rhs(
    value: &str,
    known_types: &HashMap<String, SageOwnerType>,
    allow_name_hints: bool,
) -> Option<SageOwnerType> {
    if !value.contains('*') {
        return None;
    }
    let mut saw_matrix = false;
    let mut saw_vector = false;
    for captures in word_re().captures_iter(value) {
        let Some(name) = captures.name("name").map(|name| name.as_str()) else {
            continue;
        };
        let owner_type = known_types.get(name).copied().or_else(|| {
            allow_name_hints
                .then(|| infer_owner_type_from_name(name))
                .flatten()
        });
        match owner_type {
            Some(SageOwnerType::Matrix) => saw_matrix = true,
            Some(SageOwnerType::Vector) => saw_vector = true,
            _ => {}
        }
    }
    match (saw_matrix, saw_vector) {
        (true, true) => Some(SageOwnerType::Vector),
        (true, false) => Some(SageOwnerType::Matrix),
        (false, true) => Some(SageOwnerType::Vector),
        (false, false) => None,
    }
}

fn sage_constructor_return_type(name: &str) -> Option<SageOwnerType> {
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
        SageOwnerType::PolynomialRing => &[
            "PolynomialRing",
            "LaurentPolynomialRing",
            "PowerSeriesRing",
            "BooleanPolynomialRing",
        ],
        SageOwnerType::FreeModule
        | SageOwnerType::PolynomialElement
        | SageOwnerType::Ideal
        | SageOwnerType::FieldElement => &[],
    }
}

pub(crate) fn type_symbol_for_owner_type(owner_type: SageOwnerType) -> Option<&'static str> {
    match owner_type {
        SageOwnerType::Graph => Some("Graph"),
        SageOwnerType::EllipticCurve => Some("EllipticCurve"),
        SageOwnerType::NumberField => Some("NumberField"),
        SageOwnerType::PolynomialRing => Some("PolynomialRing"),
        SageOwnerType::Field => Some("GF"),
        SageOwnerType::MatrixConstructor => Some("matrix"),
        SageOwnerType::Matrix => Some("matrix"),
        SageOwnerType::Vector => Some("vector"),
        SageOwnerType::FreeModule
        | SageOwnerType::PolynomialElement
        | SageOwnerType::Ideal
        | SageOwnerType::FieldElement => None,
    }
}

pub(crate) fn sage_method_return_type(member: &str) -> Option<SageOwnerType> {
    match member {
        "ideal" => Some(SageOwnerType::Ideal),
        "adjugate"
        | "basis_matrix"
        | "change_ring"
        | "matrix_from_columns"
        | "matrix_from_rows"
        | "matrix_from_rows_and_columns"
        | "transpose" => Some(SageOwnerType::Matrix),
        "right_kernel" | "column_space" | "kernel" => Some(SageOwnerType::FreeModule),
        "adjacency_matrix" => Some(SageOwnerType::Matrix),
        "charpoly" | "gen" | "gens" | "gcd" | "resultant" | "derivative" => {
            Some(SageOwnerType::PolynomialElement)
        }
        "base_ring" => Some(SageOwnerType::Field),
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

fn infer_owner_type_from_name_for_member(name: &str, member: &str) -> Option<SageOwnerType> {
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

pub(crate) fn assignment_detail(name: &str, annotation: Option<&str>, rhs: &str) -> String {
    if let Some(annotation) = annotation.map(str::trim).filter(|value| !value.is_empty()) {
        return format!("Variable {name}: {annotation}");
    }

    if let Some(inferred) = infer_assignment_value_kind(rhs) {
        return format!("Variable {name}: {inferred}");
    }

    format!("Variable {name}")
}

fn infer_assignment_value_kind(rhs: &str) -> Option<String> {
    let value = rhs.trim();
    if value.is_empty() {
        return None;
    }

    if value.starts_with('"') || value.starts_with('\'') {
        return Some("str".to_string());
    }
    if value.starts_with('[') {
        return Some("list".to_string());
    }
    if value.starts_with('{') {
        return Some("dict/set".to_string());
    }
    if value.starts_with('(') {
        return Some("tuple/group".to_string());
    }
    if value == "True" || value == "False" {
        return Some("bool".to_string());
    }
    if value.parse::<i64>().is_ok() {
        return Some("Integer".to_string());
    }
    if value.parse::<f64>().is_ok() {
        return Some("RealNumber".to_string());
    }
    if let Some(callee) = assignment_call_re()
        .captures(value)
        .and_then(|captures| captures.name("callee"))
        .map(|callee| callee.as_str())
    {
        if callee
            .chars()
            .next()
            .is_some_and(|first| first.is_ascii_uppercase())
            || SAGE_TYPES.contains(&callee)
        {
            return Some(callee.to_string());
        }
        return Some(format!("result of {callee}(...)"));
    }
    if identifier_re().is_match(value) {
        return Some(format!("value of {value}"));
    }
    None
}
