//! Pure editor-facing helpers for selections, folding, and inlay hints.

use super::text_positions::{
    byte_offset_to_utf16_character, utf16_character_to_byte_offset, word_at_position,
};
use std::{collections::BTreeSet, sync::OnceLock};
use tower_lsp::lsp_types::{
    FoldingRange, FoldingRangeKind, InlayHint, InlayHintKind, InlayHintLabel, InlayHintTooltip,
    Position, Range, SelectionRange,
};

pub(super) fn sage_selection_ranges(text: &str, positions: &[Position]) -> Vec<SelectionRange> {
    positions
        .iter()
        .copied()
        .map(|position| sage_selection_range(text, position))
        .collect()
}

pub(super) fn sage_selection_range(text: &str, position: Position) -> SelectionRange {
    let mut ranges = Vec::new();
    let has_word = if let Some((_, word_range)) = word_at_position(text, position) {
        push_selection_range(&mut ranges, word_range);
        true
    } else {
        push_selection_range(&mut ranges, Range::new(position, position));
        false
    };
    if let Some(line_range) = line_selection_range(text, position.line, position.character) {
        push_selection_range(&mut ranges, line_range);
    }
    for block_range in block_selection_ranges(text, position) {
        push_selection_range(&mut ranges, block_range);
    }
    push_selection_range(&mut ranges, document_selection_range(text));
    if ranges.is_empty()
        || (!has_word && !contains_range(&ranges[0], &Range::new(position, position)))
    {
        push_selection_range(&mut ranges, Range::new(position, position));
    }
    selection_range_chain(ranges)
}

pub(super) fn line_selection_range(text: &str, line_number: u32, character: u32) -> Option<Range> {
    let line = text.lines().nth(line_number as usize)?;
    let byte_character = utf16_character_to_byte_offset(line, character)?;
    let mut start = line.len().saturating_sub(line.trim_start().len());
    let mut end = line.trim_end().len();
    if byte_character < start || byte_character > end {
        start = 0;
        end = line.len();
    }
    let (start, end) = if start <= end {
        (start, end)
    } else {
        (0, line.len())
    };
    Some(Range::new(
        Position::new(
            line_number,
            byte_offset_to_utf16_character(line, start).unwrap_or(start as u32),
        ),
        Position::new(
            line_number,
            byte_offset_to_utf16_character(line, end).unwrap_or(end as u32),
        ),
    ))
}

fn block_selection_ranges(text: &str, position: Position) -> Vec<Range> {
    let mut ranges: Vec<_> = sage_folding_ranges(text)
        .into_iter()
        .filter(|fold| fold.start_line <= position.line && position.line <= fold.end_line)
        .map(|fold| {
            Range::new(
                Position::new(fold.start_line, 0),
                Position::new(fold.end_line, line_length(text, fold.end_line) as u32),
            )
        })
        .filter(|range| range.start != range.end)
        .collect();
    ranges.sort_by_key(|range| {
        (
            range.end.line.saturating_sub(range.start.line),
            range.end.character.saturating_sub(range.start.character),
            range.start.line,
            range.start.character,
        )
    });
    ranges.dedup_by(|left, right| left == right);
    ranges
        .into_iter()
        .filter(|range| {
            (
                range.end.line.saturating_sub(range.start.line),
                range.end.character.saturating_sub(range.start.character),
            ) != (0, 0)
        })
        .collect()
}

fn document_selection_range(text: &str) -> Range {
    let line_count = text.lines().count();
    if line_count == 0 {
        return Range::new(Position::new(0, 0), Position::new(0, 0));
    }
    let end_line = line_count.saturating_sub(1) as u32;
    Range::new(
        Position::new(0, 0),
        Position::new(end_line, line_length(text, end_line) as u32),
    )
}

pub(super) fn line_length(text: &str, line_number: u32) -> usize {
    text.lines()
        .nth(line_number as usize)
        .map(|line| line.encode_utf16().count())
        .unwrap_or_default()
}

fn push_selection_range(ranges: &mut Vec<Range>, range: Range) {
    if ranges.last().is_some_and(|existing| *existing == range) {
        return;
    }
    if ranges
        .last()
        .is_none_or(|existing| contains_range(&range, existing))
    {
        ranges.push(range);
    }
}

pub(super) fn contains_range(outer: &Range, inner: &Range) -> bool {
    position_leq(outer.start, inner.start) && position_leq(inner.end, outer.end)
}

pub(super) fn position_leq(left: Position, right: Position) -> bool {
    left.line < right.line || (left.line == right.line && left.character <= right.character)
}

