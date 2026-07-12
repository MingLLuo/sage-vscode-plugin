use super::declarations::top_level_declared_symbol_names;
use super::import_parsing::*;
use super::support::{line_offsets, push_import_symbol};
use super::*;

pub(super) fn capture_imports(
    module: &str,
    path: &Path,
    source: &str,
    code_map: &CodeMap,
    symbols: &mut Vec<SymbolRecord>,
) {
    let mut multiline_from_import: Option<String> = None;
    for (line_start, line) in line_offsets(source) {
        if !code_map.is_code_offset(line_start) {
            continue;
        }
        let trimmed = line.trim_start();
        let indent = line.len() - trimmed.len();
        if let Some(import_module) = multiline_from_import.clone() {
            capture_multiline_import_names(
                module,
                path,
                MultilineImportCapture {
                    text: trimmed,
                    original_line: line,
                    line_start,
                    import_module: &import_module,
                },
                code_map,
                symbols,
            );
            if trimmed.contains(')') {
                multiline_from_import = None;
            }
            continue;
        }
        if let Some((import_module, rest)) = parse_multiline_from_import_start(trimmed) {
            capture_multiline_import_names(
                module,
                path,
                MultilineImportCapture {
                    text: rest,
                    original_line: line,
                    line_start,
                    import_module: &import_module,
                },
                code_map,
                symbols,
            );
            if !rest.contains(')') {
                multiline_from_import = Some(import_module);
            }
            continue;
        }
        if module_is_sage_all_export_module(module) {
            if let Some(import_module) = parse_star_import(trimmed) {
                push_import_symbol(
                    symbols,
                    module,
                    path,
                    SAGE_STAR_IMPORT_SENTINEL,
                    code_map,
                    line_start + indent,
                    &format!("{import_module}::*"),
                );
                continue;
            }
        }
        if let Some(import) =
            parse_from_import(trimmed, false).or_else(|| parse_from_import(trimmed, true))
        {
            for binding in import.bindings {
                if let Some(relative) = line.find(&binding.binding) {
                    push_import_symbol(
                        symbols,
                        module,
                        path,
                        &binding.binding,
                        code_map,
                        line_start + relative,
                        &format!("{}::{}", import.module, binding.source_name),
                    );
                }
            }
            continue;
        }
        if let Some(include_name) = parse_cython_include(trimmed) {
            push_import_symbol(
                symbols,
                module,
                path,
                &include_name,
                code_map,
                line_start + indent,
                &include_name,
            );
            continue;
        }
        if let Some(names) = parse_plain_import(trimmed) {
            for (name, import_from) in names {
                if let Some(relative) = line.find(&name) {
                    push_import_symbol(
                        symbols,
                        module,
                        path,
                        &name,
                        code_map,
                        line_start + relative,
                        &import_from,
                    );
                }
            }
        }
    }
}

pub(super) fn capture_lazy_imports(
    module: &str,
    path: &Path,
    source: &str,
    code_map: &CodeMap,
    symbols: &mut Vec<SymbolRecord>,
) {
    for (call_start, call) in lazy_import_calls(source, code_map) {
        for import in parse_lazy_imports(call) {
            let Some(relative) = string_literal_position(call, &import.binding)
                .or_else(|| string_literal_position(call, &import.target))
            else {
                continue;
            };
            push_import_symbol(
                symbols,
                module,
                path,
                &import.binding,
                code_map,
                call_start + relative,
                &format!("{}::{}", import.module, import.target),
            );
        }
    }
    for (binding_offset, binding, import) in lazy_import_object_assignments(source, code_map) {
        push_import_symbol(
            symbols,
            module,
            path,
            &binding,
            code_map,
            binding_offset,
            &format!("{}::{}", import.module, import.target),
        );
    }
}

