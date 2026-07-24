use super::*;

pub(crate) fn local_receiver_member_symbol_from_symbols(
    source: &str,
    symbols: &[SymbolRecord],
    owner: &str,
    name: &str,
    target_range: &SourceRange,
) -> Option<SymbolRecord> {
    if !matches!(owner.trim(), "self" | "cls") {
        return None;
    }
    let target_scope = definition_scope_at_line(source, target_range.start_line);
    let class_scope = target_scope
        .iter()
        .copied()
        .rev()
        .find(|scope| scope.kind == DefinitionScopeKind::Class)?;
    let source_map = CodeMap::new(source);
    if !local_receiver_binding_is_reliable(
        source,
        &source_map,
        &target_scope,
        class_scope,
        owner,
        target_range,
    ) {
        return None;
    }
    let class_name = symbols
        .iter()
        .find(|record| {
            record.kind == SymbolKind::Class && record.range.start_line == class_scope.line
        })?
        .name
        .clone();
    let expected_detail = format!("Method {class_name}.{name}");
    let active_symbol = symbols
        .iter()
        .filter(|record| {
            record.name == name
                && definition_scope_at_line(source, record.range.start_line).last()
                    == Some(&class_scope)
        })
        .max_by_key(|record| {
            (
                record.range.start_line,
                record.range.start_character,
                record.range.end_line,
                record.range.end_character,
            )
        })
        .filter(|record| {
            record.detail == expected_detail
                && matches!(
                    record.kind,
                    SymbolKind::Function | SymbolKind::CythonDeclaration
                )
        })?;
    let overwritten_by_assignment = source
        .lines()
        .enumerate()
        .skip(active_symbol.range.start_line as usize + 1)
        .any(|(line_index, line)| {
            let line_index = line_index as u32;
            let trimmed = line.trim_start();
            let indentation = line.len().saturating_sub(trimmed.len()) as u32;
            source_map
                .offset(line_index, indentation)
                .is_some_and(|offset| source_map.is_code_offset(offset))
                && simple_assignment_re()
                    .captures(trimmed)
                    .and_then(|captures| captures.name("name"))
                    .is_some_and(|assigned| assigned.as_str() == name)
                && definition_scope_at_line(source, line_index).last() == Some(&class_scope)
        });
    (!overwritten_by_assignment).then(|| active_symbol.clone())
}

fn local_receiver_binding_is_reliable(
    source: &str,
    source_map: &CodeMap,
    target_scope: &[DefinitionScope],
    class_scope: DefinitionScope,
    owner: &str,
    target_range: &SourceRange,
) -> bool {
    let Some(class_index) = target_scope.iter().rposition(|scope| *scope == class_scope) else {
        return false;
    };
    let function_scopes: Vec<_> = target_scope[class_index + 1..]
        .iter()
        .copied()
        .filter(|scope| scope.kind == DefinitionScopeKind::Function)
        .collect();
    let Some(direct_method) = function_scopes.first().copied() else {
        return false;
    };
    if function_has_staticmethod_decorator(source, direct_method.line) {
        return false;
    }
    let direct_parameters = parameter_symbols_for_function(
        "document",
        Path::new("document.py"),
        source,
        source_map,
        direct_method.line,
        true,
    );
    if direct_parameters
        .first()
        .map(|parameter| parameter.name.as_str())
        != Some(owner)
    {
        return false;
    }

    if function_scopes.iter().skip(1).any(|scope| {
        parameter_symbols_for_function(
            "document",
            Path::new("document.py"),
            source,
            source_map,
            scope.line,
            true,
        )
        .iter()
        .any(|parameter| parameter.name == owner)
    }) {
        return false;
    }
    if function_scopes.len() > 1 {
        let lexical_scopes = LexicalScopeMap::new(source);
        if function_scopes.iter().skip(1).any(|nested_function| {
            lexical_scopes.function_statically_binds_name(source, nested_function.line, owner)
        }) {
            return false;
        }
    }

    let scan_start = source_map.offset(direct_method.line, 0).unwrap_or_default();
    let Some(target_offset) =
        source_map.offset(target_range.start_line, target_range.start_character)
    else {
        return false;
    };
    if expression_scope_binds_owner_at_offset(source, source_map, owner, scan_start, target_offset)
    {
        return false;
    }

    !source
        .lines()
        .enumerate()
        .take(target_range.start_line as usize + 1)
        .skip(direct_method.line as usize + 1)
        .any(|(line_index, line)| {
            let line_index = line_index as u32;
            let trimmed = line.trim_start();
            let indentation = line.len().saturating_sub(trimmed.len()) as u32;
            source_map
                .offset(line_index, indentation)
                .is_some_and(|offset| source_map.is_code_offset(offset))
                && (line_rebinds_name(trimmed, owner)
                    || scope_declaration_names_owner(trimmed, owner))
                && scope_is_visible_from(
                    &definition_scope_at_line(source, line_index),
                    target_scope,
                )
        })
}

