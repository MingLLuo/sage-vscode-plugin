use super::*;

fn parse_with_tree_sitter(source: &str) -> Option<tree_sitter::Tree> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_python::LANGUAGE.into())
        .ok()?;
    parser.parse(source, None)
}

const MAX_DIAGNOSTICS_PER_KIND: usize = 100;

pub(super) fn diagnostics_for_source(path: &Path, source: &str) -> Vec<DiagnosticRecord> {
    let normalized = source.replace("\r\n", "\n").replace('\r', "\n");
    let source = normalized.as_str();
    let is_sage = path.extension().is_some_and(|ext| ext == "sage");
    // Warnings are capped separately so they can never crowd out syntax errors.
    let mut warnings = if path.extension().is_some_and(|ext| ext == "py")
        && source_looks_sage_heavy_python(source)
        && !is_sage_library_module(path)
    {
        sage_python_caret_exponent_diagnostics(source)
    } else {
        Vec::new()
    };
    warnings.truncate(MAX_DIAGNOSTICS_PER_KIND);
    if path
        .extension()
        .is_some_and(|ext| matches!(ext.to_str(), Some("pyx" | "pxd" | "pxi" | "spyx")))
    {
        return warnings;
    }
    let mut diagnostics = Vec::new();
    // Normalizing line endings preserves line/byte-column coordinates, including
    // lone CR lines, while giving the Python grammar its expected LF input.
    let (generated, mapping) = if is_sage {
        let (result, mapping) = super::preprocessing::preprocess_with_mapping(&normalized);
        (result.generated, mapping)
    } else {
        (normalized.clone(), Vec::new())
    };
    let original_lines: Vec<&str> = normalized.split('\n').collect();
    let carets = if is_sage {
        sage_trailing_caret_errors(&normalized)
    } else {
        Vec::new()
    };
    let Some(tree) = parse_with_tree_sitter(&generated) else {
        return warnings;
    };
    // Walk only erroneous branches. Prefer the smallest ERROR nodes; cap reports
    // so an unfinished large paste cannot flood the editor with diagnostics.
    let mut pending = vec![tree.root_node()];
    while let Some(node) = pending.pop() {
        if diagnostics.len() >= MAX_DIAGNOSTICS_PER_KIND {
            break;
        }
        let mut cursor = node.walk();
        let children: Vec<_> = node
            .children(&mut cursor)
            .filter(|child| child.has_error() || child.is_missing())
            .collect();
        if children.is_empty() && (node.is_error() || node.is_missing()) {
            let start = map_position(node.start_position(), &mapping, &original_lines, false);
            let mut end = map_position(node.end_position(), &mapping, &original_lines, true);
            if end.0 != start.0 {
                // An unclosed bracket can make one ERROR node span the rest of the file.
                // Underline only where the problem starts.
                let line_len = original_lines.get(start.0 as usize).map_or(0, |line| line.len());
                end = (start.0, (line_len as u32).max(start.1 + 1));
            }
            let range = SourceRange {
                start_line: start.0,
                start_character: start.1,
                end_line: end.0,
                end_character: end.1,
            };
            // Only specialize an actual parser error. A caret at the end of a
            // continued expression is legal Sage and must not be diagnosed.
            let matching_carets: Vec<_> = carets
                .iter()
                .filter(|caret| {
                    start.0 == end.0
                        && (start.0, start.1) <= (caret.start_line, caret.start_character)
                        && end >= (caret.end_line, caret.end_character)
                })
                .collect();
            if !matching_carets.is_empty() {
                diagnostics.extend(matching_carets.into_iter().map(|caret| {
                    syntax_diagnostic(
                        caret.clone(),
                        "Syntax error: incomplete Sage exponentiation".to_string(),
                    )
                }));
                continue;
            }
            diagnostics.push(syntax_diagnostic(
                range,
                if node.is_missing() {
                    format!("Syntax error: expected `{}`", node.kind())
                } else {
                    "Syntax error: unexpected syntax".to_string()
                },
            ));
        } else {
            pending.extend(children.into_iter().rev());
        }
    }
    diagnostics.truncate(MAX_DIAGNOSTICS_PER_KIND);
    diagnostics.extend(warnings);
    diagnostics.sort_by_key(|diagnostic| {
        (
            diagnostic.range.start_line,
            diagnostic.range.start_character,
        )
    });
    diagnostics.dedup_by(|a, b| a.range == b.range && a.code == b.code);
    diagnostics
}

fn syntax_diagnostic(range: SourceRange, message: String) -> DiagnosticRecord {
    DiagnosticRecord {
        message,
        range,
        code: "syntax-error".to_string(),
        severity: "error".to_string(),
    }
}

