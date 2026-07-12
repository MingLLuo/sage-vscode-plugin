use super::support::{
    function_signature, line_offsets, push_import_symbol, push_simple_symbol,
    push_symbol_with_detail, SymbolPushContext,
};
use super::*;

pub(super) fn capture_declarations(
    module: &str,
    path: &Path,
    source: &str,
    code_map: &CodeMap,
    symbols: &mut Vec<SymbolRecord>,
) {
    let context = DeclarationCaptureContext {
        module,
        path,
        source,
        code_map,
    };
    let mut class_stack: Vec<(usize, String)> = Vec::new();
    for (line_start, line) in line_offsets(source) {
        let trimmed = line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let indent = line.len() - trimmed.len();
        let code_offset = line_start + indent;
        if !code_map.is_code_offset(code_offset) {
            continue;
        }
        while class_stack
            .last()
            .is_some_and(|(class_indent, _)| indent <= *class_indent)
        {
            class_stack.pop();
        }
        if let Some(captures) = class_re().captures(trimmed) {
            let Some(name) = captures.name("name") else {
                continue;
            };
            let offset = line_start + indent + name.start();
            push_declaration_symbol(
                &context,
                symbols,
                name.as_str(),
                offset,
                SymbolKind::Class,
                None,
            );
            class_stack.push((indent, name.as_str().to_string()));
            continue;
        }
        if let Some(captures) = function_re().captures(trimmed) {
            let Some(name) = captures.name("name") else {
                continue;
            };
            let offset = line_start + indent + name.start();
            let actual_kind = if is_cython_path(path) {
                SymbolKind::CythonDeclaration
            } else {
                SymbolKind::Function
            };
            let enclosing_class = class_stack
                .last()
                .filter(|(class_indent, _)| indent > *class_indent)
                .map(|(_, class_name)| class_name.as_str());
            push_declaration_symbol(
                &context,
                symbols,
                name.as_str(),
                offset,
                actual_kind,
                enclosing_class,
            );
        }
    }
}

pub(super) fn capture_class_method_aliases(
    module: &str,
    path: &Path,
    source: &str,
    code_map: &CodeMap,
    symbols: &mut Vec<SymbolRecord>,
) {
    let method_targets: BTreeSet<(String, String)> = symbols
        .iter()
        .filter(|symbol| is_source_derived_sage_method(symbol))
        .filter_map(|symbol| {
            method_detail_parts(&symbol.detail)
                .map(|(class_name, method_name)| (class_name.to_string(), method_name.to_string()))
        })
        .collect();
    if method_targets.is_empty() {
        return;
    }

    let mut class_stack: Vec<(usize, String)> = Vec::new();
    for (line_start, line) in line_offsets(source) {
        let trimmed = line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let indent = line.len() - trimmed.len();
        let code_offset = line_start + indent;
        if !code_map.is_code_offset(code_offset) {
            continue;
        }
        while class_stack
            .last()
            .is_some_and(|(class_indent, _)| indent <= *class_indent)
        {
            class_stack.pop();
        }
        if let Some(captures) = class_re().captures(trimmed) {
            if let Some(name) = captures.name("name") {
                class_stack.push((indent, name.as_str().to_string()));
            }
            continue;
        }
        let Some((class_indent, class_name)) = class_stack.last() else {
            continue;
        };
        if indent != class_indent.saturating_add(4) {
            continue;
        }
        let trimmed_assignment = trimmed
            .split('#')
            .next()
            .unwrap_or(trimmed)
            .trim()
            .trim_end_matches(';')
            .trim();
        let Some(captures) = simple_assignment_re().captures(trimmed_assignment) else {
            continue;
        };
        let Some(alias) = captures.name("name") else {
            continue;
        };
        let Some(target) = captures.name("rhs") else {
            continue;
        };
        let target = target.as_str().trim();
        if alias.as_str() == target
            || !is_valid_identifier(target)
            || (alias.as_str().starts_with("__") && alias.as_str().ends_with("__"))
            || !method_targets.contains(&(class_name.clone(), target.to_string()))
        {
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
            symbol.detail = format!(
                "MethodAlias {}.{} for {}",
                class_name,
                alias.as_str(),
                target
            );
        }
    }
}

pub(super) fn capture_matrix_constructor_method_aliases(
    module: &str,
    path: &Path,
    source: &str,
    code_map: &CodeMap,
    symbols: &mut Vec<SymbolRecord>,
) {
    if !source.contains("@matrix_method") {
        return;
    }

    let mut pending_matrix_method: Option<Option<String>> = None;
    for (line_start, line) in line_offsets(source) {
        let trimmed = line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let indent = line.len() - trimmed.len();
        if indent != 0 {
            continue;
        }
        let code_offset = line_start + indent;
        if !code_map.is_code_offset(code_offset) {
            continue;
        }
        if let Some(alias) = parse_matrix_method_decorator(trimmed) {
            pending_matrix_method = Some(alias);
            continue;
        }
        if trimmed.starts_with('@') {
            continue;
        }
        let Some(alias_override) = pending_matrix_method.take() else {
            continue;
        };
        let declaration = function_re()
            .captures(trimmed)
            .and_then(|captures| captures.name("name"))
            .or_else(|| {
                class_re()
                    .captures(trimmed)
                    .and_then(|captures| captures.name("name"))
            });
        let Some(name) = declaration else {
            continue;
        };
        let target_name = name.as_str();
        let alias = alias_override.unwrap_or_else(|| matrix_method_alias_name(target_name));
        if !is_valid_identifier(&alias) {
            continue;
        }
        let offset = line_start + indent + name.start();
        push_import_symbol(
            symbols,
            module,
            path,
            &alias,
            code_map,
            offset,
            &format!("{module}::{target_name}"),
        );
        if let Some(symbol) = symbols.last_mut() {
            symbol.detail =
                format!("MatrixConstructorMethodAlias matrix.{alias} for {target_name}");
        }
    }
}

