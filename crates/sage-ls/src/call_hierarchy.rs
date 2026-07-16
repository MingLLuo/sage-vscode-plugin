//! Call hierarchy source analysis and LSP item conversion.

use super::{
    editor_features::{position_leq, sage_folding_ranges},
    source_symbols::{
        is_call_hierarchy_symbol, module_name_for_path, symbol_body_range, symbol_kind,
    },
    text_positions::{
        byte_offset_to_utf16_character, is_word_byte, lsp_range_for_text, query_position_from_lsp,
        utf16_character_to_byte_offset, word_at_position,
    },
};
use sage_index::{parse_source, QueryDefinition, QueryResult, SymbolRecord, WorkspaceIndex};
use std::path::Path;
use tower_lsp::lsp_types::{
    CallHierarchyIncomingCall, CallHierarchyItem, CallHierarchyOutgoingCall, FoldingRange,
    Position, Range, SymbolKind, Url,
};

#[derive(Clone, Debug)]
pub(super) struct CallHierarchySourceContext {
    pub(super) text: String,
    pub(super) symbols: Vec<SymbolRecord>,
    pub(super) folds: Vec<FoldingRange>,
}

pub(super) fn call_hierarchy_item_for_local_definition(
    uri: &Url,
    path: &Path,
    text: &str,
    definition: &QueryDefinition,
) -> Option<CallHierarchyItem> {
    let parsed = parse_source(module_name_for_path(path), path, text);
    parsed
        .symbols
        .iter()
        .find(|symbol| {
            symbol.name == definition.name
                && symbol.range.start_line == definition.range.start_line
                && is_call_hierarchy_symbol(symbol)
        })
        .map(|symbol| call_hierarchy_item_for_symbol(uri, text, symbol))
}

pub(super) fn call_hierarchy_item_from_definition(
    definition: &QueryDefinition,
    uri: Url,
    range: Range,
) -> CallHierarchyItem {
    CallHierarchyItem {
        name: definition.name.clone(),
        kind: SymbolKind::FUNCTION,
        tags: None,
        detail: Some(definition.detail.clone()),
        uri,
        range,
        selection_range: range,
        data: None,
    }
}

#[cfg(test)]
pub(super) fn call_hierarchy_item_for_live_index_symbol(
    uri: &Url,
    path: &Path,
    text: &str,
    indexed_symbol: &SymbolRecord,
) -> Option<CallHierarchyItem> {
    let parsed = parse_source(module_name_for_path(path), path, text);
    parsed
        .symbols
        .iter()
        .filter(|symbol| {
            symbol.name == indexed_symbol.name
                && is_call_hierarchy_symbol(symbol)
                && (indexed_symbol.detail.is_empty() || symbol.detail == indexed_symbol.detail)
        })
        .min_by_key(|symbol| {
            symbol
                .range
                .start_line
                .abs_diff(indexed_symbol.range.start_line)
        })
        .map(|symbol| call_hierarchy_item_for_symbol(uri, text, symbol))
}

pub(super) fn enclosing_call_hierarchy_item(
    uri: &Url,
    path: &Path,
    text: &str,
    position: Position,
) -> Option<CallHierarchyItem> {
    let parsed = parse_source(module_name_for_path(path), path, text);
    let context = CallHierarchySourceContext {
        text: text.to_string(),
        symbols: parsed.symbols,
        folds: sage_folding_ranges(text),
    };
    enclosing_call_hierarchy_item_from_context(uri, &context, position)
}

pub(super) fn call_hierarchy_item_for_local_symbol_at_position(
    uri: &Url,
    path: &Path,
    text: &str,
    position: Position,
) -> Option<CallHierarchyItem> {
    let (word, range) = word_at_position(text, position)?;
    if !is_code_reference_range(text, &word, range) {
        return None;
    }
    let parsed = parse_source(module_name_for_path(path), path, text);
    let folds = sage_folding_ranges(text);
    parsed
        .symbols
        .iter()
        .filter(|symbol| {
            symbol.name == word && symbol.path == path && is_call_hierarchy_symbol(symbol)
        })
        .find(|symbol| lsp_range_for_text(text, &symbol.range) == range)
        .map(|symbol| call_hierarchy_item_for_symbol_with_folds(uri, text, &folds, symbol))
}

