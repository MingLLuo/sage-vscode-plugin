use super::*;

pub fn semantic_spans(source: &str) -> Vec<SemanticSpan> {
    let mut spans = Vec::new();
    let code_map = CodeMap::new(source);
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
                modifiers: vec!["defaultLibrary".to_string()],
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
        let (line, character) = code_map.line_col(name.start());
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