fn map_position(
    point: tree_sitter::Point,
    edits: &[PreprocessEdit],
    lines: &[&str],
    end: bool,
) -> (u32, u32) {
    let line = point.row.min(lines.len().saturating_sub(1));
    let column = point.column;
    let mut mapped = column;
    // Edits use absolute coordinates in both sources, so the last preceding edit
    // supplies the total shift without accumulating earlier replacements twice.
    let mut preceding: Vec<_> = edits
        .iter()
        .filter(|edit| edit.line as usize == line)
        .collect();
    preceding.sort_by_key(|edit| edit.generated_character);
    for edit in preceding {
        let generated = edit.generated_character as usize;
        if generated > column {
            break;
        }
        let source = edit.source_character as usize;
        let generated_end = generated + edit.generated_text.len();
        mapped = if column >= generated_end {
            source + edit.source_text.len() + column - generated_end
        } else if column == generated || !end {
            source
        } else {
            source + edit.source_text.len()
        };
    }
    let text = lines[line];
    mapped = mapped.min(text.len());
    while !text.is_char_boundary(mapped) {
        mapped -= 1;
    }
    (line as u32, mapped as u32)
}

fn sage_trailing_caret_errors(source: &str) -> Vec<SourceRange> {
    let code_map = CodeMap::new(source);
    let mut offset = 0;
    let mut ranges = Vec::new();
    for (line_index, line) in source.split('\n').enumerate() {
        let trimmed = line.trim_end();
        if trimmed.ends_with('^') && code_map.is_code_offset(offset + trimmed.len() - 1) {
            let character = (trimmed.len() - 1) as u32;
            ranges.push(SourceRange {
                start_line: line_index as u32,
                start_character: character,
                end_line: line_index as u32,
                end_character: character + 1,
            });
        }
        offset += line.len() + 1;
    }
    ranges
}

/// The XOR warning targets user scripts that mistake `^` for exponentiation. Sage's own
/// library is plain Python whose authors use `^` deliberately (hashes, bit vectors, booleans),
/// so modules inside a `sage` package that has an `all.py` are exempt.
fn is_sage_library_module(path: &Path) -> bool {
    path.ancestors()
        .skip(1)
        .any(|ancestor| ancestor.file_name().is_some_and(|name| name == "sage") && ancestor.join("all.py").is_file())
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
            if !crate::identifiers::has_boundaries(source, start, end)
                || !code_map.is_code_offset(start)
            {
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

/// Conservative source-local fallback for names bound inside a function. It keeps
/// occurrences in sibling functions separate even when navigation cannot resolve
/// a local variable to a high-confidence indexed symbol.
pub fn local_document_highlight_ranges(
    source: &str,
    name: &str,
    target_line: u32,
    target_character: u32,
) -> Option<Vec<SourceRange>> {
    let scopes = LexicalScopeMap::new(source);
    let owner = scopes.local_binding_function_line(source, target_line, target_character, name)?;
    Some(
        references_in_source(Path::new("document.py"), source, name)
            .into_iter()
            .filter(|reference| {
                scopes.local_binding_function_line(
                    source,
                    reference.range.start_line,
                    reference.range.start_character,
                    name,
                ) == Some(owner)
            })
            .map(|reference| reference.range)
            .collect(),
    )
}

pub(super) fn reference_spans_in_source(
    path: &Path,
    source: &str,
) -> Vec<(String, ReferenceRecord)> {
    let mut records = Vec::new();
    let code_map = CodeMap::new(source);
    for (start, name) in crate::identifiers::spans(source) {
        if !code_map.is_code_offset(start) {
            continue;
        }
        let (line, character) = code_map.line_col(start);
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
        if self.source.get(start..end) != Some(name) {
            return false;
        }
        if !crate::identifiers::has_boundaries(self.source, start, end) {
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

#[cfg(test)]
mod position_tests {
    use super::*;

    #[test]
    fn rewritten_positions_return_original_byte_columns() {
        for (source, generated_fragment, original_fragment) in [
            ("结果 = 2^3 + bad", "bad", "bad"),
            ("R.<x> = QQ[] # note", "QQ[", "QQ["),
            ("R.<x> = QQ[] # note", "# note", "# note"),
            ("R.<x> = GF(\n  2^3 + bad\n) # note", "bad", "bad"),
            ("R.<x> = GF(\n  2^3 + bad\n) # note", "# note", "# note"),
            ("values = [1..5] + bad", "bad", "bad"),
        ] {
            let (result, edits) = super::super::preprocessing::preprocess_with_mapping(source);
            let offset = result.generated.find(generated_fragment).unwrap();
            let prefix = &result.generated[..offset];
            let row = prefix.bytes().filter(|b| *b == b'\n').count();
            let column = prefix.rfind('\n').map_or(offset, |last| offset - last - 1);
            let lines: Vec<_> = source.split('\n').collect();
            let expected = lines[row].find(original_fragment).unwrap();
            assert_eq!(
                map_position(tree_sitter::Point { row, column }, &edits, &lines, false),
                (row as u32, expected as u32),
                "{source} => {}",
                result.generated
            );
        }
    }
}
