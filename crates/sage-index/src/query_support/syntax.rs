use super::*;

pub(crate) fn word_at_source_position(
    text: &str,
    line: u32,
    character: u32,
) -> Option<(String, SourceRange)> {
    let source_line = text.lines().nth(line as usize)?;
    let mut character = character.min(source_line.len() as u32) as usize;
    if character == source_line.len() && character > 0 {
        character -= 1;
    }
    let bytes = source_line.as_bytes();
    if character >= bytes.len() {
        return None;
    }
    if !is_word_byte(bytes[character]) && character > 0 && is_word_byte(bytes[character - 1]) {
        character -= 1;
    }
    if !is_word_byte(bytes[character]) {
        return None;
    }
    let mut start = character;
    let mut end = character + 1;
    while start > 0 && is_word_byte(bytes[start - 1]) {
        start -= 1;
    }
    while end < bytes.len() && is_word_byte(bytes[end]) {
        end += 1;
    }
    Some((
        source_line[start..end].to_string(),
        SourceRange {
            start_line: line,
            start_character: start as u32,
            end_line: line,
            end_character: end as u32,
        },
    ))
}

pub(crate) fn range_for_first_symbol(source: &str, symbol: &str) -> Option<SourceRange> {
    if symbol.is_empty() {
        return None;
    }
    range_for_symbols(source, symbol).next()
}

pub(crate) fn range_for_first_code_symbol(
    source: &str,
    symbol: &str,
    code_map: &CodeMap,
) -> Option<SourceRange> {
    if symbol.is_empty() {
        return None;
    }
    range_for_symbols(source, symbol).find(|range| {
        code_map
            .offset(range.start_line, range.start_character)
            .is_some_and(|offset| code_map.is_code_offset(offset))
    })
}

fn range_for_symbols<'a>(
    source: &'a str,
    symbol: &'a str,
) -> impl Iterator<Item = SourceRange> + 'a {
    source
        .lines()
        .enumerate()
        .flat_map(move |(line_index, line)| {
            line.match_indices(symbol).filter_map(move |(start, _)| {
                let end = start + symbol.len();
                let starts_at_boundary = start == 0 || !is_word_byte(line.as_bytes()[start - 1]);
                let ends_at_boundary = end == line.len() || !is_word_byte(line.as_bytes()[end]);
                (starts_at_boundary && ends_at_boundary).then_some(SourceRange {
                    start_line: line_index as u32,
                    start_character: start as u32,
                    end_line: line_index as u32,
                    end_character: end as u32,
                })
            })
        })
}

pub(crate) fn dotted_symbol_at_range(source: &str, range: &SourceRange) -> Option<String> {
    let line = source.lines().nth(range.start_line as usize)?;
    let bytes = line.as_bytes();
    let mut start = range.start_character as usize;
    let mut end = range.end_character as usize;
    while start > 0 && is_word_byte(bytes[start - 1]) {
        start -= 1;
    }
    while end < bytes.len() && is_word_byte(bytes[end]) {
        end += 1;
    }
    if start > 0 && bytes[start - 1] == b'.' {
        let dot = start - 1;
        if let Some(owner_start) = python_primary_start(line, dot) {
            if owner_start < dot {
                let owner = line[owner_start..dot].trim();
                let member = line[start..end].trim();
                if !owner.is_empty() && !member.is_empty() {
                    return Some(format!("{owner}.{member}"));
                }
            }
        }
    }
    while start > 0 {
        let byte = bytes[start - 1];
        if is_word_byte(byte) || byte == b'.' {
            start -= 1;
        } else {
            break;
        }
    }
    while end < bytes.len() {
        let byte = bytes[end];
        if is_word_byte(byte) || byte == b'.' {
            end += 1;
        } else {
            break;
        }
    }
    let value = line[start..end].trim_matches('.');
    value.contains('.').then(|| value.to_string())
}

