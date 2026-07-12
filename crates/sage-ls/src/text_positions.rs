//! LSP/`sage-index` text coordinate conversions.
//!
//! `sage-index` stores columns as UTF-8 byte offsets while LSP uses UTF-16 code units.
//! Keeping the conversion here makes the protocol boundary explicit and testable.

use sage_index::QueryPosition;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};
use tower_lsp::lsp_types::{Position, Range, TextDocumentContentChangeEvent};

pub(super) fn lsp_range_for_text(text: &str, range: &sage_index::SourceRange) -> Range {
    Range {
        start: lsp_position_for_byte_column(text, range.start_line, range.start_character),
        end: lsp_position_for_byte_column(text, range.end_line, range.end_character),
    }
}

pub(super) fn lsp_range_for_path(path: &Path, range: &sage_index::SourceRange) -> Range {
    std::fs::read_to_string(path)
        .ok()
        .map(|text| lsp_range_for_text(&text, range))
        .unwrap_or_else(|| raw_lsp_range(range))
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
    if text.is_empty() {
        return (target_line == 0).then_some((0, 0));
    }

    let mut line = 0u32;
    let mut start = 0usize;
    for segment in text.split_inclusive('\n') {
        let mut end = start + segment.len();
        if segment.ends_with('\n') {
            end = end.saturating_sub(1);
            if end > start && text.as_bytes().get(end - 1) == Some(&b'\r') {
                end -= 1;
            }
        }
        if line == target_line {
            return Some((start, end));
        }
        start += segment.len();
        line = line.saturating_add(1);
    }

    (text.ends_with('\n') && line == target_line).then_some((text.len(), text.len()))
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
    let bytes = line.as_bytes();
    if character >= bytes.len() {
        return None;
    }
    if !is_word_byte(bytes[character]) && character > 0 && is_word_byte(bytes[character - 1]) {
        character -= 1;
    }
    if !is_word_byte(bytes[character]) {
        return None;
    }
    let mut start = character;
    let mut end = character + 1;
    while start > 0 && is_word_byte(bytes[start - 1]) {
        start -= 1;
    }
    while end < bytes.len() && is_word_byte(bytes[end]) {
        end += 1;
    }
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
    byte == b'_' || byte.is_ascii_alphanumeric()
}
