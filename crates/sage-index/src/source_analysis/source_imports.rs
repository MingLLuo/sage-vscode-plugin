use super::import_parsing::parse_imported_binding;
use super::*;

pub(super) fn source_explicit_import_lookup(
    source: &str,
    binding_name: &str,
) -> Option<SourceImportLookup> {
    let mut multiline_module: Option<String> = None;
    for line in source.lines() {
        let trimmed = line
            .split('#')
            .next()
            .unwrap_or(line)
            .trim()
            .trim_end_matches(';')
            .trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(module) = multiline_module.as_deref() {
            let entries = trimmed.trim_end_matches(')').trim();
            if let Some(source_name) = imported_source_name(entries, binding_name) {
                return Some(SourceImportLookup {
                    import_module: module.to_string(),
                    source_name,
                });
            }
            if trimmed.contains(')') {
                multiline_module = None;
            } else {
                multiline_module = Some(module.to_string());
            }
            continue;
        }
        let Some(rest) = trimmed.strip_prefix("from ") else {
            continue;
        };
        let Some((module, names)) = rest
            .split_once(" import ")
            .or_else(|| rest.split_once(" cimport "))
        else {
            continue;
        };
        let module = module.trim();
        let names = names.trim_start();
        if names == "*" {
            continue;
        }
        if let Some(after_open) = names.strip_prefix('(') {
            let entries = after_open.trim_end_matches(')').trim();
            if let Some(source_name) = imported_source_name(entries, binding_name) {
                return Some(SourceImportLookup {
                    import_module: module.to_string(),
                    source_name,
                });
            }
            if !names.contains(')') {
                multiline_module = Some(module.to_string());
            }
            continue;
        }
        if let Some(source_name) = imported_source_name(names, binding_name) {
            return Some(SourceImportLookup {
                import_module: module.to_string(),
                source_name,
            });
        }
    }
    None
}

pub(super) fn source_imported_sage_all_lookup(
    source: &str,
    binding_name: &str,
) -> Option<SourceImportLookup> {
    let mut star_imports = Vec::new();
    let mut multiline_module: Option<String> = None;
    for line in source.lines() {
        let trimmed = line
            .split('#')
            .next()
            .unwrap_or(line)
            .trim()
            .trim_end_matches(';')
            .trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(module) = multiline_module.as_deref() {
            let entries = trimmed.trim_end_matches(')').trim();
            if let Some(source_name) = imported_source_name(entries, binding_name) {
                return Some(SourceImportLookup {
                    import_module: module.to_string(),
                    source_name,
                });
            }
            if trimmed.contains(')') {
                multiline_module = None;
            } else {
                multiline_module = Some(module.to_string());
            }
            continue;
        }
        let Some(rest) = trimmed.strip_prefix("from ") else {
            continue;
        };
        let Some((module, names)) = rest
            .split_once(" import ")
            .or_else(|| rest.split_once(" cimport "))
        else {
            continue;
        };
        let module = module.trim();
        if !module_is_sage_all_export_module(module) {
            continue;
        }
        let names = names.trim_start();
        if names == "*" {
            star_imports.push(module.to_string());
            continue;
        }
        if let Some(after_open) = names.strip_prefix('(') {
            let entries = after_open.trim_end_matches(')').trim();
            if let Some(source_name) = imported_source_name(entries, binding_name) {
                return Some(SourceImportLookup {
                    import_module: module.to_string(),
                    source_name,
                });
            }
            if !names.contains(')') {
                multiline_module = Some(module.to_string());
            }
            continue;
        }
        if let Some(source_name) = imported_source_name(names, binding_name) {
            return Some(SourceImportLookup {
                import_module: module.to_string(),
                source_name,
            });
        }
    }
    star_imports
        .into_iter()
        .next()
        .map(|import_module| SourceImportLookup {
            import_module,
            source_name: binding_name.to_string(),
        })
}

pub(super) fn is_sage_source_path(path: &Path) -> bool {
    path.extension().is_some_and(|ext| ext == "sage")
}

fn imported_source_name(entries: &str, binding_name: &str) -> Option<String> {
    entries.split(',').find_map(|entry| {
        let binding = parse_imported_binding(entry)?;
        (binding.binding == binding_name).then_some(binding.source_name)
    })
}

pub(super) fn sage_load_attach_paths_before_line(
    query_path: &Path,
    source: &str,
    max_line: u32,
) -> Vec<PathBuf> {
    let base_dir = query_path.parent().unwrap_or_else(|| Path::new("."));
    let code_map = CodeMap::new(source);
    let mut paths = Vec::new();
    let mut seen = BTreeSet::new();
    for captures in load_attach_call_re().captures_iter(source) {
        let Some(call) = captures.get(0) else {
            continue;
        };
        if !code_map.is_code_offset(call.start()) {
            continue;
        }
        let (line, _) = code_map.line_col(call.start());
        if line > max_line {
            continue;
        }
        let Some(target) = captures
            .name("double")
            .or_else(|| captures.name("single"))
            .map(|value| value.as_str())
        else {
            continue;
        };
        if target.trim().is_empty() {
            continue;
        }
        let target_path = PathBuf::from(target);
        let resolved = if target_path.is_absolute() {
            target_path
        } else {
            base_dir.join(target_path)
        };
        let resolved = normalize_path(resolved);
        if seen.insert(resolved.clone()) {
            paths.push(resolved);
        }
    }
    paths
}

fn load_attach_call_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r#"\b(?:load|attach)\s*\(\s*(?:"(?P<double>[^"\n]+)"|'(?P<single>[^'\n]+)')"#)
            .unwrap()
    })
}
