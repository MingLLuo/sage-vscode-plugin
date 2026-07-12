use super::*;

pub(crate) fn resolve_from_candidates(
    module_hint: Option<&str>,
    candidates: Vec<SymbolRecord>,
) -> Option<SymbolRecord> {
    if let Some(module_hint) = module_hint {
        if let Some(symbol) = candidates
            .iter()
            .find(|symbol| symbol.kind == SymbolKind::Import && symbol.module == module_hint)
        {
            if let Some(import_from) = &symbol.import_from {
                let (source_module, source_name) =
                    import_target_in_context(import_from, &symbol.name, &symbol.module);
                if let Some(resolved) = candidates
                    .iter()
                    .filter(|candidate| {
                        import_target_definition_matches(candidate, &source_module, &source_name)
                    })
                    .min_by_key(|candidate| symbol_choice_key(candidate))
                    .cloned()
                {
                    return Some(resolved);
                }
            }
            return Some(symbol.clone());
        }
        if let Some(symbol) = candidates
            .iter()
            .filter(|symbol| symbol.kind != SymbolKind::Import && symbol.module == module_hint)
            .min_by_key(|candidate| symbol_choice_key(candidate))
            .cloned()
        {
            return Some(symbol);
        }
    }
    best_symbol(candidates)
}

fn import_target(import_from: &str, fallback_name: &str) -> (String, String) {
    if let Some((module, name)) = import_from.split_once("::") {
        (module.to_string(), name.to_string())
    } else {
        (import_from.to_string(), fallback_name.to_string())
    }
}

pub(crate) fn import_target_in_context(
    import_from: &str,
    fallback_name: &str,
    importer_module: &str,
) -> (String, String) {
    let (module, name) = import_target(import_from, fallback_name);
    (resolve_relative_module(&module, importer_module), name)
}

pub(crate) fn normalize_import_from(
    import_from: &str,
    importer_module: &str,
    fallback_name: &str,
) -> String {
    let (module, name) = import_target_in_context(import_from, fallback_name, importer_module);
    if import_from.contains("::") {
        format!("{module}::{name}")
    } else {
        module
    }
}

pub(crate) fn resolve_relative_module(module: &str, importer_module: &str) -> String {
    if !module.starts_with('.') {
        return module.to_string();
    }
    let level = module.chars().take_while(|ch| *ch == '.').count();
    let rest = module[level..].trim_matches('.');
    let parts = importer_module.split('.').collect::<Vec<_>>();
    let base_len = parts.len().saturating_sub(level);
    let mut resolved = parts[..base_len].join(".");
    if !rest.is_empty() {
        if !resolved.is_empty() {
            resolved.push('.');
        }
        resolved.push_str(rest);
    }
    resolved
}

pub(crate) fn best_symbol(symbols: Vec<SymbolRecord>) -> Option<SymbolRecord> {
    symbols.into_iter().min_by_key(symbol_choice_key)
}

pub(crate) fn dedupe_best_symbols(symbols: Vec<SymbolRecord>, limit: usize) -> Vec<SymbolRecord> {
    let mut grouped: BTreeMap<String, Vec<SymbolRecord>> = BTreeMap::new();
    for symbol in dedupe_symbol_records(symbols) {
        grouped
            .entry(symbol.name.to_ascii_lowercase())
            .or_default()
            .push(symbol);
    }
    let mut results: Vec<_> = grouped.into_values().filter_map(best_symbol).collect();
    results.sort_by(|left, right| {
        left.name
            .cmp(&right.name)
            .then(left.module.cmp(&right.module))
    });
    results.truncate(limit);
    results
}

pub(crate) fn dedupe_symbol_records(symbols: Vec<SymbolRecord>) -> Vec<SymbolRecord> {
    let mut seen = BTreeSet::new();
    let mut deduped = Vec::new();
    for symbol in symbols {
        let key = (
            symbol.name.clone(),
            symbol_kind_as_str(&symbol.kind),
            symbol.module.clone(),
            symbol.path.clone(),
            symbol.range.start_line,
            symbol.range.start_character,
            symbol.range.end_line,
            symbol.range.end_character,
        );
        if seen.insert(key) {
            deduped.push(symbol);
        }
    }
    deduped
}