pub(super) fn python_primary_start(line: &str, end: usize) -> Option<usize> {
    let bytes = line.as_bytes();
    let mut pos = end.min(bytes.len());
    while pos > 0 && bytes[pos - 1].is_ascii_whitespace() {
        pos -= 1;
    }
    loop {
        if pos == 0 {
            break;
        }
        match bytes[pos - 1] {
            b']' => pos = matching_open_bracket(bytes, pos - 1, b'[', b']')?,
            b')' => pos = matching_open_bracket(bytes, pos - 1, b'(', b')')?,
            byte if is_word_byte(byte) => {
                while pos > 0 && is_word_byte(bytes[pos - 1]) {
                    pos -= 1;
                }
            }
            b'.' => pos -= 1,
            _ => break,
        }
    }
    (pos < end).then_some(pos)
}

fn matching_open_bracket(bytes: &[u8], close_index: usize, open: u8, close: u8) -> Option<usize> {
    let mut depth = 0usize;
    for index in (0..=close_index).rev() {
        match bytes[index] {
            byte if byte == close => depth = depth.saturating_add(1),
            byte if byte == open => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => {}
        }
    }
    None
}

pub(crate) fn dotted_owner_member(value: &str) -> Option<(&str, &str)> {
    let (owner, member) = value.rsplit_once('.')?;
    (!owner.is_empty() && !member.is_empty()).then_some((owner, member))
}

pub(crate) fn assignment_constructor_before_line(
    source: &str,
    variable: &str,
    max_line: u32,
) -> Option<String> {
    if expression_locally_binds_name_on_line(source, variable, max_line) {
        return None;
    }
    let scope_map = LexicalScopeMap::new(source);
    let mut constructor = None;
    let target_functions = scope_map.enclosing_function_lines(max_line);
    let mut entered_functions = BTreeSet::new();
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
        let preparser_assignment = preparser_statement
            .as_ref()
            .and_then(|statement| parse_preparser_assignment(&statement.text));
        let preparser_walrus_rebinds = preparser_assignment
            .as_ref()
            .is_some_and(|assignment| walrus_binding_names(assignment.rhs).contains(variable));
        match scope_map.line_relation_to(line_index as u32, max_line) {
            InferenceLineRelation::Hidden => continue,
            InferenceLineRelation::Conditional => {
                if line_rebinds_name(trimmed, variable) || preparser_walrus_rebinds {
                    constructor = None;
                }
                continue;
            }
            InferenceLineRelation::Dominates => {}
        }
        for function_line in scope_map.enclosing_function_lines(line_index as u32) {
            if target_functions.contains(&function_line)
                && entered_functions.insert(function_line)
                && scope_map.function_statically_binds_name(source, function_line, variable)
            {
                constructor = None;
            }
        }
        if scope_map
            .enclosing_function_parameters_at_line(line_index as u32, max_line)
            .is_some_and(|parameters| parameters.contains(variable))
        {
            constructor = None;
            continue;
        }
        if let Some(statement) = &preparser_statement {
            let lhs_rebinds = preparser_assignment.as_ref().is_some_and(|assignment| {
                assignment.parent == variable || assignment.generators.contains(&variable)
            });
            if preparser_walrus_rebinds || (!statement.complete && lhs_rebinds) {
                constructor = None;
            }
            if !statement.complete {
                continue;
            }
            let Some(assignment) = preparser_assignment else {
                continue;
            };
            if lhs_rebinds {
                constructor = if assignment.generators.contains(&variable) {
                    Some(format!("{}.gen", assignment.parent))
                } else {
                    assignment
                        .rhs
                        .trim()
                        .strip_suffix("[]")
                        .map(|_| "PolynomialRing".to_string())
                        .or_else(|| {
                            assignment_call_re()
                                .captures(assignment.rhs.trim())
                                .and_then(|captures| captures.name("callee"))
                                .map(|callee| callee.as_str().to_string())
                        })
                };
            }
            continue;
        }
        if !line_rebinds_name(trimmed, variable) {
            continue;
        }
        constructor = assignment_constructor_re()
            .captures(trimmed)
            .and_then(|captures| captures.name("ctor"))
            .map(|ctor| ctor.as_str().to_string());
    }
    constructor
}

pub(crate) fn is_word_byte(byte: u8) -> bool {
    byte == b'_' || byte.is_ascii_alphanumeric()
}

pub(crate) fn is_valid_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first == '_' || first.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}