pub(super) fn enclosing_call_hierarchy_item_from_context(
    uri: &Url,
    context: &CallHierarchySourceContext,
    position: Position,
) -> Option<CallHierarchyItem> {
    context
        .symbols
        .iter()
        .filter(|symbol| is_call_hierarchy_symbol(symbol))
        .filter_map(|symbol| {
            let item = call_hierarchy_item_for_symbol_with_folds(
                uri,
                &context.text,
                &context.folds,
                symbol,
            );
            contains_position(&item.range, position).then_some(item)
        })
        .min_by_key(|item| {
            (
                item.range.end.line.saturating_sub(item.range.start.line),
                item.range
                    .end
                    .character
                    .saturating_sub(item.range.start.character),
            )
        })
}

pub(super) fn call_hierarchy_item_for_symbol(
    uri: &Url,
    text: &str,
    symbol: &SymbolRecord,
) -> CallHierarchyItem {
    let folds = sage_folding_ranges(text);
    call_hierarchy_item_for_symbol_with_folds(uri, text, &folds, symbol)
}

pub(super) fn call_hierarchy_item_for_symbol_with_folds(
    uri: &Url,
    text: &str,
    folds: &[FoldingRange],
    symbol: &SymbolRecord,
) -> CallHierarchyItem {
    let selection_range = lsp_range_for_text(text, &symbol.range);
    let range = symbol_body_range(text, folds, symbol).unwrap_or(selection_range);
    CallHierarchyItem {
        name: symbol.name.clone(),
        kind: symbol_kind(&symbol.kind),
        tags: None,
        detail: symbol_detail(symbol),
        uri: uri.clone(),
        range,
        selection_range,
        data: None,
    }
}

pub(super) fn push_incoming_call(
    calls: &mut Vec<CallHierarchyIncomingCall>,
    from: CallHierarchyItem,
    from_range: Range,
) {
    if let Some(existing) = calls
        .iter_mut()
        .find(|call| same_call_hierarchy_item(&call.from, &from))
    {
        existing.from_ranges.push(from_range);
        return;
    }
    calls.push(CallHierarchyIncomingCall {
        from,
        from_ranges: vec![from_range],
    });
}

pub(super) fn push_outgoing_call(
    calls: &mut Vec<CallHierarchyOutgoingCall>,
    to: CallHierarchyItem,
    from_range: Range,
) {
    if let Some(existing) = calls
        .iter_mut()
        .find(|call| same_call_hierarchy_item(&call.to, &to))
    {
        existing.from_ranges.push(from_range);
        return;
    }
    calls.push(CallHierarchyOutgoingCall {
        to,
        from_ranges: vec![from_range],
    });
}

fn same_call_hierarchy_item(left: &CallHierarchyItem, right: &CallHierarchyItem) -> bool {
    left.name == right.name
        && left.uri == right.uri
        && left.selection_range == right.selection_range
}

pub(super) fn call_ranges_in_range(text: &str, range: Range) -> Vec<(String, Range)> {
    let lines: Vec<_> = text.lines().collect();
    if lines.is_empty() {
        return Vec::new();
    }
    let mut calls = Vec::new();
    let start_line = range.start.line.min(lines.len().saturating_sub(1) as u32) as usize;
    let end_line = range.end.line.min(lines.len().saturating_sub(1) as u32) as usize;
    for line_number in start_line..=end_line {
        let Some(line) = lines.get(line_number) else {
            continue;
        };
        let scan_start = if line_number == start_line {
            utf16_character_to_byte_offset(line, range.start.character).unwrap_or_default()
        } else {
            0
        }
        .min(line.len());
        let scan_end = if line_number == end_line {
            utf16_character_to_byte_offset(line, range.end.character).unwrap_or(line.len())
        } else {
            line.len()
        }
        .min(line.len());
        calls.extend(call_ranges_in_line(
            line,
            line_number as u32,
            scan_start,
            scan_end,
        ));
    }
    calls
}

pub(super) struct ResolvedOutgoingCall {
    pub(super) definition: QueryDefinition,
    pub(super) from_range: Range,
}

pub(super) fn high_confidence_call_hierarchy_definition(
    query: &QueryResult,
) -> Option<&QueryDefinition> {
    (query.resolution_confidence.as_deref() == Some("high"))
        .then_some(query.definition.as_ref())
        .flatten()
}

