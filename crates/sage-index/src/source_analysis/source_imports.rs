use super::import_parsing::*;
use super::*;

pub(super) fn source_import_from_at_range(
    source: &str,
    binding_name: &str,
    range: &SourceRange,
) -> Option<String> {
    let target_line = range.start_line as usize;
    let target_character = range.start_character as usize;
    let mut multiline_module: Option<String> = None;

    for (line_index, line) in source.lines().enumerate().take(target_line + 1) {
        let trimmed = line.trim_start();
        if let Some(module) = multiline_module.as_deref() {
            if line_index == target_line {
                return from_import_target_on_line(
                    line,
                    trimmed.trim_end_matches(')').trim(),
                    module,
                    binding_name,
                    target_character,
                );
            }
            if trimmed.contains(')') {
                multiline_module = None;
            }
            continue;
        }

        if let Some((module, rest)) = parse_multiline_from_import_start(trimmed) {
            if line_index == target_line {
                return from_import_target_on_line(
                    line,
                    rest.trim_end_matches(')').trim(),
                    &module,
                    binding_name,
                    target_character,
                );
            }
            if !rest.contains(')') {
                multiline_module = Some(module);
            }
            continue;
        }

        if line_index != target_line {
            continue;
        }
        if let Some(import) =
            parse_from_import(trimmed, false).or_else(|| parse_from_import(trimmed, true))
        {
            return import.bindings.into_iter().find_map(|binding| {
                import_binding_matches_range(line, &binding, binding_name, target_character)
                    .then(|| format!("{}::{}", import.module, binding.source_name))
            });
        }
        if let Some(imports) = parse_plain_import(trimmed) {
            return imports
                .into_iter()
                .find_map(|(binding, module, explicitly_aliased)| {
                    (binding == binding_name
                        && import_binding_offset(line, &binding, explicitly_aliased)
                            == Some(target_character))
                    .then(|| {
                        if explicitly_aliased {
                            let source_name = module.rsplit('.').next().unwrap_or(&module);
                            format!("{module}::{source_name}")
                        } else {
                            module
                        }
                    })
                });
        }
    }
    None
}

pub(super) fn source_aliased_import_at_range(
    source: &str,
    source_name: &str,
    range: &SourceRange,
) -> Option<SourceAliasedImport> {
    if range.start_line != range.end_line
        || range.end_character.saturating_sub(range.start_character) != source_name.len() as u32
    {
        return None;
    }
    let code_map = CodeMap::new(source);
    let start = code_map.offset(range.start_line, range.start_character)?;
    let end = code_map.offset(range.end_line, range.end_character)?;
    if !code_map.is_code_offset(start)
        || source.as_bytes().get(start..end) != Some(source_name.as_bytes())
    {
        return None;
    }

    let target_line = range.start_line as usize;
    let target_character = range.start_character as usize;
    let mut multiline_import = false;
    for (line_index, line) in source.lines().enumerate().take(target_line + 1) {
        let trimmed = line.trim_start();
        if multiline_import {
            if line_index == target_line {
                return aliased_import_on_line(
                    line,
                    trimmed.trim_end_matches(')').trim(),
                    source_name,
                    target_character,
                    range.start_line,
                );
            }
            if trimmed.contains(')') {
                multiline_import = false;
            }
            continue;
        }

        if let Some((_module, rest)) = parse_multiline_from_import_start(trimmed) {
            if line_index == target_line {
                return aliased_import_on_line(
                    line,
                    rest.trim_end_matches(')').trim(),
                    source_name,
                    target_character,
                    range.start_line,
                );
            }
            if !rest.contains(')') {
                multiline_import = true;
            }
            continue;
        }

        if line_index != target_line {
            continue;
        }
        let import =
            parse_from_import(trimmed, false).or_else(|| parse_from_import(trimmed, true))?;
        return import.bindings.into_iter().find_map(|binding| {
            source_aliased_import_from_binding(
                line,
                binding,
                source_name,
                target_character,
                range.start_line,
            )
        });
    }
    None
}

fn aliased_import_on_line(
    original_line: &str,
    entries: &str,
    source_name: &str,
    target_character: usize,
    line: u32,
) -> Option<SourceAliasedImport> {
    entries.split(',').find_map(|entry| {
        source_aliased_import_from_binding(
            original_line,
            parse_imported_binding(entry)?,
            source_name,
            target_character,
            line,
        )
    })
}

fn source_aliased_import_from_binding(
    line: &str,
    binding: ImportedBinding,
    source_name: &str,
    target_character: usize,
    line_number: u32,
) -> Option<SourceAliasedImport> {
    if binding.source_name != source_name {
        return None;
    }
    let binding_start = aliased_import_binding_offset(line, &binding, target_character)?;
    Some(SourceAliasedImport {
        binding_range: SourceRange {
            start_line: line_number,
            start_character: binding_start as u32,
            end_line: line_number,
            end_character: (binding_start + binding.binding.len()) as u32,
        },
        binding_name: binding.binding,
    })
}

fn from_import_target_on_line(
    original_line: &str,
    entries: &str,
    module: &str,
    binding_name: &str,
    target_character: usize,
) -> Option<String> {
    entries.split(',').find_map(|entry| {
        let binding = parse_imported_binding(entry)?;
        import_binding_matches_range(original_line, &binding, binding_name, target_character)
            .then(|| format!("{}::{}", module, binding.source_name))
    })
}

fn import_binding_matches_range(
    line: &str,
    binding: &ImportedBinding,
    binding_name: &str,
    target_character: usize,
) -> bool {
    binding.binding == binding_name
        && import_binding_offset(
            line,
            &binding.binding,
            binding.binding != binding.source_name,
        ) == Some(target_character)
}

pub(super) fn source_imported_sage_all_star_lookup(
    source: &str,
    binding_name: &str,
) -> Option<SourceImportLookup> {
    source.lines().find_map(|line| {
        if line.len() != line.trim_start().len() {
            return None;
        }
        let trimmed = line
            .split('#')
            .next()
            .unwrap_or(line)
            .trim()
            .trim_end_matches(';')
            .trim();
        let rest = trimmed.strip_prefix("from ")?;
        let (module, names) = rest
            .split_once(" import ")
            .or_else(|| rest.split_once(" cimport "))?;
        let import_module = module.trim();
        (module_is_sage_all_export_module(import_module) && names.trim() == "*").then(|| {
            SourceImportLookup {
                import_module: import_module.to_string(),
                source_name: binding_name.to_string(),
            }
        })
    })
}

pub(super) fn is_sage_source_path(path: &Path) -> bool {
    path.extension().is_some_and(|ext| ext == "sage")
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