pub(crate) fn suppress_workspace_import_noise(symbols: Vec<SymbolRecord>) -> Vec<SymbolRecord> {
    let names_with_definitions: BTreeSet<String> = symbols
        .iter()
        .filter(|symbol| symbol.kind != SymbolKind::Import)
        .map(|symbol| symbol.name.to_ascii_lowercase())
        .collect();
    if names_with_definitions.is_empty() {
        return symbols;
    }
    symbols
        .into_iter()
        .filter(|symbol| {
            symbol.kind != SymbolKind::Import
                || !names_with_definitions.contains(&symbol.name.to_ascii_lowercase())
        })
        .collect()
}

pub(crate) fn workspace_symbol_sort_key(
    symbol: &SymbolRecord,
    needle: &str,
) -> (u8, u8, u8, usize) {
    let name = symbol.name.to_ascii_lowercase();
    let module = symbol.module.to_ascii_lowercase();
    let match_rank = if needle.is_empty() {
        3
    } else if name == needle {
        0
    } else if name.starts_with(needle) {
        1
    } else if symbol_word_boundary_match(&name, needle) {
        2
    } else if name.contains(needle) {
        3
    } else if module.contains(needle) {
        4
    } else {
        5
    };
    (
        match_rank,
        symbol_resolution_rank(&symbol.kind),
        symbol_path_rank(&symbol.path),
        symbol.name.len(),
    )
}

fn symbol_word_boundary_match(name: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }
    name.split('_').any(|part| part.starts_with(needle))
}

pub(crate) fn documentation_for_symbol(symbol: &SymbolRecord) -> DocumentationRecord {
    let uri =
        (!symbol.path.as_os_str().is_empty()).then(|| format!("file://{}", symbol.path.display()));
    DocumentationRecord {
        name: symbol.name.clone(),
        module_name: symbol.module.clone(),
        kind: format!("{:?}", symbol.kind),
        detail: symbol.detail.clone(),
        summary: symbol
            .docstring
            .as_deref()
            .and_then(documentation_summary)
            .unwrap_or_else(|| format!("{} from {}", symbol.name, symbol.module)),
        docstring: symbol.docstring.clone(),
        uri,
        markers: vec!["source:rust-index-v2".to_string()],
        sections: Vec::new(),
    }
}

pub(crate) fn documentation_has_specific_docstring(record: &DocumentationRecord) -> bool {
    record.docstring.as_deref().is_some_and(|docstring| {
        let docstring = docstring.trim();
        !docstring.is_empty() && !docstring.contains("Runtime documentation worker can provide")
    })
}

pub(crate) fn hover_markdown_for_symbol(
    symbol: &SymbolRecord,
    documentation: Option<&DocumentationRecord>,
) -> String {
    let mut lines = vec![
        "```sage".to_string(),
        symbol.detail.clone(),
        "```".to_string(),
        String::new(),
        format!("Module: `{}`", symbol.module),
    ];
    let docstring = documentation
        .and_then(|documentation| documentation.docstring.as_ref())
        .or(symbol.docstring.as_ref());
    if let Some(docstring) = docstring {
        if !docstring.is_empty() {
            lines.push(String::new());
            lines.push(compact_hover_docstring(docstring));
        }
    }
    lines.join("\n")
}

pub(crate) fn hover_markdown_for_ambiguous_member(documentation: &DocumentationRecord) -> String {
    let mut lines = vec![
        "```sage".to_string(),
        documentation.detail.clone(),
        "```".to_string(),
        String::new(),
        documentation.summary.clone(),
    ];
    if let Some(reason) = &documentation.docstring {
        lines.push(String::new());
        lines.push(reason.clone());
    }
    if !documentation.sections.is_empty() {
        lines.push(String::new());
        lines.push("Top indexed candidates:".to_string());
        for section in documentation.sections.iter().take(3) {
            lines.push(format!("- {}", section.title));
        }
    }
    lines.join("\n")
}