pub(super) fn resolve_outgoing_calls(
    index: &WorkspaceIndex,
    path: &Path,
    text: &str,
    range: Range,
    caller_name: &str,
) -> Vec<ResolvedOutgoingCall> {
    call_ranges_in_range(text, range)
        .into_iter()
        .filter(|(name, _)| name != caller_name)
        .filter_map(|(_, from_range)| {
            let position = query_position_from_lsp(text, from_range.start)?;
            let query = index.query_source_at_navigation(path, text, position);
            high_confidence_call_hierarchy_definition(&query)
                .cloned()
                .map(|definition| ResolvedOutgoingCall {
                    definition,
                    from_range,
                })
        })
        .collect()
}

fn call_ranges_in_line(
    line: &str,
    line_number: u32,
    scan_start: usize,
    scan_end: usize,
) -> Vec<(String, Range)> {
    let bytes = line.as_bytes();
    let mut calls = Vec::new();
    let mut quote: Option<u8> = None;
    let mut escaped = false;
    let mut index = 0;
    while index < scan_end {
        let byte = bytes[index];
        if escaped {
            escaped = false;
            index += 1;
            continue;
        }
        if byte == b'\\' {
            escaped = true;
            index += 1;
            continue;
        }
        if let Some(active) = quote {
            if byte == active {
                quote = None;
            }
            index += 1;
            continue;
        }
        if byte == b'#' {
            break;
        }
        if byte == b'\'' || byte == b'"' {
            quote = Some(byte);
            index += 1;
            continue;
        }
        if index >= scan_start
            && is_identifier_start(byte)
            && (index == 0 || !is_word_byte(bytes[index - 1]))
        {
            let start = index;
            let mut end = index + 1;
            while end < scan_end && is_word_byte(bytes[end]) {
                end += 1;
            }
            let mut cursor = end;
            while cursor < scan_end && bytes[cursor].is_ascii_whitespace() {
                cursor += 1;
            }
            let name = &line[start..end];
            if cursor < scan_end
                && bytes[cursor] == b'('
                && !is_call_hierarchy_keyword(name)
                && !is_declaration_identifier(line, start)
            {
                calls.push((
                    name.to_string(),
                    Range::new(
                        Position::new(
                            line_number,
                            byte_offset_to_utf16_character(line, start).unwrap_or(start as u32),
                        ),
                        Position::new(
                            line_number,
                            byte_offset_to_utf16_character(line, end).unwrap_or(end as u32),
                        ),
                    ),
                ));
            }
            index = end;
            continue;
        }
        index += 1;
    }
    calls
}

pub(super) fn is_identifier_start(byte: u8) -> bool {
    byte == b'_' || byte.is_ascii_alphabetic()
}

fn is_declaration_identifier(line: &str, start: usize) -> bool {
    line.get(..start)
        .unwrap_or_default()
        .split_whitespace()
        .last()
        .is_some_and(|token| matches!(token, "def" | "class" | "cdef" | "cpdef"))
}

fn is_call_hierarchy_keyword(name: &str) -> bool {
    matches!(
        name,
        "if" | "elif"
            | "for"
            | "while"
            | "with"
            | "except"
            | "return"
            | "yield"
            | "assert"
            | "lambda"
            | "def"
            | "class"
            | "cdef"
            | "cpdef"
    )
}

fn contains_position(range: &Range, position: Position) -> bool {
    position_leq(range.start, position) && position_leq(position, range.end)
}

fn is_code_reference_range(text: &str, word: &str, target_range: Range) -> bool {
    let Some(start) = query_position_from_lsp(text, target_range.start) else {
        return false;
    };
    let Some(end) = query_position_from_lsp(text, target_range.end) else {
        return false;
    };
    let range = sage_index::SourceRange {
        start_line: start.line,
        start_character: start.character,
        end_line: end.line,
        end_character: end.character,
    };
    sage_index::is_code_reference_at_range(text, word, &range)
}