pub(super) fn capture_import_alias_assignments(
    module: &str,
    path: &Path,
    source: &str,
    code_map: &CodeMap,
    symbols: &mut Vec<SymbolRecord>,
) {
    let imports_by_name: BTreeMap<String, String> = symbols
        .iter()
        .filter(|symbol| symbol.kind == SymbolKind::Import)
        .filter_map(|symbol| {
            symbol
                .import_from
                .as_ref()
                .map(|import_from| (symbol.name.clone(), import_from.clone()))
        })
        .collect();

    for (line_start, line) in line_offsets(source) {
        let trimmed = line
            .split('#')
            .next()
            .unwrap_or(line)
            .trim()
            .trim_end_matches(';')
            .trim();
        let Some(captures) = simple_assignment_re().captures(trimmed) else {
            continue;
        };
        let Some(alias) = captures.name("name") else {
            continue;
        };
        let Some(target) = captures.name("rhs") else {
            continue;
        };
        let target = target.as_str().trim();
        if !is_valid_identifier(target) {
            continue;
        }
        let Some(import_from) = imports_by_name.get(target) else {
            continue;
        };
        let Some(relative) = line.find(alias.as_str()) else {
            continue;
        };
        let offset = line_start + relative;
        if !code_map.is_code_offset(offset) {
            continue;
        }
        push_import_symbol(
            symbols,
            module,
            path,
            alias.as_str(),
            code_map,
            offset,
            import_from,
        );
    }
}

pub(super) fn capture_local_definition_alias_assignments(
    module: &str,
    path: &Path,
    source: &str,
    code_map: &CodeMap,
    symbols: &mut Vec<SymbolRecord>,
) {
    let declarations = top_level_declared_symbol_names(symbols);
    if declarations.is_empty() {
        return;
    }

    for (line_start, line) in line_offsets(source) {
        if line.trim_start() != line {
            continue;
        }
        let trimmed = line
            .split('#')
            .next()
            .unwrap_or(line)
            .trim()
            .trim_end_matches(';')
            .trim();
        let Some(captures) = simple_assignment_re().captures(trimmed) else {
            continue;
        };
        let Some(alias) = captures.name("name") else {
            continue;
        };
        let Some(target) = captures.name("rhs") else {
            continue;
        };
        let target = target.as_str().trim();
        if alias.as_str() == target || !declarations.contains(target) {
            continue;
        }
        let Some(relative) = line.find(alias.as_str()) else {
            continue;
        };
        let offset = line_start + relative;
        if !code_map.is_code_offset(offset) {
            continue;
        }
        push_import_symbol(
            symbols,
            module,
            path,
            alias.as_str(),
            code_map,
            offset,
            &format!("{module}::{target}"),
        );
        if let Some(symbol) = symbols.last_mut() {
            symbol.detail = format!("Import alias {} for {}", alias.as_str(), target);
        }
    }
}

pub(super) fn capture_import_member_alias_assignments(
    module: &str,
    path: &Path,
    source: &str,
    code_map: &CodeMap,
    symbols: &mut Vec<SymbolRecord>,
) {
    let imports_by_name: BTreeMap<String, String> = symbols
        .iter()
        .filter(|symbol| symbol.kind == SymbolKind::Import)
        .filter_map(|symbol| {
            symbol
                .import_from
                .as_ref()
                .map(|import_from| (symbol.name.clone(), import_from.clone()))
        })
        .collect();

    for (line_start, line) in line_offsets(source) {
        if line.trim_start() != line {
            continue;
        }
        let trimmed = line
            .split('#')
            .next()
            .unwrap_or(line)
            .trim()
            .trim_end_matches(';')
            .trim();
        let Some(captures) = member_alias_assignment_re().captures(trimmed) else {
            continue;
        };
        let Some(alias) = captures.name("name") else {
            continue;
        };
        let Some(owner) = captures.name("owner") else {
            continue;
        };
        let Some(member) = captures.name("member") else {
            continue;
        };
        let Some(owner_import) = imports_by_name.get(owner.as_str()) else {
            continue;
        };
        let Some(relative) = line.find(alias.as_str()) else {
            continue;
        };
        let offset = line_start + relative;
        if !code_map.is_code_offset(offset) {
            continue;
        }
        let target_module = imported_module_path(owner_import, owner.as_str(), module);
        push_import_symbol(
            symbols,
            module,
            path,
            alias.as_str(),
            code_map,
            offset,
            &format!("{}::{}", target_module, member.as_str()),
        );
        if let Some(symbol) = symbols.last_mut() {
            symbol.detail = format!(
                "Import alias {} for {}.{}",
                alias.as_str(),
                owner.as_str(),
                member.as_str()
            );
        }
    }
}