pub(super) fn expression_locally_binds_name_on_line(
    source: &str,
    name: &str,
    target_line: u32,
) -> bool {
    if name.is_empty() {
        return false;
    }
    let source_map = CodeMap::new(source);
    let Some((line_start, line)) = line_offsets(source).get(target_line as usize).copied() else {
        return false;
    };
    line.match_indices(name).any(|(relative_offset, _)| {
        let target_offset = line_start + relative_offset;
        let bytes = source.as_bytes();
        let end = target_offset + name.len();
        let starts_at_boundary =
            target_offset == 0 || !is_word_byte(bytes[target_offset.saturating_sub(1)]);
        let ends_at_boundary =
            end >= bytes.len() || bytes.get(end).is_none_or(|byte| !is_word_byte(*byte));
        starts_at_boundary
            && ends_at_boundary
            && source_map.is_code_offset(target_offset)
            && expression_scope_binds_owner_at_offset(
                source,
                &source_map,
                name,
                innermost_expression_start(source.as_bytes(), &source_map, target_offset)
                    .unwrap_or(line_start),
                target_offset,
            )
    })
}

fn innermost_expression_start(
    bytes: &[u8],
    source_map: &CodeMap,
    target_offset: usize,
) -> Option<usize> {
    let mut openings = Vec::new();
    for (offset, byte) in bytes
        .iter()
        .enumerate()
        .take(target_offset.min(bytes.len()))
    {
        if !source_map.is_code_offset(offset) {
            continue;
        }
        match *byte {
            b'(' | b'[' | b'{' => openings.push(offset),
            b')' | b']' | b'}' => {
                openings.pop();
            }
            _ => {}
        }
    }
    openings.last().copied()
}

fn expression_scope_binds_owner_at_offset(
    source: &str,
    source_map: &CodeMap,
    owner: &str,
    scan_start: usize,
    target_offset: usize,
) -> bool {
    lambda_parameter_binds_owner(source, source_map, owner, scan_start, target_offset)
        || enclosing_comprehension_binds_owner(source, source_map, owner, target_offset)
        || enclosing_expression_has_walrus_binding(source, source_map, owner, target_offset)
}

fn lambda_parameter_binds_owner(
    source: &str,
    source_map: &CodeMap,
    owner: &str,
    scan_start: usize,
    target_offset: usize,
) -> bool {
    let bytes = source.as_bytes();
    let scan_end = target_offset.min(bytes.len());
    let mut offset = scan_start.min(scan_end);
    while offset + "lambda".len() <= scan_end {
        if bytes.get(offset..offset + "lambda".len()) == Some(b"lambda")
            && source_map.is_code_offset(offset)
            && (offset == 0 || !is_word_byte(bytes[offset - 1]))
            && bytes
                .get(offset + "lambda".len())
                .is_none_or(|byte| !is_word_byte(*byte))
        {
            let parameters_start = offset + "lambda".len();
            if let Some(colon) =
                lambda_parameter_colon(bytes, source_map, parameters_start, scan_end)
            {
                if source[parameters_start..colon]
                    .split(',')
                    .filter_map(|parameter| parameter.split('=').next())
                    .map(|parameter| parameter.trim().trim_start_matches('*').trim())
                    .any(|parameter| parameter == owner)
                {
                    return true;
                }
                offset = colon;
            }
        }
        offset += 1;
    }
    false
}

fn lambda_parameter_colon(
    bytes: &[u8],
    source_map: &CodeMap,
    start: usize,
    end: usize,
) -> Option<usize> {
    let mut depth = 0usize;
    for (offset, byte) in bytes.iter().enumerate().take(end).skip(start) {
        if !source_map.is_code_offset(offset) {
            continue;
        }
        match *byte {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth = depth.saturating_sub(1),
            b':' if depth == 0 => return Some(offset),
            _ => {}
        }
    }
    None
}

fn enclosing_comprehension_binds_owner(
    source: &str,
    source_map: &CodeMap,
    owner: &str,
    target_offset: usize,
) -> bool {
    let bytes = source.as_bytes();
    let mut openings = Vec::new();
    for (offset, byte) in bytes
        .iter()
        .enumerate()
        .take(target_offset.min(bytes.len()))
    {
        if !source_map.is_code_offset(offset) {
            continue;
        }
        match *byte {
            b'(' | b'[' | b'{' => openings.push((offset, *byte)),
            b')' | b']' | b'}' => {
                openings.pop();
            }
            _ => {}
        }
    }
    openings.into_iter().rev().any(|(open, delimiter)| {
        matching_bracket_end(bytes, source_map, open, delimiter).is_some_and(|close| {
            comprehension_clause_binds_owner(source, source_map, owner, open + 1, close)
        })
    })
}

