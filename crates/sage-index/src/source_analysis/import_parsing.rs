use super::support::push_import_symbol;
use super::*;

#[derive(Clone, Debug)]
pub(super) struct ParsedImport {
    pub(super) module: String,
    pub(super) bindings: Vec<ImportedBinding>,
}

#[derive(Clone, Debug)]
pub(super) struct ImportedBinding {
    pub(super) binding: String,
    pub(super) source_name: String,
}

#[derive(Clone, Debug)]
pub(super) struct ParsedLazyImport {
    pub(super) module: String,
    pub(super) target: String,
    pub(super) binding: String,
}

pub(super) fn lazy_import_calls<'a>(source: &'a str, code_map: &CodeMap) -> Vec<(usize, &'a str)> {
    let mut calls = Vec::new();
    for (start, _) in source.match_indices("lazy_import(") {
        if !code_map.is_code_offset(start) {
            continue;
        }
        let Some(end) = matching_python_call_end(&source[start..]) else {
            continue;
        };
        calls.push((start, &source[start..start + end]));
    }
    calls
}

pub(super) fn lazy_import_object_assignments(
    source: &str,
    code_map: &CodeMap,
) -> Vec<(usize, String, ParsedLazyImport)> {
    let mut assignments = Vec::new();
    for (start, _) in source.match_indices("LazyImport(") {
        if !code_map.is_code_offset(start) {
            continue;
        }
        let Some((binding_offset, binding)) = assignment_binding_before_call(source, start) else {
            continue;
        };
        let Some(end) = matching_python_call_end(&source[start..]) else {
            continue;
        };
        let call = &source[start..start + end];
        let Some(import) = parse_lazy_import_object_call(call, &binding) else {
            continue;
        };
        assignments.push((binding_offset, binding, import));
    }
    assignments
}

fn assignment_binding_before_call(source: &str, call_start: usize) -> Option<(usize, String)> {
    let line_start = source[..call_start]
        .rfind('\n')
        .map(|index| index + 1)
        .unwrap_or(0);
    let prefix = &source[line_start..call_start];
    let eq = prefix.rfind('=')?;
    let lhs = prefix[..eq].trim();
    if !is_valid_identifier(lhs) {
        return None;
    }
    let lhs_offset = prefix[..eq].rfind(lhs)?;
    Some((line_start + lhs_offset, lhs.to_string()))
}

fn parse_lazy_import_object_call(call: &str, binding: &str) -> Option<ParsedLazyImport> {
    let args = lazy_import_argument_text(call)?;
    let args = split_top_level_args(args);
    let module = args.first().and_then(|arg| first_string_literal(arg))?;
    let target = args
        .get(1)
        .and_then(|arg| string_literal_args(arg).into_iter().next())?;
    if !is_valid_identifier(binding) || !is_valid_identifier(&target) {
        return None;
    }
    Some(ParsedLazyImport {
        module,
        target,
        binding: binding.to_string(),
    })
}

pub(super) fn matching_python_call_end(text: &str) -> Option<usize> {
    let mut depth = 0usize;
    let mut chars = text.char_indices().peekable();
    while let Some((index, ch)) = chars.next() {
        if ch == '\'' || ch == '"' {
            skip_python_string(ch, &mut chars);
            continue;
        }
        match ch {
            '(' | '[' | '{' => depth = depth.saturating_add(1),
            ')' | ']' | '}' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(index + ch.len_utf8());
                }
            }
            _ => {}
        }
    }
    None
}

pub(super) fn skip_python_string(
    quote: char,
    chars: &mut std::iter::Peekable<std::str::CharIndices<'_>>,
) {
    while let Some((_, inner)) = chars.next() {
        if inner == '\\' {
            let _ = chars.next();
            continue;
        }
        if inner == quote {
            break;
        }
    }
}

pub(super) fn parse_lazy_imports(call: &str) -> Vec<ParsedLazyImport> {
    let Some(args) = lazy_import_argument_text(call) else {
        return Vec::new();
    };
    let args = split_top_level_args(args);
    let Some(module) = args.first().and_then(|arg| first_string_literal(arg)) else {
        return Vec::new();
    };
    let Some(targets) = args.get(1).map(|arg| string_literal_args(arg)) else {
        return Vec::new();
    };
    let alias_arg = args
        .iter()
        .skip(2)
        .find_map(|arg| {
            let trimmed = arg.trim();
            if let Some((key, value)) = trimmed.split_once('=') {
                (key.trim() == "as_").then_some(value.trim())
            } else {
                Some(trimmed)
            }
        })
        .filter(|arg| !arg.is_empty() && *arg != "None");
    let aliases = alias_arg.map(string_literal_args).unwrap_or_default();
    targets
        .into_iter()
        .enumerate()
        .filter_map(|(index, target)| {
            if !is_valid_identifier(&target) {
                return None;
            }
            let binding = aliases
                .get(index)
                .or_else(|| (aliases.len() == 1).then(|| aliases.first()).flatten())
                .cloned()
                .unwrap_or_else(|| target.clone());
            is_valid_identifier(&binding).then_some(ParsedLazyImport {
                module: module.clone(),
                target,
                binding,
            })
        })
        .collect()
}

