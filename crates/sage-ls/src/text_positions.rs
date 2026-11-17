//! LSP/`sage-index` text coordinate conversions.
//!
//! `sage-index` stores columns as UTF-8 byte offsets while LSP uses UTF-16 code units.
//! Keeping the conversion here makes the protocol boundary explicit and testable.

use sage_index::QueryPosition;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};
use tower_lsp_server::ls_types::{Position, Range, TextDocumentContentChangeEvent};

/// Byte bounds of every LSP line, excluding LF, CRLF, and standalone CR.
#[derive(Clone, Debug)]
pub(super) struct LineIndex(Vec<(usize, usize)>);

impl LineIndex {
    pub(super) fn new(text: &str) -> Self {
        let bytes = text.as_bytes();
        let mut lines = Vec::new();
        let (mut start, mut cursor) = (0, 0);
        while cursor < bytes.len() {
            if matches!(bytes[cursor], b'\r' | b'\n') {
                lines.push((start, cursor));
                if bytes[cursor] == b'\r' && bytes.get(cursor + 1) == Some(&b'\n') {
                    cursor += 1;
                }
                start = cursor + 1;
            }
            cursor += 1;
        }
        lines.push((start, bytes.len()));
        Self(lines)
    }

    pub(super) fn bounds(&self, line: u32) -> Option<(usize, usize)> {
        self.0.get(line as usize).copied()
    }

    pub(super) fn position(&self, text: &str, line: u32, column: u32) -> Position {
        let character = self
            .bounds(line)
            .and_then(|(start, end)| {
                byte_offset_to_utf16_character(&text[start..end], column as usize)
            })
            .unwrap_or(column);
        Position::new(line, character)
    }

    pub(super) fn range(&self, text: &str, range: &sage_index::SourceRange) -> Range {
        Range::new(
            self.position(text, range.start_line, range.start_character),
            self.position(text, range.end_line, range.end_character),
        )
    }
}

pub(super) fn lsp_range_for_text(text: &str, range: &sage_index::SourceRange) -> Range {
    Range {
        start: lsp_position_for_byte_column(text, range.start_line, range.start_character),
        end: lsp_position_for_byte_column(text, range.end_line, range.end_character),
    }
}

pub(super) fn lsp_range_for_path_cached(
    source_text_by_path: &mut HashMap<PathBuf, Option<String>>,
    path: &Path,
    range: &sage_index::SourceRange,
) -> Range {
    source_text_by_path
        .entry(path.to_path_buf())
        .or_insert_with(|| std::fs::read_to_string(path).ok())
        .as_deref()
        .map(|text| lsp_range_for_text(text, range))
        .unwrap_or_else(|| raw_lsp_range(range))
}

fn raw_lsp_range(range: &sage_index::SourceRange) -> Range {
    Range::new(
        Position::new(range.start_line, range.start_character),
        Position::new(range.end_line, range.end_character),
    )
}

pub(super) fn lsp_position_for_byte_column(text: &str, line: u32, byte_column: u32) -> Position {
    let character = line_byte_bounds(text, line)
        .and_then(|(start, end)| {
            byte_offset_to_utf16_character(&text[start..end], byte_column as usize)
        })
        .unwrap_or(byte_column);
    Position::new(line, character)
}

pub(super) fn query_position_from_lsp(text: &str, position: Position) -> Option<QueryPosition> {
    let (line_start, line_end) = line_byte_bounds(text, position.line)?;
    let line = &text[line_start..line_end];
    let byte_column = utf16_character_to_byte_offset(line, position.character)?;
    Some(QueryPosition {
        line: position.line,
        character: byte_column as u32,
    })
}

pub(super) fn apply_text_document_change(
    text: &mut String,
    change: &TextDocumentContentChangeEvent,
) -> std::result::Result<(), String> {
    let Some(range) = change.range else {
        *text = change.text.clone();
        return Ok(());
    };
    let start = position_to_byte_index(text, range.start)
        .ok_or_else(|| format!("invalid start position {:?}", range.start))?;
    let end = position_to_byte_index(text, range.end)
        .ok_or_else(|| format!("invalid end position {:?}", range.end))?;
    if start > end {
        return Err(format!("range start {start} is after end {end}"));
    }
    text.replace_range(start..end, &change.text);
    Ok(())
}