fn matching_bracket_end(
    bytes: &[u8],
    source_map: &CodeMap,
    open: usize,
    opening: u8,
) -> Option<usize> {
    let closing = match opening {
        b'(' => b')',
        b'[' => b']',
        b'{' => b'}',
        _ => return None,
    };
    let mut depth = 0usize;
    for (offset, byte) in bytes.iter().enumerate().skip(open) {
        if !source_map.is_code_offset(offset) {
            continue;
        }
        if *byte == opening {
            depth += 1;
        } else if *byte == closing {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                return Some(offset);
            }
        }
    }
    None
}

fn comprehension_clause_binds_owner(
    source: &str,
    source_map: &CodeMap,
    owner: &str,
    start: usize,
    end: usize,
) -> bool {
    let bytes = source.as_bytes();
    let mut offset = start;
    let mut in_target = false;
    let mut target_names_owner = false;
    while offset < end.min(bytes.len()) {
        if !source_map.is_code_offset(offset) || !is_word_byte(bytes[offset]) {
            offset += 1;
            continue;
        }
        let word_start = offset;
        while offset < end && is_word_byte(bytes[offset]) {
            offset += 1;
        }
        let word = &source[word_start..offset];
        if !in_target {
            if word == "for" {
                in_target = true;
                target_names_owner = false;
            }
        } else if word == "in" {
            if target_names_owner {
                return true;
            }
            in_target = false;
        } else if word == owner {
            target_names_owner = true;
        }
    }
    false
}

fn enclosing_expression_has_walrus_binding(
    source: &str,
    source_map: &CodeMap,
    owner: &str,
    target_offset: usize,
) -> bool {
    let bytes = source.as_bytes();
    let mut openings = Vec::new();
    for (offset, byte) in bytes
        .iter()
        .enumerate()
        .take(target_offset.min(bytes.len()))
    {
        if !source_map.is_code_offset(offset) {
            continue;
        }
        match *byte {
            b'(' | b'[' | b'{' => openings.push((offset, *byte)),
            b')' | b']' | b'}' => {
                openings.pop();
            }
            _ => {}
        }
    }
    openings.into_iter().rev().any(|(open, delimiter)| {
        matching_bracket_end(bytes, source_map, open, delimiter).is_some_and(|close| {
            walrus_binding_in_range(source, source_map, owner, open + 1, close)
        })
    })
}

fn walrus_binding_in_range(
    source: &str,
    source_map: &CodeMap,
    owner: &str,
    start: usize,
    end: usize,
) -> bool {
    let bytes = source.as_bytes();
    let mut offset = start.min(bytes.len());
    while offset + owner.len() <= end.min(bytes.len()) {
        if bytes.get(offset..offset + owner.len()) == Some(owner.as_bytes())
            && source_map.is_code_offset(offset)
            && (offset == 0 || !is_word_byte(bytes[offset - 1]))
            && bytes
                .get(offset + owner.len())
                .is_none_or(|byte| !is_word_byte(*byte))
        {
            let mut operator = offset + owner.len();
            while operator < end
                && bytes
                    .get(operator)
                    .is_some_and(|byte| byte.is_ascii_whitespace())
            {
                operator += 1;
            }
            if bytes.get(operator..operator + 2) == Some(b":=") {
                return true;
            }
        }
        offset += 1;
    }
    false
}

fn scope_declaration_names_owner(line: &str, owner: &str) -> bool {
    line.split(';').any(|statement| {
        let statement = statement.trim();
        let names = statement
            .strip_prefix("global ")
            .or_else(|| statement.strip_prefix("nonlocal "));
        names.is_some_and(|names| {
            names.split(',').any(|name| {
                name.trim_start()
                    .split(|ch: char| !(ch == '_' || ch.is_ascii_alphanumeric()))
                    .next()
                    == Some(owner)
            })
        })
    })
}

fn function_has_staticmethod_decorator(source: &str, function_line: u32) -> bool {
    let lines: Vec<_> = source.lines().collect();
    let Some(function_source_line) = lines.get(function_line as usize) else {
        return false;
    };
    let function_indent = line_indent(function_source_line);
    for line in lines[..function_line as usize].iter().rev() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if line_indent(line) != function_indent || !trimmed.starts_with('@') {
            break;
        }
        let decorator = trimmed
            .trim_start_matches('@')
            .split(|ch: char| ch == '(' || ch.is_whitespace())
            .next()
            .unwrap_or_default();
        if decorator.rsplit('.').next() == Some("staticmethod") {
            return true;
        }
    }
    false
}