pub(super) fn lazy_import_argument_text(call: &str) -> Option<&str> {
    let open = call.find('(')?;
    let close = call.rfind(')')?;
    (open < close).then(|| &call[open + 1..close])
}

fn split_top_level_args(args: &str) -> Vec<&str> {
    let mut result = Vec::new();
    let mut depth = 0usize;
    let mut start = 0usize;
    let mut chars = args.char_indices().peekable();
    while let Some((index, ch)) = chars.next() {
        if ch == '\'' || ch == '"' {
            skip_python_string(ch, &mut chars);
            continue;
        }
        match ch {
            '(' | '[' | '{' => depth = depth.saturating_add(1),
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                let value = args[start..index].trim();
                if !value.is_empty() {
                    result.push(value);
                }
                start = index + ch.len_utf8();
            }
            _ => {}
        }
    }
    let value = args[start..].trim();
    if !value.is_empty() {
        result.push(value);
    }
    result
}

fn first_string_literal(text: &str) -> Option<String> {
    string_literal_args(text).into_iter().next()
}

pub(super) fn string_literal_position(text: &str, value: &str) -> Option<usize> {
    for quote in ['\'', '"'] {
        let needle = format!("{quote}{value}{quote}");
        if let Some(index) = text.find(&needle) {
            return Some(index + quote.len_utf8());
        }
    }
    text.find(value)
}

pub(super) fn string_literal_args(text: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut chars = text.char_indices().peekable();
    while let Some((_, ch)) = chars.next() {
        if ch != '\'' && ch != '"' {
            continue;
        }
        let quote = ch;
        let mut value = String::new();
        while let Some((_, inner)) = chars.next() {
            if inner == '\\' {
                if let Some((_, escaped)) = chars.next() {
                    value.push(escaped);
                }
                continue;
            }
            if inner == quote {
                break;
            }
            value.push(inner);
        }
        result.push(value);
    }
    result
}

pub(super) fn parse_from_import(line: &str, cython: bool) -> Option<ParsedImport> {
    let keyword = if cython { " cimport " } else { " import " };
    let prefix = "from ";
    let line = line.strip_prefix(prefix)?;
    let (module, names) = line.split_once(keyword)?;
    Some(ParsedImport {
        module: module.trim().to_string(),
        bindings: names
            .split(',')
            .filter_map(parse_imported_binding)
            .collect(),
    })
}

pub(super) fn parse_star_import(line: &str) -> Option<String> {
    let line = line
        .split('#')
        .next()
        .unwrap_or(line)
        .trim()
        .trim_end_matches(';')
        .trim();
    let rest = line.strip_prefix("from ")?;
    let (module, names) = rest
        .split_once(" import ")
        .or_else(|| rest.split_once(" cimport "))?;
    (names.trim() == "*").then(|| module.trim().to_string())
}

pub(super) fn parse_multiline_from_import_start(line: &str) -> Option<(String, &str)> {
    let line = line.strip_prefix("from ")?;
    let (module, rest) = line
        .split_once(" import ")
        .or_else(|| line.split_once(" cimport "))?;
    let rest = rest.trim_start();
    let rest = rest.strip_prefix('(')?;
    Some((module.trim().to_string(), rest))
}

pub(super) struct MultilineImportCapture<'a> {
    pub(super) text: &'a str,
    pub(super) original_line: &'a str,
    pub(super) line_start: usize,
    pub(super) import_module: &'a str,
}

pub(super) fn capture_multiline_import_names(
    module: &str,
    path: &Path,
    capture: MultilineImportCapture<'_>,
    code_map: &CodeMap,
    symbols: &mut Vec<SymbolRecord>,
) {
    let text = capture
        .text
        .split('#')
        .next()
        .unwrap_or(capture.text)
        .replace(')', "");
    for entry in text.split(',') {
        let Some(binding) = parse_imported_binding(entry) else {
            continue;
        };
        if let Some(relative) = import_binding_offset(
            capture.original_line,
            &binding.binding,
            binding.binding != binding.source_name,
        ) {
            push_import_symbol(
                symbols,
                module,
                path,
                &binding.binding,
                code_map,
                capture.line_start + relative,
                &format!("{}::{}", capture.import_module, binding.source_name),
            );
        }
    }
}