fn position_to_byte_index(text: &str, position: Position) -> Option<usize> {
    let (line_start, line_end) = line_byte_bounds(text, position.line)?;
    let line = &text[line_start..line_end];
    utf16_character_to_byte_offset(line, position.character).map(|offset| line_start + offset)
}

pub(super) fn line_byte_bounds(text: &str, target_line: u32) -> Option<(usize, usize)> {
    let bytes = text.as_bytes();
    let (mut line, mut start, mut cursor) = (0, 0, 0);
    while cursor < bytes.len() {
        if matches!(bytes[cursor], b'\r' | b'\n') {
            if line == target_line {
                return Some((start, cursor));
            }
            if bytes[cursor] == b'\r' && bytes.get(cursor + 1) == Some(&b'\n') {
                cursor += 1;
            }
            start = cursor + 1;
            line += 1;
        }
        cursor += 1;
    }
    (line == target_line).then_some((start, bytes.len()))
}

pub(super) fn utf16_character_to_byte_offset(line: &str, character: u32) -> Option<usize> {
    let mut utf16_offset = 0u32;
    for (byte_offset, ch) in line.char_indices() {
        if utf16_offset == character {
            return Some(byte_offset);
        }
        let next = utf16_offset.saturating_add(ch.len_utf16() as u32);
        if character < next {
            return None;
        }
        utf16_offset = next;
    }
    if character >= utf16_offset {
        Some(line.len())
    } else {
        None
    }
}

pub(super) fn byte_offset_to_utf16_character(line: &str, byte_offset: usize) -> Option<u32> {
    let byte_offset = byte_offset.min(line.len());
    line.is_char_boundary(byte_offset)
        .then(|| line[..byte_offset].encode_utf16().count() as u32)
}

pub(super) fn word_at_position(text: &str, position: Position) -> Option<(String, Range)> {
    let line = text.lines().nth(position.line as usize)?;
    let mut character = utf16_character_to_byte_offset(line, position.character)?;
    if character == line.len() && character > 0 {
        character -= 1;
        while character > 0 && !line.is_char_boundary(character) {
            character -= 1;
        }
    }
    let (start, end) = sage_index::identifiers::span_at(line, character)?;
    Some((
        line[start..end].to_string(),
        Range {
            start: Position {
                line: position.line,
                character: byte_offset_to_utf16_character(line, start)?,
            },
            end: Position {
                line: position.line,
                character: byte_offset_to_utf16_character(line, end)?,
            },
        },
    ))
}

pub(super) fn is_word_byte(byte: u8) -> bool {
    byte == b'_' || byte.is_ascii_alphanumeric() || byte >= 0x80
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn random_unicode_edits_match_character_model_with_all_line_endings() {
        let mut state = 42u64;
        let mut random = |bound: usize| {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
            (state >> 32) as usize % bound
        };
        let mut model: Vec<char> = "α😀\r\n中\rx\n".chars().collect();
        let mut actual: String = model.iter().collect();
        for _ in 0..1000 {
            let valid: Vec<_> = (0..=model.len())
                .filter(|&i| !(i > 0 && model[i - 1] == '\r' && model.get(i) == Some(&'\n')))
                .collect();
            let mut a = valid[random(valid.len())];
            let mut b = valid[random(valid.len())];
            if a > b {
                std::mem::swap(&mut a, &mut b);
            }
            let position = |offset: usize| {
                let (mut line, mut column) = (0, 0);
                for (i, ch) in model[..offset].iter().enumerate() {
                    match ch {
                        '\r' => {
                            line += 1;
                            column = 0;
                        }
                        '\n' => {
                            if i == 0 || model[i - 1] != '\r' {
                                line += 1;
                            }
                            column = 0;
                        }
                        ch => column += ch.len_utf16() as u32,
                    }
                }
                Position::new(line, column)
            };
            let replacement = ["😀", "α中", "\n", "\r\n", "\r", "", "ab"][random(7)];
            apply_text_document_change(
                &mut actual,
                &TextDocumentContentChangeEvent {
                    range: Some(Range::new(position(a), position(b))),
                    range_length: None,
                    text: replacement.into(),
                },
            )
            .unwrap();
            model.splice(a..b, replacement.chars());
            assert_eq!(actual, model.iter().collect::<String>());
        }
    }
}
