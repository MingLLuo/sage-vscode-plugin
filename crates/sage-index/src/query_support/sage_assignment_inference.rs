//! Type and hover-detail inference for assignment right-hand sides.

use super::*;

pub(super) fn parse_simple_assignment(line: &str) -> Option<(&str, &str)> {
    let captures = simple_assignment_re().captures(line)?;
    let name = captures.name("name")?.as_str();
    let rhs = captures.name("rhs")?.as_str();
    Some((name, rhs))
}

pub(super) struct PreparserAssignment<'a> {
    pub(super) parent: &'a str,
    pub(super) generators: Vec<&'a str>,
    pub(super) rhs: &'a str,
}

pub(super) fn parse_preparser_assignment(line: &str) -> Option<PreparserAssignment<'_>> {
    let captures = preparser_assignment_re().captures(line)?;
    Some(PreparserAssignment {
        parent: captures.name("parent")?.as_str(),
        generators: captures
            .name("symbols")?
            .as_str()
            .split(',')
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .collect(),
        rhs: captures.name("rhs")?.as_str(),
    })
}

pub(super) fn infer_preparser_parent_type(
    rhs: &str,
    known_types: &HashMap<String, SageOwnerType>,
    local_function_returns: &HashMap<String, SageOwnerType>,
    allow_name_hints: bool,
) -> Option<SageOwnerType> {
    infer_type_from_rhs(rhs, known_types, local_function_returns, allow_name_hints).or_else(|| {
        let compact: String = rhs.chars().filter(|ch| !ch.is_whitespace()).collect();
        compact
            .ends_with("[]")
            .then_some(SageOwnerType::PolynomialRing)
    })
}

pub(super) fn preparser_generator_type(
    parent_type: SageOwnerType,
    generator_count: usize,
) -> Option<SageOwnerType> {
    match (parent_type, generator_count) {
        // Multiple named generators are direct evidence for Sage's
        // multivariate polynomial representation. A single generator is not:
        // it may come from a univariate ring whose method implementations live
        // in a different hierarchy, so keep it unresolved rather than
        // producing a wrong high-confidence jump.
        (SageOwnerType::MultivariatePolynomialRing, count) if count > 1 => {
            Some(SageOwnerType::PolynomialElement)
        }
        (SageOwnerType::Field, 1) => Some(SageOwnerType::FieldElement),
        (SageOwnerType::NumberField, 1) => Some(SageOwnerType::NumberFieldElement),
        _ => None,
    }
}

pub(super) fn specialize_preparser_parent_type(
    parent_type: SageOwnerType,
    generator_count: usize,
    rhs: &str,
) -> SageOwnerType {
    let compact: String = rhs.chars().filter(|ch| !ch.is_whitespace()).collect();
    let is_plain_polynomial_ring = assignment_call_re()
        .captures(rhs.trim())
        .and_then(|captures| captures.name("callee"))
        .is_some_and(|callee| {
            callee
                .as_str()
                .rsplit('.')
                .next()
                .is_some_and(|name| name == "PolynomialRing")
        });
    let is_bracket_polynomial_ring = compact.ends_with("[]");
    if !is_plain_polynomial_ring && !is_bracket_polynomial_ring {
        // Laurent, power-series, Boolean, and other ring factories have
        // separate method implementations. Keep their shared catalog owner
        // conservative until the constructor family is modeled explicitly.
        return parent_type;
    }
    match (parent_type, generator_count) {
        (SageOwnerType::PolynomialRing, 1) => SageOwnerType::UnivariatePolynomialRing,
        (SageOwnerType::PolynomialRing, count) if count > 1 => {
            SageOwnerType::MultivariatePolynomialRing
        }
        _ => parent_type,
    }
}

pub(super) fn infer_type_from_rhs(
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
        if short == "PolynomialRing" {
            if let Some(owner_type) = polynomial_ring_constructor_type(value) {
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
            let receiver_type = known_types.get(receiver).copied().or_else(|| {
                allow_name_hints
                    .then(|| infer_owner_type_from_name_for_member(receiver, member))
                    .flatten()
            });
            if let Some(owner_type) =
                receiver_type.and_then(|owner_type| sage_method_return_type(owner_type, member))
            {
                return Some(owner_type);
            }
        }
    }
    if identifier_re().is_match(value) {
        return known_types.get(value).copied();
    }
    None
}

fn polynomial_ring_constructor_type(value: &str) -> Option<SageOwnerType> {
    let open = value.find('(')?;
    let close = value.rfind(')')?;
    if open >= close {
        return None;
    }
    let arguments = split_call_arguments(&value[open + 1..close]);
    let generator_argument = arguments.iter().skip(1).find_map(|argument| {
        let argument = argument.trim();
        if let Some((name, value)) = argument.split_once('=') {
            return matches!(name.trim(), "name" | "names").then_some(value.trim());
        }
        (!argument.contains('=')).then_some(argument)
    })?;
    let generator_count = polynomial_generator_count(generator_argument)?;
    match generator_count {
        1 => Some(SageOwnerType::UnivariatePolynomialRing),
        count if count > 1 => Some(SageOwnerType::MultivariatePolynomialRing),
        _ => None,
    }
}

fn polynomial_generator_count(argument: &str) -> Option<usize> {
    let argument = argument.trim();
    if let Ok(count) = argument.parse::<usize>() {
        return Some(count);
    }
    if argument.starts_with(['\'', '"']) {
        let quote = argument.as_bytes()[0] as char;
        let value = argument.strip_prefix(quote)?.strip_suffix(quote)?;
        return Some(
            value
                .split(',')
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .count(),
        );
    }
    let inner = argument
        .strip_prefix('(')
        .and_then(|value| value.strip_suffix(')'))
        .or_else(|| {
            argument
                .strip_prefix('[')
                .and_then(|value| value.strip_suffix(']'))
        })?;
    Some(split_call_arguments(inner).len())
}

fn split_call_arguments(arguments: &str) -> Vec<&str> {
    let mut result = Vec::new();
    let mut depth = 0usize;
    let mut quote = None;
    let mut escaped = false;
    let mut start = 0usize;
    for (index, ch) in arguments.char_indices() {
        if let Some(active_quote) = quote {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == active_quote {
                quote = None;
            }
            continue;
        }
        match ch {
            '\'' | '"' => quote = Some(ch),
            '(' | '[' | '{' => depth = depth.saturating_add(1),
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                let argument = arguments[start..index].trim();
                if !argument.is_empty() {
                    result.push(argument);
                }
                start = index + ch.len_utf8();
            }
            _ => {}
        }
    }
    let argument = arguments[start..].trim();
    if !argument.is_empty() {
        result.push(argument);
    }
    result
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
