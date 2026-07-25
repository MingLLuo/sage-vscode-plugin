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
