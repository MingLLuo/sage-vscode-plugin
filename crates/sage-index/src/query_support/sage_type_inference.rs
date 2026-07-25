//! Scope-aware owner-type inference for Sage member navigation.

use super::local_function_returns::infer_local_function_return_types;
use super::*;

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
    let lines: Vec<&str> = source.lines().collect();
    let mut preparser_continuation_end = None;
    for (line_index, line) in lines.iter().enumerate() {
        if line_index as u32 > max_line {
            break;
        }
        if preparser_continuation_end.is_some_and(|end_line| line_index as u32 <= end_line) {
            continue;
        }
        if !scope_map.is_code_line(line_index as u32) {
            continue;
        }
        let trimmed = line.trim_start();
        let preparser_statement =
            preparser_assignment_statement(&lines, line_index as u32, max_line);
        if let Some(statement) = &preparser_statement {
            preparser_continuation_end = Some(statement.end_line);
        }
        let preparser_walrus_bindings = preparser_statement
            .as_ref()
            .and_then(|statement| parse_preparser_assignment(&statement.text))
            .map(|assignment| walrus_binding_names(assignment.rhs))
            .unwrap_or_default();
        match scope_map.line_relation_to(line_index as u32, max_line) {
            InferenceLineRelation::Hidden => continue,
            InferenceLineRelation::Conditional => {
                known_types.retain(|name, _| {
                    !line_rebinds_name(trimmed, name) && !preparser_walrus_bindings.contains(name)
                });
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
        if let Some(statement) = preparser_statement {
            let assignment_text = if statement.complete {
                statement.text.as_str()
            } else {
                trimmed
            };
            let Some(assignment) = parse_preparser_assignment(assignment_text) else {
                continue;
            };
            let assigned_names: BTreeSet<_> = std::iter::once(assignment.parent)
                .chain(assignment.generators.iter().copied())
                .collect();
            known_types.retain(|known_name, _| {
                !preparser_walrus_bindings.contains(known_name)
                    && (assigned_names.contains(known_name.as_str())
                        || !line_rebinds_name(trimmed, known_name))
            });
            if !statement.complete {
                for name in &assigned_names {
                    known_types.remove(*name);
                }
                continue;
            }
            let parent_type = infer_preparser_parent_type(
                assignment.rhs,
                &known_types,
                &local_function_returns,
                allow_name_hints,
            );
            for name in &assigned_names {
                known_types.remove(*name);
            }
            if let Some(parent_type) = parent_type {
                let parent_type = specialize_preparser_parent_type(
                    parent_type,
                    assignment.generators.len(),
                    assignment.rhs,
                );
                known_types.insert(assignment.parent.to_string(), parent_type);
                if let Some(generator_type) =
                    preparser_generator_type(parent_type, assignment.generators.len())
                {
                    for generator in assignment.generators {
                        known_types.insert(generator.to_string(), generator_type);
                    }
                }
            }
            continue;
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

fn owner_is_compound(owner: &str) -> bool {
    owner.contains('.') || owner.contains('(') || owner.contains('[')
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
    if compact.contains("Polyhedron(") {
        return Some(SageOwnerType::Polyhedron);
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
        "Polyhedron(",
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