pub(super) fn capture_static_member_aliases(
    module: &str,
    path: &Path,
    source: &str,
    code_map: &CodeMap,
    symbols: &mut Vec<SymbolRecord>,
) {
    let imports_by_name: BTreeMap<String, String> = symbols
        .iter()
        .filter(|symbol| symbol.kind == SymbolKind::Import)
        .filter_map(|symbol| {
            symbol
                .import_from
                .as_ref()
                .map(|import_from| (symbol.name.clone(), import_from.clone()))
        })
        .collect();

    for (line_start, line) in line_offsets(source) {
        let Some(captures) = static_member_alias_re().captures(line) else {
            continue;
        };
        let Some(alias) = captures.name("name") else {
            continue;
        };
        let Some(module_alias) = captures.name("module") else {
            continue;
        };
        let Some(member) = captures.name("member") else {
            continue;
        };
        let Some(relative) = line.find(alias.as_str()) else {
            continue;
        };
        let offset = line_start + relative;
        if !code_map.is_code_offset(offset) {
            continue;
        }
        let Some(import_from) = imports_by_name.get(module_alias.as_str()) else {
            continue;
        };
        let target_module = imported_module_path(import_from, module_alias.as_str(), module);
        push_import_symbol(
            symbols,
            module,
            path,
            alias.as_str(),
            code_map,
            offset,
            &format!("{}::{}", target_module, member.as_str()),
        );
    }
}

pub(super) fn capture_deprecated_function_aliases(
    module: &str,
    path: &Path,
    source: &str,
    code_map: &CodeMap,
    symbols: &mut Vec<SymbolRecord>,
) {
    let imports_by_name: BTreeMap<String, String> = symbols
        .iter()
        .filter(|symbol| symbol.kind == SymbolKind::Import)
        .filter_map(|symbol| {
            symbol
                .import_from
                .as_ref()
                .map(|import_from| (symbol.name.clone(), import_from.clone()))
        })
        .collect();

    for (line_start, line) in line_offsets(source) {
        let Some(captures) = deprecated_function_alias_re().captures(line) else {
            continue;
        };
        let Some(alias) = captures.name("name") else {
            continue;
        };
        let Some(issue) = captures.name("issue") else {
            continue;
        };
        let Some(target) = captures.name("target") else {
            continue;
        };
        let Some(relative) = line.find(alias.as_str()) else {
            continue;
        };
        let offset = line_start + relative;
        if !code_map.is_code_offset(offset) {
            continue;
        }
        let Some(import_from) =
            deprecated_alias_import_target(module, target.as_str(), &imports_by_name)
        else {
            continue;
        };
        push_import_symbol(
            symbols,
            module,
            path,
            alias.as_str(),
            code_map,
            offset,
            &import_from,
        );
        if let Some(symbol) = symbols.last_mut() {
            symbol.detail = format!(
                "Deprecated alias {} for {} (Sage issue #{})",
                alias.as_str(),
                target.as_str(),
                issue.as_str()
            );
        }
    }
}

fn deprecated_alias_import_target(
    module: &str,
    target: &str,
    imports_by_name: &BTreeMap<String, String>,
) -> Option<String> {
    if let Some((owner, member)) = target.rsplit_once('.') {
        if !is_valid_identifier(member) {
            return None;
        }
        let owner_import = imports_by_name.get(owner)?;
        let target_module = imported_module_path(owner_import, owner, module);
        return Some(format!("{target_module}::{member}"));
    }
    if !is_valid_identifier(target) {
        return None;
    }
    imports_by_name
        .get(target)
        .cloned()
        .or_else(|| Some(format!("{module}::{target}")))
}

fn imported_module_path(import_from: &str, fallback_name: &str, importer_module: &str) -> String {
    if import_from.contains("::") {
        let (source_module, source_name) =
            import_target_in_context(import_from, fallback_name, importer_module);
        format!("{source_module}.{source_name}")
    } else {
        resolve_relative_module(import_from, importer_module)
    }
}
