use super::*;

fn parse_with_tree_sitter(source: &str) -> Option<tree_sitter::Tree> {
    let mut parser = Parser::new();
    parser.set_language(tree_sitter_python::language()).ok()?;
    parser.parse(source, None)
}

pub(super) fn diagnostics_for_source(path: &Path, source: &str) -> Vec<DiagnosticRecord> {
    if let Some(caret) = sage_trailing_caret_error(source) {
        return vec![DiagnosticRecord {
            message: "Syntax error: incomplete Sage exponentiation".to_string(),
            range: caret,
            code: "syntax-error".to_string(),
            severity: "error".to_string(),
        }];
    }
    let mut diagnostics = if path.extension().is_some_and(|ext| ext == "py")
        && source_looks_sage_heavy_python(source)
    {
        sage_python_caret_exponent_diagnostics(source)
    } else {
        Vec::new()
    };
    if path
        .extension()
        .is_some_and(|ext| ext == "pyx" || ext == "pxd" || ext == "pxi")
    {
        return diagnostics;
    }
    let generated = if path.extension().is_some_and(|ext| ext == "sage") {
        preprocess_sage_source(source).generated
    } else {
        source.to_string()
    };
    let Some(tree) = parse_with_tree_sitter(&generated) else {
        return diagnostics;
    };
    if tree.root_node().has_error() {
        diagnostics.push(DiagnosticRecord {
            message: "Syntax error: source could not be parsed".to_string(),
            range: SourceRange {
                start_line: 0,
                start_character: 0,
                end_line: 0,
                end_character: 1,
            },
            code: "syntax-error".to_string(),
            severity: "error".to_string(),
        });
    }
    diagnostics
}

fn sage_trailing_caret_error(source: &str) -> Option<SourceRange> {
    for (line_index, line) in source.lines().enumerate() {
        let trimmed = line.trim_end();
        if trimmed.ends_with('^') {
            let character = line.rfind('^')? as u32;
            return Some(SourceRange {
                start_line: line_index as u32,
                start_character: character,
                end_line: line_index as u32,
                end_character: character + 1,
            });
        }
    }
    None
}

fn source_looks_sage_heavy_python(source: &str) -> bool {
    source.contains("from sage.all import")
        || source.contains("import sage.all")
        || source.contains("from sage.")
        || source.contains("from sage_")
}

fn sage_python_caret_exponent_diagnostics(source: &str) -> Vec<DiagnosticRecord> {
    let code_map = CodeMap::new(source);
    let bytes = source.as_bytes();
    let mut diagnostics = Vec::new();
    for (offset, byte) in bytes.iter().enumerate() {
        if *byte != b'^'
            || !code_map.is_code_offset(offset)
            || !looks_like_binary_caret_expression(bytes, &code_map, offset)
        {
            continue;
        }
        let (line, character) = code_map.line_col(offset);
        diagnostics.push(DiagnosticRecord {
            message:
                "Sage-style exponent operator `^` has Python XOR semantics in `.py`; use `**`."
                    .to_string(),
            range: SourceRange {
                start_line: line,
                start_character: character,
                end_line: line,
                end_character: character + 1,
            },
            code: "sage-python-caret-exponent".to_string(),
            severity: "warning".to_string(),
        });
    }
    diagnostics
}

fn looks_like_binary_caret_expression(bytes: &[u8], code_map: &CodeMap, offset: usize) -> bool {
    let Some(left) = nearest_code_byte_before(bytes, code_map, offset) else {
        return false;
    };
    let Some(right) = nearest_code_byte_after(bytes, code_map, offset + 1) else {
        return false;
    };
    is_caret_operand_end(left) && is_caret_operand_start(right)
}

fn nearest_code_byte_before(bytes: &[u8], code_map: &CodeMap, offset: usize) -> Option<u8> {
    let mut index = offset;
    while index > 0 {
        index -= 1;
        if bytes[index].is_ascii_whitespace() || !code_map.is_code_offset(index) {
            continue;
        }
        return Some(bytes[index]);
    }
    None
}

fn nearest_code_byte_after(bytes: &[u8], code_map: &CodeMap, offset: usize) -> Option<u8> {
    let mut index = offset;
    while index < bytes.len() {
        if bytes[index].is_ascii_whitespace() || !code_map.is_code_offset(index) {
            index += 1;
            continue;
        }
        return Some(bytes[index]);
    }
    None
}

