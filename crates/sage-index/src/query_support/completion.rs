use super::*;

pub(crate) fn current_prefix(text: &str, line: u32, character: u32) -> Option<String> {
    let source_line = text.lines().nth(line as usize)?;
    let character = character.min(source_line.len() as u32) as usize;
    let bytes = source_line.as_bytes();
    let mut start = character;
    while start > 0 && is_word_byte(bytes[start - 1]) {
        start -= 1;
    }
    Some(source_line[start..character].to_string())
}

pub(crate) fn is_code_completion_position(source: &str, position: QueryPosition) -> bool {
    if source.is_empty() {
        return true;
    }
    let code_map = CodeMap::new(source);
    let Some(offset) = code_map.offset(position.line, position.character) else {
        return false;
    };
    let check_offset = if offset >= source.len() {
        offset.saturating_sub(1)
    } else {
        offset
    };
    code_map.is_code_offset(check_offset)
}

pub(crate) fn local_completion_items(
    source: &str,
    position: QueryPosition,
    prefix: &str,
    limit: usize,
) -> Vec<QueryCompletion> {
    if limit == 0 {
        return Vec::new();
    }
    let mut records = parse_source("document", Path::new("document.py"), source).symbols;
    records.extend(scoped_local_symbols(source, position));

    let needle = prefix.to_ascii_lowercase();
    let mut seen = BTreeSet::new();
    let mut completions = Vec::new();
    for record in records {
        if completions.len() >= limit {
            break;
        }
        if !needle.is_empty() && !record.name.to_ascii_lowercase().starts_with(&needle) {
            continue;
        }
        if !should_offer_document_symbol(&record, position) {
            continue;
        }
        if seen.insert(record.name.to_ascii_lowercase()) {
            completions.push(completion_from_symbol(record));
        }
    }
    completions
}

pub(crate) fn local_shadow_symbol_from_source(
    module: &str,
    path: &Path,
    source: &str,
    name: &str,
    target_range: &SourceRange,
) -> Option<SymbolRecord> {
    let symbols = parse_source(module, path, source).symbols;
    local_shadow_symbol_from_symbols(module, path, source, &symbols, name, target_range)
}

pub(crate) fn local_shadow_symbol_from_symbols(
    module: &str,
    path: &Path,
    source: &str,
    symbols: &[SymbolRecord],
    name: &str,
    target_range: &SourceRange,
) -> Option<SymbolRecord> {
    let target_scope = definition_scope_at_line(source, target_range.start_line);
    let mut candidates: Vec<_> = symbols
        .iter()
        // Scope discovery walks source up to the declaration line. Filter first so
        // reference validation does not repeat that walk for every unrelated symbol
        // in a large module.
        .filter(|record| {
            record.name == name
                && record.kind != SymbolKind::Import
                && is_local_shadow_before_or_at_target(record, target_range)
        })
        .cloned()
        .map(|record| ScopedLocalCandidate {
            binding_scope: definition_scope_at_line(source, record.range.start_line),
            record,
            is_parameter: false,
        })
        .collect();
    candidates.extend(parameter_candidates_for_target(
        module,
        path,
        source,
        target_range,
        &target_scope,
    ));
    candidates
        .into_iter()
        .filter(|candidate| {
            candidate.record.name == name
                && candidate.record.kind != SymbolKind::Import
                && (candidate.record.range == *target_range
                    || scope_is_visible_from(&candidate.binding_scope, &target_scope))
        })
        .min_by_key(|candidate| {
            (
                usize::MAX - candidate.binding_scope.len(),
                u8::from(!candidate.is_parameter),
                target_range
                    .start_line
                    .saturating_sub(candidate.record.range.start_line),
                symbol_choice_key(&candidate.record),
            )
        })
        .map(|candidate| candidate.record)
}

pub(crate) fn is_local_parameter_symbol(record: &SymbolRecord) -> bool {
    record.kind == SymbolKind::Variable && record.detail.starts_with("Local parameter ")
}

