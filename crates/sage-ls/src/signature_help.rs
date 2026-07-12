//! Signature-help construction and UTF-16 parameter label offsets.

use super::text_positions::byte_offset_to_utf16_character;
use tower_lsp::lsp_types::{
    Documentation, ParameterInformation, ParameterLabel, SignatureInformation,
};

pub(super) fn signature_information(
    label: String,
    documentation: Option<String>,
    active_parameter: u32,
) -> SignatureInformation {
    let parameters = signature_parameter_information(&label);
    SignatureInformation {
        label,
        documentation: documentation.map(Documentation::String),
        parameters: (!parameters.is_empty()).then_some(parameters),
        active_parameter: Some(active_parameter),
    }
}

fn signature_parameter_information(label: &str) -> Vec<ParameterInformation> {
    signature_parameter_offsets(label)
        .into_iter()
        .map(|offsets| ParameterInformation {
            label: ParameterLabel::LabelOffsets(offsets),
            documentation: None,
        })
        .collect()
}

pub(super) fn signature_parameter_offsets(label: &str) -> Vec<[u32; 2]> {
    let Some(open) = label.find('(') else {
        return Vec::new();
    };
    let Some(close) = matching_signature_close(label, open) else {
        return Vec::new();
    };
    let mut offsets = Vec::new();
    let mut start = open + 1;
    let mut depth = 0usize;
    let mut quote: Option<char> = None;
    let mut escaped = false;

    for (relative, ch) in label[open + 1..close].char_indices() {
        let index = open + 1 + relative;
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
            Some(_) => continue,
            None if ch == '\'' || ch == '"' => {
                quote = Some(ch);
                continue;
            }
            None => {}
        }
        match ch {
            '(' | '[' | '{' => depth = depth.saturating_add(1),
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                push_signature_parameter_offset(label, start, index, &mut offsets);
                start = index + 1;
            }
            _ => {}
        }
    }
    push_signature_parameter_offset(label, start, close, &mut offsets);
    offsets
}

fn matching_signature_close(label: &str, open: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for (relative, ch) in label[open..].char_indices() {
        let index = open + relative;
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
            Some(_) => continue,
            None if ch == '\'' || ch == '"' => {
                quote = Some(ch);
                continue;
            }
            None => {}
        }
        match ch {
            '(' => depth = depth.saturating_add(1),
            ')' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => {}
        }
    }
    None
}

fn push_signature_parameter_offset(
    label: &str,
    start: usize,
    end: usize,
    offsets: &mut Vec<[u32; 2]>,
) {
    let mut trimmed_start = start;
    let mut trimmed_end = end;
    while trimmed_start < trimmed_end && label.as_bytes()[trimmed_start].is_ascii_whitespace() {
        trimmed_start += 1;
    }
    while trimmed_end > trimmed_start && label.as_bytes()[trimmed_end - 1].is_ascii_whitespace() {
        trimmed_end -= 1;
    }
    if trimmed_start < trimmed_end {
        let start =
            byte_offset_to_utf16_character(label, trimmed_start).unwrap_or(trimmed_start as u32);
        let end = byte_offset_to_utf16_character(label, trimmed_end).unwrap_or(trimmed_end as u32);
        offsets.push([start, end]);
    }
}
