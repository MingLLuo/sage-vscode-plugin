use super::*;

pub fn semantic_spans(source: &str) -> Vec<SemanticSpan> {
    let mut spans = Vec::new();
    let code_map = CodeMap::new(source);
    let scopes = LexicalScopeMap::new(source);
    let lines: Vec<&str> = source.lines().collect();
    let mut module_bindings: HashMap<&str, Vec<u32>> = HashMap::new();
    let mut function_bindings: HashMap<(u32, &str), bool> = HashMap::new();
    for re in [class_re(), function_re()] {
        for captures in re.captures_iter(source) {
            if let Some(name) = captures.name("name") {
                if !code_map.is_code_offset(name.start()) {
                    continue;
                }
                let (line, character) = code_map.line_col(name.start());
                spans.push(SemanticSpan {
                    line,
                    start: character,
                    length: name.as_str().len() as u32,
                    token_type: if re.as_str().contains("class") {
                        "class".to_string()
                    } else {
                        "function".to_string()
                    },
                    modifiers: vec!["declaration".to_string()],
                });
            }
        }
    }
    for captures in preparser_re().captures_iter(source) {
        if let Some(parent) = captures.name("parent") {
            if !code_map.is_code_offset(parent.start()) {
                continue;
            }
            let (line, character) = code_map.line_col(parent.start());
            spans.push(SemanticSpan {
                line,
                start: character,
                length: parent.as_str().len() as u32,
                token_type: "variable".to_string(),
                modifiers: vec!["declaration".to_string()],
            });
        }
        if let Some(symbols) = captures.name("symbols") {
            if !code_map.is_code_offset(symbols.start()) {
                continue;
            }
            for name in symbols
                .as_str()
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                if let Some(offset) = source[symbols.start()..symbols.end()].find(name) {
                    let absolute = symbols.start() + offset;
                    let (line, character) = code_map.line_col(absolute);
                    spans.push(SemanticSpan {
                        line,
                        start: character,
                        length: name.len() as u32,
                        token_type: "parameter".to_string(),
                        modifiers: vec!["declaration".to_string()],
                    });
                }
            }
        }
    }
    for captures in semantic_assignment_re().captures_iter(source) {
        let Some(name) = captures.name("name") else {
            continue;
        };
        if !code_map.is_code_offset(name.start()) {
            continue;
        }
        let (line, character) = code_map.line_col(name.start());
        spans.push(SemanticSpan {
            line,
            start: character,
            length: name.as_str().len() as u32,
            token_type: "variable".to_string(),
            modifiers: vec!["declaration".to_string()],
        });
    }
    for captures in decorator_re().captures_iter(source) {
        if let Some(name) = captures.name("name") {
            if !code_map.is_code_offset(name.start()) {
                continue;
            }
            let (line, character) = code_map.line_col(name.start());
            spans.push(SemanticSpan {
                line,
                start: character,
                length: name.as_str().len() as u32,
                token_type: "decorator".to_string(),
                modifiers: if is_known_sage_decorator(name.as_str())
                    && !is_shadowed_name(
                        name.as_str(),
                        line,
                        source,
                        &lines,
                        &scopes,
                        &mut module_bindings,
                        &mut function_bindings,
                    ) {
                    vec!["defaultLibrary".to_string()]
                } else {
                    Vec::new()
                },
            });
        }
    }
    for captures in word_re().captures_iter(source) {
        let Some(name) = captures.name("name") else {
            continue;
        };
        if !code_map.is_code_offset(name.start()) {
            continue;
        }
        if name.start() > 0 && source.as_bytes()[name.start() - 1] == b'@' {
            continue;
        }
        let token = name.as_str();
        let (line, character) = code_map.line_col(name.start());
        let token_type = if SAGE_NAMESPACES.contains(&token) {
            "namespace"
        } else if SAGE_TYPES.contains(&token) {
            "type"
        } else if SAGE_FUNCTIONS.contains(&token) {
            "function"
        } else if SAGE_READONLY.contains(&token) {
            "variable"
        } else {
            continue;
        };
        if is_shadowed_name(
            token,
            line,
            source,
            &lines,
            &scopes,
            &mut module_bindings,
            &mut function_bindings,
        ) {
            continue;
        }
        let modifiers = if SAGE_READONLY.contains(&token) {
            vec!["readonly".to_string(), "defaultLibrary".to_string()]
        } else {
            vec!["defaultLibrary".to_string()]
        };
        spans.push(SemanticSpan {
            line,
            start: character,
            length: token.len() as u32,
            token_type: token_type.to_string(),
            modifiers,
        });
    }
    spans.sort_by(|left, right| {
        left.line
            .cmp(&right.line)
            .then(left.start.cmp(&right.start))
            .then(right.length.cmp(&left.length))
    });
    let mut filtered = Vec::with_capacity(spans.len());
    let mut last_line = None;
    let mut last_end = 0u32;
    for span in spans {
        if last_line != Some(span.line) {
            last_line = Some(span.line);
            last_end = 0;
        }
        if span.start < last_end {
            continue;
        }
        last_end = span.start.saturating_add(span.length);
        filtered.push(span);
    }
    filtered
}

fn is_known_sage_decorator(name: &str) -> bool {
    matches!(
        name,
        "interact"
            | "interact_manual"
            | "cached_method"
            | "cached_function"
            | "cached_in_parent_method"
            | "cached_property"
            | "lazy_attribute"
    )
}

fn is_shadowed_name<'a>(
    name: &'a str,
    line: u32,
    source: &str,
    lines: &[&str],
    scopes: &LexicalScopeMap,
    module_bindings: &mut HashMap<&'a str, Vec<u32>>,
    function_bindings: &mut HashMap<(u32, &'a str), bool>,
) -> bool {
    if scopes.is_function_parameter_at_header(line, name) {
        return true;
    }
    if scopes
        .enclosing_function_lines(line)
        .into_iter()
        .any(|function_line| {
            *function_bindings
                .entry((function_line, name))
                .or_insert_with(|| {
                    scopes.function_statically_binds_name(source, function_line, name)
                        && (scopes.is_function_parameter_at_header(function_line, name)
                            || lines.iter().enumerate().any(|(line_number, source_line)| {
                                let trimmed = source_line.trim_start();
                                scopes.is_code_line(line_number as u32)
                                    && scopes.is_direct_function_body_line(
                                        line_number as u32,
                                        function_line,
                                    )
                                    && line_rebinds_name(trimmed, name)
                                    && !is_sage_import_binding(trimmed)
                            }))
                })
        })
    {
        return true;
    }
    module_bindings
        .entry(name)
        .or_insert_with(|| {
            lines
                .iter()
                .enumerate()
                .filter_map(|(line_number, source_line)| {
                    let trimmed = source_line.trim_start();
                    (scopes.is_code_line(line_number as u32)
                        && scopes.is_module_scope_line(line_number as u32)
                        && line_rebinds_name(trimmed, name)
                        && !is_sage_import_binding(trimmed))
                    .then_some(line_number as u32)
                })
                .collect()
        })
        .iter()
        .any(|binding_line| *binding_line <= line)
}

fn is_sage_import_binding(line: &str) -> bool {
    line.starts_with("from sage.") || line.starts_with("from sage import ")
}