pub(crate) fn local_parameter_reference_matches(
    module: &str,
    path: &Path,
    source: &str,
    reference_range: &SourceRange,
    target: &SymbolRecord,
) -> bool {
    is_local_parameter_symbol(target)
        && local_shadow_symbol_from_source(module, path, source, &target.name, reference_range)
            .is_some_and(|candidate| {
                candidate.name == target.name
                    && candidate.module == target.module
                    && candidate.path == target.path
                    && candidate.range == target.range
                    && candidate.detail == target.detail
            })
}

pub fn local_import_alias_symbol_from_source(
    module: &str,
    path: &Path,
    source: &str,
    name: &str,
    target_range: &SourceRange,
) -> Option<SymbolRecord> {
    let symbols = parse_source(module, path, source).symbols;
    local_import_alias_symbol_from_symbols(source, &symbols, name, target_range)
}

pub fn local_import_alias_symbol_from_symbols(
    source: &str,
    symbols: &[SymbolRecord],
    name: &str,
    target_range: &SourceRange,
) -> Option<SymbolRecord> {
    let record = local_import_symbol_from_symbols(source, symbols, name, target_range)?;
    is_explicit_import_alias_symbol(&record).then_some(record)
}

pub fn local_import_alias_symbol_from_source_name(
    source: &str,
    symbols: &[SymbolRecord],
    source_name: &str,
    source_range: &SourceRange,
) -> Option<SymbolRecord> {
    // Most references are ordinary uses. Avoid reparsing import syntax unless the indexed
    // declarations show an explicit alias on the same source line.
    if !symbols.iter().any(|record| {
        record.kind == SymbolKind::Import
            && record.range.start_line == source_range.start_line
            && record.name != source_name
            && record
                .import_from
                .as_deref()
                .and_then(|import_from| import_from.rsplit_once("::"))
                .is_some_and(|(_, imported_name)| imported_name == source_name)
    }) {
        return None;
    }
    let binding = source_aliased_import_at_range(source, source_name, source_range)?;
    let mut record = symbols
        .iter()
        .find(|record| {
            record.kind == SymbolKind::Import
                && record.name == binding.binding_name
                && record.range == binding.binding_range
        })?
        .clone();
    if let Some(import_from) = source_import_from_at_range(source, &record.name, &record.range) {
        record.detail = format!("Import {} from {import_from}", record.name);
        record.import_from = Some(import_from);
    }
    record
        .import_from
        .as_deref()
        .and_then(|import_from| import_from.rsplit_once("::"))
        .is_some_and(|(_, imported_name)| {
            imported_name == source_name && record.name != source_name
        })
        .then_some(record)
}

pub(crate) fn local_import_symbol_from_source(
    module: &str,
    path: &Path,
    source: &str,
    name: &str,
    target_range: &SourceRange,
) -> Option<SymbolRecord> {
    let symbols = parse_source(module, path, source).symbols;
    local_import_symbol_from_symbols(source, &symbols, name, target_range)
}

pub(crate) fn local_import_symbol_from_symbols(
    source: &str,
    symbols: &[SymbolRecord],
    name: &str,
    target_range: &SourceRange,
) -> Option<SymbolRecord> {
    let target_scope = definition_scope_at_line(source, target_range.start_line);
    let mut record = symbols
        .iter()
        .filter(|record| record.kind == SymbolKind::Import && record.name == name)
        .filter(|record| {
            source_range_starts_before_or_at(&record.range, target_range)
                && (record.range == *target_range
                    || scope_is_visible_from(
                        &definition_scope_at_line(source, record.range.start_line),
                        &target_scope,
                    ))
        })
        .cloned()
        .min_by_key(|record| {
            let scope_depth = definition_scope_at_line(source, record.range.start_line).len();
            (
                usize::MAX - scope_depth,
                target_range
                    .start_line
                    .saturating_sub(record.range.start_line),
                target_range
                    .start_character
                    .saturating_sub(record.range.start_character),
            )
        })?;
    if let Some(import_from) = source_import_from_at_range(source, &record.name, &record.range) {
        record.detail = format!("Import {} from {import_from}", record.name);
        record.import_from = Some(import_from);
    }
    if local_binding_shadows_import(source, symbols, name, target_range, &target_scope, &record) {
        return None;
    }
    Some(record)
}

