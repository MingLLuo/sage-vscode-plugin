//! Conservative return-type inference for source-local functions.

use super::*;

pub(super) fn infer_local_function_return_types(
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
        if let Some(assignment) = parse_preparser_assignment(body_trimmed) {
            let assigned_names: BTreeSet<_> = std::iter::once(assignment.parent)
                .chain(assignment.generators.iter().copied())
                .collect();
            known_types.retain(|known_name, _| {
                assigned_names.contains(known_name.as_str())
                    || !line_rebinds_name(body_trimmed, known_name)
            });
            let parent_type = infer_preparser_parent_type(
                assignment.rhs,
                &known_types,
                local_function_returns,
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
        } else if let Some((assigned, rhs)) = parse_simple_assignment(body_trimmed) {
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
