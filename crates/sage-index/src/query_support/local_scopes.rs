//! Local definition-scope and source-symbol helpers shared by completion and navigation.

//! Local bindings and lightweight definition scopes shared by completion and resolution.

use super::*;

#[derive(Clone, Debug)]
pub(super) struct ScopedLocalCandidate {
    pub(super) record: SymbolRecord,
    pub(super) binding_scope: Vec<DefinitionScope>,
    pub(super) is_parameter: bool,
}

pub(super) fn parameter_candidates_for_target(
    module: &str,
    path: &Path,
    source: &str,
    target_range: &SourceRange,
    target_scope: &[DefinitionScope],
) -> Vec<ScopedLocalCandidate> {
    let mut function_scopes: Vec<_> = target_scope
        .iter()
        .copied()
        .filter(|scope| scope.kind == DefinitionScopeKind::Function)
        .collect();
    if source
        .lines()
        .nth(target_range.start_line as usize)
        .map(str::trim_start)
        .and_then(definition_scope_kind)
        == Some(DefinitionScopeKind::Function)
    {
        function_scopes.push(DefinitionScope {
            line: target_range.start_line,
            kind: DefinitionScopeKind::Function,
        });
    }
    function_scopes.sort_by_key(|scope| scope.line);
    function_scopes.dedup();

    if function_scopes.is_empty() {
        return Vec::new();
    }

    let code_map = CodeMap::new(source);
    let mut candidates = Vec::new();
    for function_scope in function_scopes {
        let mut binding_scope = definition_scope_at_line(source, function_scope.line);
        binding_scope.push(function_scope);
        candidates.extend(
            parameter_symbols_for_function(
                module,
                path,
                source,
                &code_map,
                function_scope.line,
                true,
            )
            .into_iter()
            .map(|record| ScopedLocalCandidate {
                record,
                binding_scope: binding_scope.clone(),
                is_parameter: true,
            }),
        );
    }
    candidates
}

pub(super) fn definition_scope_at_line(source: &str, target_line: u32) -> Vec<DefinitionScope> {
    let mut scope: Vec<(DefinitionScope, usize)> = Vec::new();
    for (line_index, line) in source.lines().enumerate().take(target_line as usize + 1) {
        let trimmed = line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let indent = line_indent(line);
        while scope
            .last()
            .is_some_and(|(_, scope_indent)| *scope_indent >= indent)
        {
            scope.pop();
        }
        if line_index as u32 == target_line {
            break;
        }
        if let Some(kind) = definition_scope_kind(trimmed) {
            scope.push((
                DefinitionScope {
                    line: line_index as u32,
                    kind,
                },
                indent,
            ));
        }
    }
    scope.into_iter().map(|(scope, _)| scope).collect()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum DefinitionScopeKind {
    Function,
    Class,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct DefinitionScope {
    pub(super) line: u32,
    pub(super) kind: DefinitionScopeKind,
}

fn definition_scope_kind(trimmed: &str) -> Option<DefinitionScopeKind> {
    if trimmed.starts_with("class ") || trimmed.starts_with("cdef class ") {
        return Some(DefinitionScopeKind::Class);
    }
    (trimmed.starts_with("def ")
        || trimmed.starts_with("async def ")
        || (trimmed.starts_with("cpdef ") && trimmed.contains('('))
        || (trimmed.starts_with("cdef ") && trimmed.contains('(')))
    .then_some(DefinitionScopeKind::Function)
}

pub(super) fn scope_is_visible_from(
    binding_scope: &[DefinitionScope],
    target_scope: &[DefinitionScope],
) -> bool {
    if !target_scope.starts_with(binding_scope) {
        return false;
    }
    binding_scope.len() == target_scope.len()
        || binding_scope
            .last()
            .is_none_or(|scope| scope.kind == DefinitionScopeKind::Function)
}

pub(super) fn is_local_shadow_before_or_at_target(
    record: &SymbolRecord,
    target_range: &SourceRange,
) -> bool {
    let same_range = record.range == *target_range;
    match record.kind {
        SymbolKind::Function | SymbolKind::Class | SymbolKind::CythonDeclaration => {
            same_range || record.range.start_line < target_range.start_line
        }
        SymbolKind::Variable | SymbolKind::PreparserGenerator => {
            same_range || record.range.start_line < target_range.start_line
        }
        SymbolKind::Import | SymbolKind::Module => false,
    }
}

pub(super) fn scoped_local_symbols(source: &str, position: QueryPosition) -> Vec<SymbolRecord> {
    let code_map = CodeMap::new(source);
    let scope = enclosing_function_scope(source, position);
    let mut symbols = Vec::new();
    symbols.extend(parameter_symbols_for_scope(source, &code_map, scope));
    symbols.extend(local_assignment_symbols_for_scope(
        source, &code_map, position, scope,
    ));
    symbols
}

fn enclosing_function_scope(source: &str, position: QueryPosition) -> Option<(u32, usize)> {
    let current_indent = source
        .lines()
        .nth(position.line as usize)
        .map(line_indent)
        .unwrap_or(0);
    if current_indent == 0 {
        return None;
    }
    let mut best = None;
    for (line_index, line) in source.lines().enumerate().take(position.line as usize + 1) {
        let trimmed = line.trim_start();
        if !(trimmed.starts_with("def ")
            || trimmed.starts_with("async def ")
            || trimmed.starts_with("cpdef ")
            || trimmed.starts_with("cdef "))
        {
            continue;
        }
        let indent = line_indent(line);
        if indent < current_indent {
            best = Some((line_index as u32, indent));
        }
    }
    best
}

fn parameter_symbols_for_scope(
    source: &str,
    code_map: &CodeMap,
    scope: Option<(u32, usize)>,
) -> Vec<SymbolRecord> {
    let Some((scope_line, _)) = scope else {
        return Vec::new();
    };
    parameter_symbols_for_function(
        "document",
        Path::new("document.py"),
        source,
        code_map,
        scope_line,
        false,
    )
}

pub(super) fn parameter_symbols_for_function(
    module: &str,
    path: &Path,
    source: &str,
    code_map: &CodeMap,
    scope_line: u32,
    include_receivers: bool,
) -> Vec<SymbolRecord> {
    let Some((line_start, line)) = line_offsets(source).into_iter().nth(scope_line as usize) else {
        return Vec::new();
    };
    let header_end = definition_header_end(source, line_start).unwrap_or(line_start + line.len());
    let header = &source[line_start..header_end];
    let Some(open) = header.find('(') else {
        return Vec::new();
    };
    let Some(close) = matching_close_paren(header, open) else {
        return Vec::new();
    };
    let mut symbols = Vec::new();
    for (raw, relative_start) in split_parameter_segments(&header[open + 1..close], open + 1) {
        let Some(name) = parameter_name(raw) else {
            continue;
        };
        if !include_receivers && matches!(name, "self" | "cls") {
            continue;
        }
        let Some(name_relative) = raw.find(name) else {
            continue;
        };
        let absolute = line_start + relative_start + name_relative;
        let (line, character) = code_map.line_col(absolute);
        symbols.push(SymbolRecord {
            name: name.to_string(),
            kind: SymbolKind::Variable,
            module: module.to_string(),
            path: path.to_path_buf(),
            range: SourceRange {
                start_line: line,
                start_character: character,
                end_line: line,
                end_character: character + name.len() as u32,
            },
            detail: format!("Local parameter {name}"),
            docstring: None,
            import_from: None,
            signature: None,
        });
    }
    symbols
}

fn matching_close_paren(text: &str, open: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut quote = None;
    for (index, ch) in text[open..].char_indices() {
        let absolute = open + index;
        if ch == '\'' || ch == '"' {
            quote = match quote {
                Some(current) if current == ch => None,
                None => Some(ch),
                current => current,
            };
            continue;
        }
        if quote.is_some() {
            continue;
        }
        match ch {
            '(' => depth += 1,
            ')' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(absolute);
                }
            }
            _ => {}
        }
    }
    None
}