fn selection_range_chain(ranges: Vec<Range>) -> SelectionRange {
    let mut parent = None;
    for range in ranges.into_iter().rev() {
        parent = Some(Box::new(SelectionRange { range, parent }));
    }
    *parent.expect("selection range chain should contain at least one range")
}

pub(super) fn sage_folding_ranges(text: &str) -> Vec<FoldingRange> {
    let lines: Vec<_> = text.lines().collect();
    let mut ranges = Vec::new();
    add_explicit_region_folds(&lines, &mut ranges);
    add_comment_block_folds(&lines, &mut ranges);
    add_indentation_folds(&lines, &mut ranges);
    dedupe_folding_ranges(ranges)
}

fn add_explicit_region_folds(lines: &[&str], ranges: &mut Vec<FoldingRange>) {
    let mut stack = Vec::new();
    for (line_number, line) in lines.iter().enumerate() {
        let normalized = line.trim_start().to_ascii_lowercase();
        if normalized.starts_with("# region") {
            stack.push(line_number);
        } else if normalized.starts_with("# endregion") {
            let Some(start_line) = stack.pop() else {
                continue;
            };
            if line_number > start_line {
                ranges.push(folding_range(
                    start_line,
                    line_number,
                    Some(FoldingRangeKind::Region),
                ));
            }
        }
    }
}

fn add_comment_block_folds(lines: &[&str], ranges: &mut Vec<FoldingRange>) {
    let mut start_line = None;
    for (line_number, line) in lines.iter().enumerate() {
        let normalized = line.trim_start().to_ascii_lowercase();
        let is_comment = normalized.starts_with('#')
            && !normalized.starts_with("# region")
            && !normalized.starts_with("# endregion");
        if is_comment {
            start_line.get_or_insert(line_number);
            continue;
        }
        if let Some(start) = start_line.take() {
            if line_number.saturating_sub(start) > 1 {
                ranges.push(folding_range(
                    start,
                    line_number - 1,
                    Some(FoldingRangeKind::Comment),
                ));
            }
        }
    }
    if let Some(start) = start_line {
        if lines.len().saturating_sub(start) > 1 {
            ranges.push(folding_range(
                start,
                lines.len() - 1,
                Some(FoldingRangeKind::Comment),
            ));
        }
    }
}

fn add_indentation_folds(lines: &[&str], ranges: &mut Vec<FoldingRange>) {
    for (line_number, line) in lines.iter().enumerate() {
        let code = code_before_comment(line).trim_end();
        if !is_foldable_block_header(code.trim_start()) {
            continue;
        }
        let start_indent = leading_indent_width(line);
        let mut last_inside_line = None;
        for (next_line_number, next_line) in lines.iter().enumerate().skip(line_number + 1) {
            let trimmed = next_line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let indent = leading_indent_width(next_line);
            if indent <= start_indent {
                break;
            }
            last_inside_line = Some(next_line_number);
        }
        let Some(end_line) = last_inside_line else {
            continue;
        };
        if end_line > line_number {
            ranges.push(folding_range(line_number, end_line, None));
        }
    }
}

fn is_foldable_block_header(trimmed_code: &str) -> bool {
    if !trimmed_code.ends_with(':') {
        return false;
    }
    let headers = [
        "def ",
        "async def ",
        "class ",
        "cdef ",
        "cpdef ",
        "if ",
        "elif ",
        "else:",
        "for ",
        "async for ",
        "while ",
        "with ",
        "async with ",
        "try:",
        "except",
        "finally:",
        "match ",
        "case ",
    ];
    headers
        .iter()
        .any(|header| trimmed_code.starts_with(header))
}

fn leading_indent_width(line: &str) -> usize {
    line.chars()
        .take_while(|ch| *ch == ' ' || *ch == '\t')
        .map(|ch| if ch == '\t' { 4 } else { 1 })
        .sum()
}

fn folding_range(
    start_line: usize,
    end_line: usize,
    kind: Option<FoldingRangeKind>,
) -> FoldingRange {
    FoldingRange {
        start_line: start_line as u32,
        start_character: None,
        end_line: end_line as u32,
        end_character: None,
        kind,
        collapsed_text: None,
    }
}

fn dedupe_folding_ranges(ranges: Vec<FoldingRange>) -> Vec<FoldingRange> {
    let mut seen = BTreeSet::new();
    let mut deduped = Vec::new();
    for range in ranges {
        let kind = match &range.kind {
            Some(FoldingRangeKind::Comment) => "comment",
            Some(FoldingRangeKind::Imports) => "imports",
            Some(FoldingRangeKind::Region) => "region",
            None => "code",
        };
        let key = (range.start_line, range.end_line, kind);
        if seen.insert(key) {
            deduped.push(range);
        }
    }
    deduped.sort_by_key(|range| (range.start_line, range.end_line));
    deduped
}

