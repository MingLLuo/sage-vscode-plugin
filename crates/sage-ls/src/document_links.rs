//! Document links for Sage `load`/`attach` and Cython `include` statements.

use super::{
    editor_features::code_before_comment,
    is_identifier_start,
    text_positions::{byte_offset_to_utf16_character, is_word_byte},
};
use std::path::{Path, PathBuf};
use tower_lsp::lsp_types::{DocumentLink, Position, Range, Url};

pub(super) fn sage_document_links(text: &str, document_path: &Path) -> Vec<DocumentLink> {
    let base_dir = document_path.parent().unwrap_or_else(|| Path::new("."));
    let mut links = Vec::new();
    for (line_number, line) in text.lines().enumerate() {
        links.extend(sage_load_attach_links_in_line(
            line,
            line_number as u32,
            base_dir,
        ));
        if let Some(link) = cython_include_link_in_line(line, line_number as u32, base_dir) {
            links.push(link);
        }
    }
    links
}

fn sage_load_attach_links_in_line(
    line: &str,
    line_number: u32,
    base_dir: &Path,
) -> Vec<DocumentLink> {
    let bytes = line.as_bytes();
    let mut links = Vec::new();
    let mut quote: Option<u8> = None;
    let mut escaped = false;
    let mut index = 0;
    while index < bytes.len() {
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
        if is_identifier_start(byte) && (index == 0 || !is_word_byte(bytes[index - 1])) {
            let start = index;
            let mut end = index + 1;
            while end < bytes.len() && is_word_byte(bytes[end]) {
                end += 1;
            }
            let name = &line[start..end];
            let mut cursor = end;
            while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
                cursor += 1;
            }
            if matches!(name, "load" | "attach") && cursor < bytes.len() && bytes[cursor] == b'(' {
                if let Some((target, inner_start, inner_end)) =
                    quoted_path_literal_after(line, cursor + 1)
                {
                    if let Some(link) = document_link_for_path_literal(
                        line,
                        line_number,
                        inner_start,
                        inner_end,
                        base_dir,
                        &target,
                    ) {
                        links.push(link);
                    }
                }
            }
            index = end;
            continue;
        }
        index += 1;
    }
    links
}

fn cython_include_link_in_line(
    line: &str,
    line_number: u32,
    base_dir: &Path,
) -> Option<DocumentLink> {
    let code = code_before_comment(line);
    let leading = code.len().saturating_sub(code.trim_start().len());
    let trimmed = code.trim_start();
    let rest = trimmed.strip_prefix("include")?;
    if !rest
        .as_bytes()
        .first()
        .is_some_and(|byte| byte.is_ascii_whitespace())
    {
        return None;
    }
    let offset = leading + "include".len();
    let (target, inner_start, inner_end) = quoted_path_literal_after(line, offset)?;
    document_link_for_path_literal(line, line_number, inner_start, inner_end, base_dir, &target)
}

fn quoted_path_literal_after(line: &str, offset: usize) -> Option<(String, usize, usize)> {
    let bytes = line.as_bytes();
    let mut cursor = offset;
    while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
        cursor += 1;
    }
    let quote = *bytes.get(cursor)?;
    if quote != b'\'' && quote != b'"' {
        return None;
    }
    let inner_start = cursor + 1;
    cursor = inner_start;
    let mut escaped = false;
    let mut value = Vec::new();
    while cursor < bytes.len() {
        let byte = bytes[cursor];
        if escaped {
            value.push(byte);
            escaped = false;
            cursor += 1;
            continue;
        }
        if byte == b'\\' {
            escaped = true;
            cursor += 1;
            continue;
        }
        if byte == quote {
            return Some((String::from_utf8(value).ok()?, inner_start, cursor));
        }
        value.push(byte);
        cursor += 1;
    }
    None
}

fn document_link_for_path_literal(
    line: &str,
    line_number: u32,
    start: usize,
    end: usize,
    base_dir: &Path,
    target: &str,
) -> Option<DocumentLink> {
    if target.trim().is_empty() {
        return None;
    }
    let target_path = PathBuf::from(target);
    let resolved = if target_path.is_absolute() {
        target_path
    } else {
        base_dir.join(target_path)
    };
    let resolved = normalize_path_lexically(resolved);
    Some(DocumentLink {
        range: Range::new(
            Position::new(
                line_number,
                byte_offset_to_utf16_character(line, start).unwrap_or(start as u32),
            ),
            Position::new(
                line_number,
                byte_offset_to_utf16_character(line, end).unwrap_or(end as u32),
            ),
        ),
        target: Url::from_file_path(resolved).ok(),
        tooltip: Some("Open referenced Sage/Cython file".to_string()),
        data: None,
    })
}

pub(super) fn normalize_path_lexically(path: PathBuf) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            std::path::Component::Normal(part) => normalized.push(part),
            std::path::Component::RootDir | std::path::Component::Prefix(_) => {
                normalized.push(component.as_os_str());
            }
        }
    }
    normalized
}