fn is_caret_operand_end(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b')' | b']')
}

fn is_caret_operand_start(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'(' | b'[' | b'+' | b'-')
}

pub fn references_in_source(path: &Path, source: &str, name: &str) -> Vec<ReferenceRecord> {
    if name.is_empty() || !source.contains(name) {
        return Vec::new();
    }
    let code_map = CodeMap::new(source);
    source
        .match_indices(name)
        .filter_map(|(start, _)| {
            let end = start + name.len();
            let starts_at_boundary = start == 0 || !is_word_byte(source.as_bytes()[start - 1]);
            let ends_at_boundary = end == source.len() || !is_word_byte(source.as_bytes()[end]);
            if !starts_at_boundary || !ends_at_boundary || !code_map.is_code_offset(start) {
                return None;
            }
            let (line, character) = code_map.line_col(start);
            Some(ReferenceRecord {
                path: path.to_path_buf(),
                range: SourceRange {
                    start_line: line,
                    start_character: character,
                    end_line: line,
                    end_character: character + name.len() as u32,
                },
            })
        })
        .collect()
}

pub(super) fn reference_spans_in_source(
    path: &Path,
    source: &str,
) -> Vec<(String, ReferenceRecord)> {
    let mut records = Vec::new();
    let code_map = CodeMap::new(source);
    for captures in word_re().captures_iter(source) {
        let Some(candidate) = captures.name("name") else {
            continue;
        };
        if !code_map.is_code_offset(candidate.start()) {
            continue;
        }
        let name = candidate.as_str();
        let (line, character) = code_map.line_col(candidate.start());
        records.push((
            name.to_string(),
            ReferenceRecord {
                path: path.to_path_buf(),
                range: SourceRange {
                    start_line: line,
                    start_character: character,
                    end_line: line,
                    end_character: character + name.len() as u32,
                },
            },
        ));
    }
    records
}

pub struct CodeReferenceMap<'source> {
    source: &'source str,
    code_map: CodeMap,
}

impl<'source> CodeReferenceMap<'source> {
    pub fn new(source: &'source str) -> Self {
        Self {
            source,
            code_map: CodeMap::new(source),
        }
    }

    pub fn contains(&self, name: &str, range: &SourceRange) -> bool {
        if name.is_empty() {
            return false;
        }
        let Some(start) = self
            .code_map
            .offset(range.start_line, range.start_character)
        else {
            return false;
        };
        let Some(end) = self.code_map.offset(range.end_line, range.end_character) else {
            return false;
        };
        if start >= end || !self.code_map.is_code_offset(start) {
            return false;
        }
        let bytes = self.source.as_bytes();
        if bytes.get(start..end) != Some(name.as_bytes()) {
            return false;
        }
        if start > 0 && is_word_byte(bytes[start - 1]) {
            return false;
        }
        if bytes.get(end).is_some_and(|byte| is_word_byte(*byte)) {
            return false;
        }
        true
    }
}

pub fn is_code_reference_at_range(source: &str, name: &str, range: &SourceRange) -> bool {
    CodeReferenceMap::new(source).contains(name, range)
}

pub(super) fn dedupe_reference_records(references: Vec<ReferenceRecord>) -> Vec<ReferenceRecord> {
    let mut seen = BTreeSet::new();
    let mut deduped = Vec::new();
    for reference in references {
        let key = (
            reference.path.clone(),
            reference.range.start_line,
            reference.range.start_character,
            reference.range.end_line,
            reference.range.end_character,
        );
        if seen.insert(key) {
            deduped.push(reference);
        }
    }
    deduped
}

pub(super) fn scope_references_for_resolved_symbol(
    references: Vec<ReferenceRecord>,
    resolved: Option<&SymbolRecord>,
    query_path: &Path,
) -> Vec<ReferenceRecord> {
    let Some(resolved) = resolved else {
        return references;
    };
    if !matches!(
        resolved.kind,
        SymbolKind::Variable | SymbolKind::PreparserGenerator
    ) || resolved.path != query_path
    {
        return references;
    }
    references
        .into_iter()
        .filter(|reference| reference.path == resolved.path)
        .collect()
}