fn local_binding_shadows_import(
    source: &str,
    symbols: &[SymbolRecord],
    name: &str,
    target_range: &SourceRange,
    target_scope: &[DefinitionScope],
    import: &SymbolRecord,
) -> bool {
    let import_scope = definition_scope_at_line(source, import.range.start_line);
    let mut candidates: Vec<_> = symbols
        .iter()
        .filter(|record| {
            record.name == name
                && record.kind != SymbolKind::Import
                && is_local_shadow_before_or_at_target(record, target_range)
        })
        .cloned()
        .map(|record| ScopedLocalCandidate {
            binding_scope: definition_scope_at_line(source, record.range.start_line),
            record,
            is_parameter: false,
        })
        .collect();
    candidates.extend(parameter_candidates_for_target(
        &import.module,
        &import.path,
        source,
        target_range,
        target_scope,
    ));

    candidates.into_iter().any(|candidate| {
        if candidate.record.name != name
            || !scope_is_visible_from(&candidate.binding_scope, target_scope)
            || candidate.binding_scope.len() < import_scope.len()
        {
            return false;
        }
        if candidate.binding_scope.len() > import_scope.len() {
            return true;
        }
        candidate.binding_scope == import_scope
            && source_range_starts_before_or_at(&import.range, &candidate.record.range)
            && import.range != candidate.record.range
    })
}

fn is_explicit_import_alias_symbol(record: &SymbolRecord) -> bool {
    if record.kind != SymbolKind::Import {
        return false;
    }
    record
        .import_from
        .as_deref()
        .and_then(|import_from| import_from.rsplit_once("::"))
        .is_some_and(|(_, source_name)| {
            source_name != record.name && is_valid_identifier(source_name)
        })
}

fn source_range_starts_before_or_at(left: &SourceRange, right: &SourceRange) -> bool {
    left.start_line < right.start_line
        || (left.start_line == right.start_line && left.start_character <= right.start_character)
}

#[derive(Clone, Debug)]
struct ScopedLocalCandidate {
    record: SymbolRecord,
    binding_scope: Vec<DefinitionScope>,
    is_parameter: bool,
}

