//! Encoding index semantic spans as LSP semantic tokens (UTF-16 columns, delta form).

use super::*;

pub(crate) fn encode_semantic_tokens(text: &str) -> Vec<SemanticToken> {
    encode_semantic_spans(text, semantic_spans(text))
}

#[cfg(test)]
pub(crate) fn encode_semantic_tokens_for_range(text: &str, range: Range) -> Vec<SemanticToken> {
    tokens_for_range(&encode_semantic_tokens(text), range)
}

pub(crate) fn tokens_for_range(tokens: &[SemanticToken], range: Range) -> Vec<SemanticToken> {
    let (mut line, mut column, mut previous_line, mut previous_column) = (0, 0, 0, 0);
    let mut filtered = Vec::new();
    for token in tokens {
        line += token.delta_line;
        column = if token.delta_line == 0 {
            column + token.delta_start
        } else {
            token.delta_start
        };
        if line > range.end.line {
            break;
        }
        if line < range.start.line
            || (line == range.start.line && column + token.length <= range.start.character)
            || (line == range.end.line && column >= range.end.character)
        {
            continue;
        }
        let mut token = *token;
        token.delta_line = line - previous_line;
        token.delta_start = if token.delta_line == 0 {
            column - previous_column
        } else {
            column
        };
        filtered.push(token);
        previous_line = line;
        previous_column = column;
    }
    filtered
}

pub(crate) fn encode_semantic_spans<I>(text: &str, spans: I) -> Vec<SemanticToken>
where
    I: IntoIterator<Item = sage_index::SemanticSpan>,
{
    let lines = text_positions::LineIndex::new(text);
    let mut data = Vec::new();
    let mut previous_line = 0u32;
    let mut previous_start = 0u32;
    for span in spans {
        let Some((start, length)) = semantic_span_lsp_columns(text, &lines, &span) else {
            continue;
        };
        let delta_line = span.line.saturating_sub(previous_line);
        let delta_start = if delta_line == 0 {
            start.saturating_sub(previous_start)
        } else {
            start
        };
        data.push(SemanticToken {
            delta_line,
            delta_start,
            length,
            token_type: token_type_index(&span.token_type),
            token_modifiers_bitset: modifier_bitset(&span.modifiers),
        });
        previous_line = span.line;
        previous_start = start;
    }
    data
}

pub(crate) fn semantic_span_lsp_columns(
    text: &str,
    lines: &text_positions::LineIndex,
    span: &sage_index::SemanticSpan,
) -> Option<(u32, u32)> {
    let (line_start, line_end) = lines.bounds(span.line)?;
    let line = &text[line_start..line_end];
    let start = byte_offset_to_utf16_character(line, span.start as usize)?;
    let end =
        byte_offset_to_utf16_character(line, span.start.saturating_add(span.length) as usize)?;
    Some((start, end.saturating_sub(start)))
}

pub(crate) fn token_type_index(name: &str) -> u32 {
    match name {
        "namespace" => 0,
        "type" => 1,
        "class" => 2,
        "function" => 3,
        "method" => 4,
        "variable" => 5,
        "parameter" => 6,
        "keyword" => 7,
        "decorator" => 8,
        _ => 5,
    }
}

pub(crate) fn modifier_bitset(modifiers: &[String]) -> u32 {
    let mut bits = 0;
    for modifier in modifiers {
        match modifier.as_str() {
            "declaration" => bits |= 1 << 0,
            "readonly" => bits |= 1 << 1,
            "defaultLibrary" => bits |= 1 << 2,
            _ => {}
        }
    }
    bits
}