fn parse_matrix_method_decorator(trimmed: &str) -> Option<Option<String>> {
    let rest = trimmed.strip_prefix("@matrix_method")?;
    if rest.trim().is_empty() {
        return Some(None);
    }
    if !rest.trim_start().starts_with('(') {
        return None;
    }
    let explicit = matrix_method_name_override_re()
        .captures(rest)
        .and_then(|captures| {
            captures
                .name("double")
                .or_else(|| captures.name("single"))
                .map(|value| value.as_str().to_string())
        });
    Some(explicit)
}

fn matrix_method_alias_name(target_name: &str) -> String {
    let alias = target_name.replace("matrix", "");
    let alias = alias.trim_matches('_');
    if alias.is_empty() {
        target_name.to_string()
    } else {
        alias.to_string()
    }
}

struct DeclarationCaptureContext<'a> {
    module: &'a str,
    path: &'a Path,
    source: &'a str,
    code_map: &'a CodeMap,
}

fn push_declaration_symbol(
    context: &DeclarationCaptureContext<'_>,
    symbols: &mut Vec<SymbolRecord>,
    name: &str,
    offset: usize,
    kind: SymbolKind,
    enclosing_class: Option<&str>,
) {
    if !context.code_map.is_code_offset(offset) {
        return;
    }
    let detail = if let Some(class_name) = enclosing_class {
        format!("Method {class_name}.{name}")
    } else {
        format!("{kind:?} {name}")
    };
    let (line, character) = context.code_map.line_col(offset);
    symbols.push(SymbolRecord {
        name: name.to_string(),
        kind: kind.clone(),
        module: context.module.to_string(),
        path: context.path.to_path_buf(),
        range: SourceRange {
            start_line: line,
            start_character: character,
            end_line: line,
            end_character: character + name.len() as u32,
        },
        detail,
        docstring: doc_after_offset(context.source, offset + name.len()),
        import_from: None,
        signature: function_signature(context.source, offset, name),
    });
}

pub(super) fn capture_preparser_generators(
    module: &str,
    path: &Path,
    source: &str,
    code_map: &CodeMap,
    symbols: &mut Vec<SymbolRecord>,
) {
    for captures in preparser_re().captures_iter(source) {
        if let Some(parent) = captures.name("parent") {
            if !code_map.is_code_offset(parent.start()) {
                continue;
            }
            push_simple_symbol(
                symbols,
                module,
                path,
                parent.as_str(),
                SymbolKind::Variable,
                code_map,
                parent.start(),
            );
        }
        if let Some(generators) = captures.name("symbols") {
            if !code_map.is_code_offset(generators.start()) {
                continue;
            }
            for name in generators
                .as_str()
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                if let Some(relative) = source[generators.start()..generators.end()].find(name) {
                    push_simple_symbol(
                        symbols,
                        module,
                        path,
                        name,
                        SymbolKind::PreparserGenerator,
                        code_map,
                        generators.start() + relative,
                    );
                }
            }
        }
    }
}

pub(super) fn capture_assignments(
    module: &str,
    path: &Path,
    source: &str,
    code_map: &CodeMap,
    symbols: &mut Vec<SymbolRecord>,
) {
    let context = SymbolPushContext {
        module,
        path,
        code_map,
    };
    let declared_names = top_level_declared_symbol_names(symbols);
    for captures in assignment_re().captures_iter(source) {
        let Some(name) = captures.name("name") else {
            continue;
        };
        if !code_map.is_code_offset(name.start()) {
            continue;
        }
        let (line, _) = code_map.line_col(name.start());
        if symbols.iter().any(|symbol| {
            symbol.name == name.as_str() && symbol.path == path && symbol.range.start_line == line
        }) {
            continue;
        }
        let rhs = captures
            .name("rhs")
            .map(|value| value.as_str())
            .unwrap_or_default();
        if rhs.trim_start().starts_with("deprecated_function_alias(") {
            continue;
        }
        if member_reference_re().is_match(rhs.trim()) {
            continue;
        }
        if declared_names.contains(rhs.trim()) {
            continue;
        }
        let detail = assignment_detail(
            name.as_str(),
            captures.name("annotation").map(|value| value.as_str()),
            rhs,
        );
        push_symbol_with_detail(
            symbols,
            &context,
            name.as_str(),
            SymbolKind::Variable,
            name.start(),
            detail,
        );
    }
}

pub(super) fn top_level_declared_symbol_names(symbols: &[SymbolRecord]) -> BTreeSet<String> {
    symbols
        .iter()
        .filter(|symbol| {
            matches!(
                symbol.kind,
                SymbolKind::Class | SymbolKind::Function | SymbolKind::CythonDeclaration
            )
        })
        .map(|symbol| symbol.name.clone())
        .collect()
}
