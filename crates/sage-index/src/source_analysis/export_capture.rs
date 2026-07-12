use super::import_parsing::{
    lazy_import_argument_text, matching_python_call_end, skip_python_string, string_literal_args,
    string_literal_position,
};
use super::support::line_offsets;
use super::*;

pub(super) fn capture_all_exports(
    module: &str,
    path: &Path,
    source: &str,
    code_map: &CodeMap,
    symbols: &mut Vec<SymbolRecord>,
) {
    let mut explicit_offsets = Vec::new();
    let mut entries = Vec::<(usize, String)>::new();

    for (line_start, line) in line_offsets(source) {
        let trimmed = line.trim_start();
        if !trimmed.starts_with("__all__") {
            continue;
        }
        let indent = line.len() - trimmed.len();
        let name_offset = line_start + indent;
        if !code_map.is_code_offset(name_offset) {
            continue;
        }
        let Some(eq_relative) = line.find('=') else {
            continue;
        };
        explicit_offsets.push(name_offset);
        let rhs_start = line_start + eq_relative + 1;
        let Some((value_offset, value)) = python_container_literal_after(source, rhs_start) else {
            continue;
        };
        entries.extend(string_literal_entries(value, value_offset));
    }

    for (call_start, call) in all_export_calls(source, code_map, "__all__.append(") {
        explicit_offsets.push(call_start);
        if let Some(argument_text) = lazy_import_argument_text(call) {
            if let Some(entry) = string_literal_entries(argument_text, call_start)
                .into_iter()
                .next()
            {
                entries.push(entry);
            }
        }
    }

    for (call_start, call) in all_export_calls(source, code_map, "__all__.extend(") {
        explicit_offsets.push(call_start);
        if let Some(argument_text) = lazy_import_argument_text(call) {
            entries.extend(string_literal_entries(argument_text, call_start));
        }
    }

    if explicit_offsets.is_empty() {
        return;
    }
    let marker_offset = explicit_offsets[0];
    push_all_export_symbol(symbols, module, path, code_map, marker_offset, None);
    let mut seen = BTreeSet::new();
    for (offset, name) in entries {
        if is_valid_identifier(&name) && seen.insert(name.clone()) {
            push_all_export_symbol(symbols, module, path, code_map, offset, Some(&name));
        }
    }
}

fn all_export_calls<'a>(
    source: &'a str,
    code_map: &CodeMap,
    pattern: &str,
) -> Vec<(usize, &'a str)> {
    let mut calls = Vec::new();
    for (start, _) in source.match_indices(pattern) {
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

fn python_container_literal_after(source: &str, start: usize) -> Option<(usize, &str)> {
    let offset = start
        + source[start..]
            .char_indices()
            .find_map(|(index, ch)| (!ch.is_whitespace()).then_some(index))?;
    let opener = source[offset..].chars().next()?;
    let closer = match opener {
        '[' => ']',
        '(' => ')',
        _ => {
            let line_end = source[offset..]
                .find('\n')
                .map(|index| offset + index)
                .unwrap_or(source.len());
            return Some((offset, &source[offset..line_end]));
        }
    };
    let mut depth = 0usize;
    let mut chars = source[offset..].char_indices().peekable();
    while let Some((relative, ch)) = chars.next() {
        if ch == '\'' || ch == '"' {
            skip_python_string(ch, &mut chars);
            continue;
        }
        if ch == opener {
            depth = depth.saturating_add(1);
        } else if ch == closer {
            depth = depth.checked_sub(1)?;
            if depth == 0 {
                let end = offset + relative + ch.len_utf8();
                return Some((offset, &source[offset..end]));
            }
        }
    }
    None
}

fn string_literal_entries(text: &str, base_offset: usize) -> Vec<(usize, String)> {
    let values = string_literal_args(text);
    let mut entries = Vec::new();
    let mut search_start = 0usize;
    for value in values {
        let relative = string_literal_position(&text[search_start..], &value)
            .map(|index| search_start + index)
            .or_else(|| string_literal_position(text, &value))
            .unwrap_or(0);
        search_start = relative.saturating_add(value.len());
        entries.push((base_offset + relative, value));
    }
    entries
}

fn push_all_export_symbol(
    symbols: &mut Vec<SymbolRecord>,
    module: &str,
    path: &Path,
    code_map: &CodeMap,
    offset: usize,
    name: Option<&str>,
) {
    let (line, character) = code_map.line_col(offset);
    let import_from = name
        .map(|name| format!("__all__::{name}"))
        .unwrap_or_else(|| SAGE_ALL_EXPORT_MARKER.to_string());
    symbols.push(SymbolRecord {
        name: SAGE_ALL_EXPORT_SENTINEL.to_string(),
        kind: SymbolKind::Import,
        module: module.to_string(),
        path: path.to_path_buf(),
        range: SourceRange {
            start_line: line,
            start_character: character,
            end_line: line,
            end_character: character + SAGE_ALL_EXPORT_SENTINEL.len() as u32,
        },
        detail: "Sage __all__ export".to_string(),
        docstring: None,
        import_from: Some(import_from),
        signature: None,
    });
}