pub(super) fn sage_inlay_hints(text: &str, range: Range) -> Vec<InlayHint> {
    text.lines()
        .enumerate()
        .filter(|(line_number, _)| {
            let line = *line_number as u32;
            line >= range.start.line && line <= range.end.line
        })
        .filter_map(|(line_number, line)| {
            let code = code_before_comment(line).trim_end();
            let assignment = sage_assignment_for_inlay(code)?;
            let label = infer_sage_inlay_label(assignment.rhs)?;
            Some(InlayHint {
                position: Position {
                    line: line_number as u32,
                    character: byte_offset_to_utf16_character(line, assignment.name_end)
                        .unwrap_or(assignment.name_end as u32),
                },
                label: InlayHintLabel::String(format!(": {label}")),
                kind: Some(InlayHintKind::TYPE),
                text_edits: None,
                tooltip: Some(InlayHintTooltip::String(
                    "Sage static type hint inferred from the assignment expression.".to_string(),
                )),
                padding_left: Some(true),
                padding_right: Some(false),
                data: None,
            })
        })
        .collect()
}

#[derive(Clone, Copy, Debug)]
struct SageInlayAssignment<'a> {
    name_end: usize,
    rhs: &'a str,
}

fn sage_assignment_for_inlay(line: &str) -> Option<SageInlayAssignment<'_>> {
    static PREPARSER_ASSIGNMENT_RE: OnceLock<regex::Regex> = OnceLock::new();
    static SIMPLE_ASSIGNMENT_RE: OnceLock<regex::Regex> = OnceLock::new();
    let preparser = PREPARSER_ASSIGNMENT_RE.get_or_init(|| {
        regex::Regex::new(r"^\s*(?P<name>[A-Za-z_][A-Za-z0-9_]*)\s*\.<[^>]+>\s*=\s*(?P<rhs>.+)$")
            .expect("valid preparser assignment regex")
    });
    let simple = SIMPLE_ASSIGNMENT_RE.get_or_init(|| {
        regex::Regex::new(r"^\s*(?P<name>[A-Za-z_][A-Za-z0-9_]*)\s*=\s*(?P<rhs>.+)$")
            .expect("valid assignment regex")
    });
    let captures = preparser.captures(line).or_else(|| simple.captures(line))?;
    let name = captures.name("name")?;
    let rhs = captures.name("rhs")?.as_str().trim_start();
    if rhs.starts_with('=') {
        return None;
    }
    Some(SageInlayAssignment {
        name_end: name.end(),
        rhs,
    })
}

fn infer_sage_inlay_label(rhs: &str) -> Option<&'static str> {
    let normalized = rhs.trim_start();
    if starts_with_call(normalized, &["GF", "FiniteField"]) {
        return Some("Field");
    }
    if starts_with_call(normalized, &["PolynomialRing", "BooleanPolynomialRing"]) {
        return Some("PolynomialRing");
    }
    if starts_with_call(
        normalized,
        &[
            "matrix",
            "Matrix",
            "zero_matrix",
            "identity_matrix",
            "random_matrix",
        ],
    ) {
        return Some("Matrix");
    }
    if starts_with_call(normalized, &["vector", "zero_vector", "random_vector"]) {
        return Some("Vector");
    }
    if starts_with_call(normalized, &["Graph", "DiGraph"]) {
        return Some("Graph");
    }
    if starts_with_call(normalized, &["EllipticCurve"]) {
        return Some("EllipticCurve");
    }
    if starts_with_call(
        normalized,
        &["NumberField", "CyclotomicField", "QuadraticField"],
    ) {
        return Some("NumberField");
    }
    if normalized.contains(".ideal(") || starts_with_call(normalized, &["ideal"]) {
        return Some("Ideal");
    }
    if normalized.contains(".gen(") || normalized.contains(".gen()") {
        return Some("PolynomialElement");
    }
    None
}

fn starts_with_call(value: &str, names: &[&str]) -> bool {
    names.iter().any(|name| {
        value
            .strip_prefix(name)
            .is_some_and(|rest| rest.trim_start().starts_with('('))
    })
}

pub(super) fn code_before_comment(line: &str) -> &str {
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for (offset, ch) in line.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        match quote {
            Some(active) if ch == active => quote = None,
            Some(_) => {}
            None if ch == '\'' || ch == '"' => quote = Some(ch),
            None if ch == '#' => return &line[..offset],
            None => {}
        }
    }
    line
}