fn parameter_candidates_for_target(
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

fn is_local_shadow_before_or_at_target(record: &SymbolRecord, target_range: &SourceRange) -> bool {
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

fn should_offer_document_symbol(record: &SymbolRecord, position: QueryPosition) -> bool {
    match record.kind {
        SymbolKind::Class
        | SymbolKind::Function
        | SymbolKind::CythonDeclaration
        | SymbolKind::PreparserGenerator => true,
        SymbolKind::Import => !is_star_import_symbol(record) && !is_all_export_symbol(record),
        SymbolKind::Variable => record.range.start_line <= position.line,
        SymbolKind::Module => false,
    }
}

pub(crate) fn completion_from_symbol(record: SymbolRecord) -> QueryCompletion {
    let documentation = record.docstring.as_ref().map(|docstring| {
        if let Some(signature) = &record.signature {
            format!("```sage\n{signature}\n```\n\n{docstring}")
        } else {
            docstring.clone()
        }
    });
    QueryCompletion {
        label: record.name.clone(),
        kind: format!("{:?}", record.kind),
        detail: record.detail.clone(),
        signature: record.signature,
        documentation,
        resolve_name: Some(record.name),
        module: Some(record.module),
    }
}

fn scoped_local_symbols(source: &str, position: QueryPosition) -> Vec<SymbolRecord> {
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

#[derive(Clone, Debug)]
pub(crate) struct MemberCompletionContext {
    pub(crate) owner: String,
    pub(crate) prefix: String,
}

pub(crate) fn member_completion_context(
    source: &str,
    position: QueryPosition,
) -> Option<MemberCompletionContext> {
    let code_map = CodeMap::new(source);
    let offset = code_map.offset(position.line, position.character)?;
    if offset > 0 && !code_map.is_code_offset(offset - 1) {
        return None;
    }
    let source_line = source.lines().nth(position.line as usize)?;
    let character = position.character.min(source_line.len() as u32) as usize;
    let bytes = source_line.as_bytes();
    let mut prefix_start = character;
    while prefix_start > 0 && is_word_byte(bytes[prefix_start - 1]) {
        prefix_start -= 1;
    }
    if prefix_start == 0 || bytes[prefix_start - 1] != b'.' {
        return None;
    }
    let dot = prefix_start - 1;
    let owner_start = python_primary_start(source_line, dot)?;
    if owner_start >= dot {
        return None;
    }
    let owner = source_line[owner_start..dot].trim();
    if owner.is_empty() {
        return None;
    }
    Some(MemberCompletionContext {
        owner: owner.to_string(),
        prefix: source_line[prefix_start..character].to_string(),
    })
}

pub(crate) fn infer_completion_owner_type(
    source: &str,
    owner: &str,
    line: u32,
) -> Option<SageOwnerType> {
    infer_owner_type_before(source, owner, "", line)
        .or_else(|| infer_owner_type_from_owner_expression(owner, ""))
        .or_else(|| {
            owner_base_identifier(owner).and_then(|name| {
                infer_owner_type_from_completion_owner_name(name)
                    .or_else(|| infer_owner_type_from_name(name))
            })
        })
        .or_else(|| infer_owner_type_from_completion_owner_name(owner))
        .or_else(|| infer_owner_type_from_name(owner))
}

fn infer_owner_type_from_completion_owner_name(name: &str) -> Option<SageOwnerType> {
    let lower = name.to_ascii_lowercase();
    if name == "matrix" {
        return Some(SageOwnerType::MatrixConstructor);
    }
    if matches!(
        name,
        "A" | "G"
            | "M"
            | "P"
            | "Q"
            | "Q0"
            | "Q0inv"
            | "Qa"
            | "S1"
            | "T"
            | "base"
            | "base_inv"
            | "symbolic_obj"
            | "numeric_obj"
    ) || lower.contains("mat")
        || lower.ends_with("matrix")
    {
        return Some(SageOwnerType::Matrix);
    }
    if matches!(
        name,
        "u" | "v"
            | "target_u"
            | "u_candidate"
            | "vec"
            | "vec_obj"
            | "signature"
            | "normalized_signature"
    ) || lower.ends_with("vec")
        || lower.ends_with("vector")
    {
        return Some(SageOwnerType::Vector);
    }
    if matches!(name, "field" | "F" | "K") || lower.ends_with("_field") {
        return Some(SageOwnerType::Field);
    }
    if matches!(name, "curve" | "elliptic_curve") || lower.ends_with("_curve") {
        return Some(SageOwnerType::EllipticCurve);
    }
    if matches!(name, "graph" | "digraph") || lower.ends_with("_graph") {
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
    if matches!(name, "value" | "element" | "entry" | "x" | "y" | "root") {
        return Some(SageOwnerType::FieldElement);
    }
    if matches!(
        name,
        "f" | "f1" | "f2" | "poly" | "polynomial" | "fac" | "factor"
    ) || lower.contains("poly")
        || lower.ends_with("_factor")
    {
        return Some(SageOwnerType::PolynomialElement);
    }
    None
}

pub(crate) fn method_completion_from_record(
    owner_type: SageOwnerType,
    label: &str,
    record: Option<&SymbolRecord>,
) -> QueryCompletion {
    let detail = record
        .map(|record| {
            if record.name != label {
                format!("{} (alias for {})", record.detail, record.name)
            } else {
                record.detail.clone()
            }
        })
        .filter(|detail| !detail.trim().is_empty())
        .unwrap_or_else(|| format!("Sage {} method", owner_type.as_str()));
    QueryCompletion {
        label: label.to_string(),
        kind: record
            .map(|record| format!("{:?}", record.kind))
            .unwrap_or_else(|| "Method".to_string()),
        detail,
        signature: record.and_then(|record| record.signature.clone()),
        documentation: record.and_then(|record| {
            record.docstring.as_ref().map(|docstring| {
                if let Some(signature) = &record.signature {
                    format!("```sage\n{signature}\n```\n\n{docstring}")
                } else {
                    docstring.clone()
                }
            })
        }),
        resolve_name: record
            .map(|record| record.name.clone())
            .or_else(|| Some(label.to_string())),
        module: record.map(|record| record.module.clone()),
    }
}