pub(super) fn import_binding_offset(
    line: &str,
    binding: &str,
    explicitly_aliased: bool,
) -> Option<usize> {
    let code = line.split('#').next().unwrap_or(line);
    let tokens = import_identifier_tokens(code);
    let import_token = tokens
        .iter()
        .position(|(token, _)| matches!(*token, "import" | "cimport"));
    tokens
        .iter()
        .enumerate()
        .find(|(index, (token, _))| {
            if *token != binding {
                return false;
            }
            let preceded_by_as = index
                .checked_sub(1)
                .and_then(|previous| tokens.get(previous))
                .is_some_and(|(previous, _)| *previous == "as");
            if explicitly_aliased {
                preceded_by_as
            } else {
                !preceded_by_as && import_token.is_none_or(|import| *index > import)
            }
        })
        .map(|(_, (_, start))| *start)
}

pub(super) fn aliased_import_binding_offset(
    line: &str,
    binding: &ImportedBinding,
    source_offset: usize,
) -> Option<usize> {
    if binding.binding == binding.source_name {
        return None;
    }
    let code = line.split('#').next().unwrap_or(line);
    import_identifier_tokens(code)
        .windows(3)
        .find_map(|tokens| {
            let (source_name, source_start) = tokens[0];
            let (keyword, _) = tokens[1];
            let (binding_name, binding_start) = tokens[2];
            (source_name == binding.source_name
                && source_start == source_offset
                && keyword == "as"
                && binding_name == binding.binding)
                .then_some(binding_start)
        })
}

fn import_identifier_tokens(line: &str) -> Vec<(&str, usize)> {
    let bytes = line.as_bytes();
    let mut tokens = Vec::new();
    let mut index = 0usize;
    while index < bytes.len() {
        if bytes[index] != b'_' && !bytes[index].is_ascii_alphabetic() {
            index += 1;
            continue;
        }
        let start = index;
        index += 1;
        while index < bytes.len() && is_word_byte(bytes[index]) {
            index += 1;
        }
        tokens.push((&line[start..index], start));
    }
    tokens
}

pub(super) fn parse_plain_import(line: &str) -> Option<Vec<(String, String, bool)>> {
    let rest = line
        .strip_prefix("import ")
        .or_else(|| line.strip_prefix("cimport "))?;
    Some(
        rest.split(',')
            .filter_map(|entry| {
                let module = entry.split_whitespace().next()?.to_string();
                let explicit_alias = entry
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .windows(2)
                    .find_map(|window| (window[0] == "as").then(|| window[1].to_string()));
                let explicitly_aliased = explicit_alias.is_some();
                let binding = explicit_alias
                    .unwrap_or_else(|| module.split('.').next().unwrap_or(&module).to_string());
                is_valid_identifier(&binding).then_some((binding, module, explicitly_aliased))
            })
            .collect(),
    )
}

pub(super) fn parse_cython_include(line: &str) -> Option<String> {
    let rest = line.strip_prefix("include ")?;
    Some(rest.trim().trim_matches('"').trim_matches('\'').to_string())
}

pub(super) fn parse_imported_binding(entry: &str) -> Option<ImportedBinding> {
    let entry = entry.trim();
    if entry.is_empty() || entry == "*" {
        return None;
    }
    let parts: Vec<_> = entry.split_whitespace().collect();
    let source_name = parts
        .first()
        .copied()
        .filter(|value| is_valid_identifier(value))?;
    let binding = parts
        .iter()
        .position(|part| *part == "as")
        .and_then(|alias_index| parts.get(alias_index + 1).copied())
        .unwrap_or(source_name);
    is_valid_identifier(binding).then(|| ImportedBinding {
        binding: binding.to_string(),
        source_name: source_name.to_string(),
    })
}

pub(super) fn sage_export_import_from(import_from: &str, name: &str) -> Option<String> {
    let module = import_from
        .split_once("::")
        .map_or(import_from, |(module, _)| module);
    let target = SAGE_EXPORT_MAP
        .iter()
        .find(|target| target.import_module == module && target.name == name)
        .or_else(|| {
            (module == "sage.all")
                .then(|| {
                    SAGE_EXPORT_MAP
                        .iter()
                        .find(|target| target.name == name && target.import_module != "sage.all")
                })
                .flatten()
        })?;
    Some(format!("{}::{}", target.source_module, target.source_name))
}