pub(crate) fn symbol_map_from_files(files: &[IndexedFile]) -> HashMap<String, Vec<SymbolRecord>> {
    let mut symbols_by_name: HashMap<String, Vec<SymbolRecord>> = HashMap::new();
    for file in files {
        for symbol in &file.symbols {
            symbols_by_name
                .entry(symbol.name.to_ascii_lowercase())
                .or_default()
                .push(symbol.clone());
        }
    }
    symbols_by_name
}

pub(crate) fn insert_file_symbol_names(names: &mut BTreeSet<String>, file: &IndexedFile) {
    insert_symbol_names(names, file.symbols.iter());
}

fn insert_symbol_names<'a>(
    names: &mut BTreeSet<String>,
    symbols: impl IntoIterator<Item = &'a SymbolRecord>,
) {
    for symbol in symbols {
        names.insert(symbol.name.to_ascii_lowercase());
        if let Some(import_from) = symbol.import_from.as_deref() {
            let (_module, source_name) =
                import_target_in_context(import_from, &symbol.name, &symbol.module);
            names.insert(source_name.to_ascii_lowercase());
        }
    }
}

pub(crate) fn paths_need_materialized_cache_refresh(
    changed: &[IndexedFile],
    deleted: &[PathBuf],
    roots: &[PathBuf],
) -> bool {
    changed
        .iter()
        .any(|file| module_needs_materialized_cache_refresh(&file.module))
        || deleted.iter().any(|path| {
            roots
                .iter()
                .find(|root| path.starts_with(root))
                .map(|root| module_name_from_path(root, path))
                .is_some_and(|module| module_needs_materialized_cache_refresh(&module))
        })
}

fn module_needs_materialized_cache_refresh(module: &str) -> bool {
    module == "sage.all" || module.starts_with("sage.")
}

pub(crate) fn documentation_summary(docstring: &str) -> Option<String> {
    docstring
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(str::to_string)
}

fn compact_hover_docstring(docstring: &str) -> String {
    const MAX_LINES: usize = 24;
    const MAX_BYTES: usize = 2400;

    let trimmed = docstring.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    let mut output = String::new();
    let mut truncated = false;
    for (line_count, line) in trimmed.lines().enumerate() {
        if line_count >= MAX_LINES || output.len() + line.len() + 1 > MAX_BYTES {
            truncated = true;
            break;
        }
        if !output.is_empty() {
            output.push('\n');
        }
        output.push_str(line);
    }

    if truncated {
        output.push_str("\n\n... (open Sage documentation for the full docstring)");
    }
    output
}

pub(crate) fn builtin_symbol_record(name: &str) -> Option<SymbolRecord> {
    let short_name = name.rsplit('.').next().unwrap_or(name);
    let (kind, module, detail) = if SAGE_NAMESPACES.contains(&short_name) {
        (
            SymbolKind::Module,
            "sage.all",
            format!("namespace {short_name}"),
        )
    } else if SAGE_TYPES.contains(&short_name) {
        (
            SymbolKind::Class,
            "sage.all",
            format!("constructor {short_name}"),
        )
    } else if SAGE_FUNCTIONS.contains(&short_name) {
        (
            SymbolKind::Function,
            "sage.all",
            format!("function {short_name}"),
        )
    } else if SAGE_READONLY.contains(&short_name) {
        (
            SymbolKind::Variable,
            "sage.all",
            format!("constant {short_name}"),
        )
    } else {
        return None;
    };
    Some(SymbolRecord {
        name: short_name.to_string(),
        kind,
        module: module.to_string(),
        path: PathBuf::new(),
        range: SourceRange::default(),
        detail,
        docstring: Some(format!(
            "Known Sage symbol `{}`. Runtime documentation worker can provide the full Sage documentation when enabled.",
            name
        )),
        import_from: None,
        signature: None,
    })
}