fn symbol_detail(symbol: &SymbolRecord) -> Option<String> {
    symbol
        .signature
        .clone()
        .or_else(|| (!symbol.detail.is_empty()).then(|| symbol.detail.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sage_index::IndexOptions;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn indexed_call_target_is_reparsed_from_live_alias_text() {
        let indexed_path = PathBuf::from("/workspace/physical.py");
        let alias_path = PathBuf::from("/workspace/alias.py");
        let indexed = parse_source("physical", &indexed_path, "def target():\n    return 1\n")
            .symbols
            .into_iter()
            .find(|symbol| symbol.name == "target")
            .unwrap();
        let uri = Url::from_file_path(&alias_path).unwrap();
        let live_text = "π = 3\n\ndef target(value='🚀'):\n    return value\n";

        let item =
            call_hierarchy_item_for_live_index_symbol(&uri, &alias_path, live_text, &indexed)
                .unwrap();

        assert_eq!(item.uri, uri);
        assert_eq!(item.selection_range.start, Position::new(2, 4));
        assert_eq!(item.range.end, Position::new(3, 16));
    }

    #[test]
    fn outgoing_calls_follow_explicit_imports_instead_of_global_homonyms() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("sage-ls-outgoing-import-{nonce}"));
        fs::create_dir_all(&root).unwrap();
        let intended = root.join("intended.py");
        let decoy = root.join("aaa_decoy.py");
        let consumer = root.join("consumer.py");
        fs::write(&intended, "def target():\n    return 'intended'\n").unwrap();
        fs::write(&decoy, "def target():\n    return 'decoy'\n").unwrap();
        let source = "from intended import target\n\ndef caller():\n    return target()\n";
        fs::write(&consumer, source).unwrap();

        let mut index = WorkspaceIndex::new(IndexOptions {
            roots: vec![root.clone()],
            editable_roots: vec![root.clone()],
            exclude_globs: Vec::new(),
            cache_dir: root.join(".cache"),
            enable_pyx: true,
        });
        index.rebuild().unwrap();
        let calls = resolve_outgoing_calls(
            &index,
            &consumer,
            source,
            Range::new(Position::new(2, 0), Position::new(3, 19)),
            "caller",
        );

        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].definition.path, intended.canonicalize().unwrap());
        assert_eq!(calls[0].definition.name, "target");
        assert_eq!(calls[0].from_range.start, Position::new(3, 11));
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn call_hierarchy_definitions_require_high_confidence() {
        let definition = QueryDefinition {
            name: "target".to_string(),
            path: PathBuf::from("/workspace/target.py"),
            range: sage_index::SourceRange::default(),
            detail: "Function target".to_string(),
            module: "target".to_string(),
        };
        let mut query = QueryResult {
            definition: Some(definition.clone()),
            resolution_confidence: Some("medium".to_string()),
            ..QueryResult::default()
        };
        assert!(high_confidence_call_hierarchy_definition(&query).is_none());

        query.resolution_confidence = Some("ambiguous".to_string());
        assert!(high_confidence_call_hierarchy_definition(&query).is_none());

        query.resolution_confidence = Some("high".to_string());
        assert_eq!(
            high_confidence_call_hierarchy_definition(&query).map(|value| &value.name),
            Some(&definition.name)
        );
    }

    #[test]
    fn outgoing_calls_ignore_unique_but_unbound_global_matches() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("sage-ls-outgoing-unbound-{nonce}"));
        fs::create_dir_all(&root).unwrap();
        let target = root.join("provider.py");
        let consumer = root.join("consumer.py");
        fs::write(&target, "def external_target():\n    return 1\n").unwrap();
        let source = "def caller():\n    return external_target()\n";
        fs::write(&consumer, source).unwrap();

        let mut index = WorkspaceIndex::new(IndexOptions {
            roots: vec![root.clone()],
            editable_roots: vec![root.clone()],
            exclude_globs: Vec::new(),
            cache_dir: root.join(".cache"),
            enable_pyx: true,
        });
        index.rebuild().unwrap();
        let query = index.query_source_at_navigation(
            &consumer,
            source,
            sage_index::QueryPosition {
                line: 1,
                character: 11,
            },
        );
        assert_eq!(query.resolution_confidence.as_deref(), Some("medium"));
        assert!(query.definition.is_some());

        let calls = resolve_outgoing_calls(
            &index,
            &consumer,
            source,
            Range::new(Position::new(0, 0), Position::new(1, 28)),
            "caller",
        );
        assert!(calls.is_empty());
        fs::remove_dir_all(root).ok();
    }
}
