use super::import_parsing::sage_export_import_from;
use super::*;

pub(super) fn push_import_symbol(
    symbols: &mut Vec<SymbolRecord>,
    module: &str,
    path: &Path,
    name: &str,
    code_map: &CodeMap,
    offset: usize,
    import_from: &str,
) {
    let import_from = normalize_import_from(import_from, module, name);
    let import_from =
        sage_export_import_from(&import_from, name).unwrap_or_else(|| import_from.to_string());
    let (line, character) = code_map.line_col(offset);
    symbols.push(SymbolRecord {
        name: name.to_string(),
        kind: SymbolKind::Import,
        module: module.to_string(),
        path: path.to_path_buf(),
        range: SourceRange {
            start_line: line,
            start_character: character,
            end_line: line,
            end_character: character + name.len() as u32,
        },
        detail: format!("Import {name} from {import_from}"),
        docstring: None,
        import_from: Some(import_from),
        signature: None,
    });
}

pub(super) fn line_offsets(source: &str) -> Vec<(usize, &str)> {
    let mut result = Vec::new();
    let mut offset = 0usize;
    for line in source.lines() {
        result.push((offset, line));
        offset += line.len() + 1;
    }
    result
}

pub(super) fn push_simple_symbol(
    symbols: &mut Vec<SymbolRecord>,
    module: &str,
    path: &Path,
    name: &str,
    kind: SymbolKind,
    code_map: &CodeMap,
    offset: usize,
) {
    let detail = format!("{:?} {}", kind, name);
    let context = SymbolPushContext {
        module,
        path,
        code_map,
    };
    push_symbol_with_detail(symbols, &context, name, kind, offset, detail);
}

pub(super) struct SymbolPushContext<'a> {
    pub(super) module: &'a str,
    pub(super) path: &'a Path,
    pub(super) code_map: &'a CodeMap,
}

pub(super) fn push_symbol_with_detail(
    symbols: &mut Vec<SymbolRecord>,
    context: &SymbolPushContext<'_>,
    name: &str,
    kind: SymbolKind,
    offset: usize,
    detail: String,
) {
    let (line, character) = context.code_map.line_col(offset);
    symbols.push(SymbolRecord {
        name: name.to_string(),
        kind,
        module: context.module.to_string(),
        path: context.path.to_path_buf(),
        range: SourceRange {
            start_line: line,
            start_character: character,
            end_line: line,
            end_character: character + name.len() as u32,
        },
        detail,
        docstring: None,
        import_from: None,
        signature: None,
    });
}

pub(super) fn function_signature(source: &str, offset: usize, name: &str) -> Option<String> {
    let line_start = source[..offset].rfind('\n').map_or(0, |index| index + 1);
    let line_end = source[offset..]
        .find('\n')
        .map_or(source.len(), |index| offset + index);
    let header_end = definition_header_end(source, offset).unwrap_or(line_end);
    let header = source[line_start..header_end]
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let name_offset = header.find(name)?;
    let rest = &header[name_offset + name.len()..];
    let open = rest.find('(')?;
    let close = rest.rfind(')')?;
    if close < open {
        return None;
    }
    Some(format!("{}{}", name, &rest[open..=close]))
}