fn split_parameter_segments(params: &str, base_offset: usize) -> Vec<(&str, usize)> {
    let mut segments = Vec::new();
    let mut start = 0usize;
    let mut depth = 0usize;
    let mut quote = None;
    for (index, ch) in params.char_indices() {
        if ch == '\'' || ch == '"' {
            quote = match quote {
                Some(current) if current == ch => None,
                None => Some(ch),
                current => current,
            };
            continue;
        }
        if quote.is_some() {
            continue;
        }
        match ch {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                segments.push((&params[start..index], base_offset + start));
                start = index + 1;
            }
            _ => {}
        }
    }
    segments.push((&params[start..], base_offset + start));
    segments
}

fn local_assignment_symbols_for_scope(
    source: &str,
    code_map: &CodeMap,
    position: QueryPosition,
    scope: Option<(u32, usize)>,
) -> Vec<SymbolRecord> {
    let mut symbols = Vec::new();
    for captures in semantic_assignment_re().captures_iter(source) {
        let Some(name) = captures.name("name") else {
            continue;
        };
        if !code_map.is_code_offset(name.start()) {
            continue;
        }
        let (line, character) = code_map.line_col(name.start());
        if line > position.line {
            continue;
        }
        let Some(source_line) = source.lines().nth(line as usize) else {
            continue;
        };
        let indent = line_indent(source_line);
        let in_scope = match scope {
            Some((scope_line, scope_indent)) => line > scope_line && indent > scope_indent,
            None => indent == 0,
        };
        if !in_scope {
            continue;
        }
        symbols.push(SymbolRecord {
            name: name.as_str().to_string(),
            kind: SymbolKind::Variable,
            module: "document".to_string(),
            path: PathBuf::from("document.py"),
            range: SourceRange {
                start_line: line,
                start_character: character,
                end_line: line,
                end_character: character + name.as_str().len() as u32,
            },
            detail: format!("Local variable {}", name.as_str()),
            docstring: None,
            import_from: None,
            signature: None,
        });
    }
    symbols
}

fn parameter_name(raw: &str) -> Option<&str> {
    let without_default = raw.split('=').next()?.trim();
    let without_annotation = without_default.split(':').next()?.trim();
    let name = without_annotation
        .trim_start_matches('*')
        .trim()
        .trim_start_matches('/');
    if !name.is_empty() && is_valid_identifier(name) {
        return Some(name);
    }
    // Cython commonly spells receivers as `Matrix self` or `Matrix* self`.
    // Python annotations already returned through the branch above, so the
    // final identifier is the binding name for the typed Cython form.
    let bytes = without_default.as_bytes();
    let mut end = bytes.len();
    while end > 0 && !is_word_byte(bytes[end - 1]) {
        end -= 1;
    }
    let mut start = end;
    while start > 0 && is_word_byte(bytes[start - 1]) {
        start -= 1;
    }
    let name = without_default.get(start..end)?;
    is_valid_identifier(name).then_some(name)
}

pub(super) fn line_indent(line: &str) -> usize {
    line.chars().take_while(|ch| ch.is_whitespace()).count()
}
