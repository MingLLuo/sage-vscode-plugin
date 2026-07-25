//! Documentation lookup, source extraction, and hover-markdown rendering helpers.

use crate::initialization::DocumentationPreferredSource;
use crate::source_symbols::module_name_for_path;
use crate::text_positions::{lsp_position_for_byte_column, word_at_position};
use sage_index::{
    parse_source, DocumentationRecord, QueryPosition, QueryResult, SymbolRecord, WorkspaceIndex,
};
use std::path::Path;
use tower_lsp::lsp_types::Url;

pub(super) fn documentation_record_for_source_position(
    index: &WorkspaceIndex,
    path: &Path,
    text: &str,
    position: QueryPosition,
) -> Option<DocumentationRecord> {
    if let Some(documentation) = index
        .query_source_at(path, text, position, None)
        .documentation
    {
        return Some(documentation);
    }
    let (word, _) = word_at_position(
        text,
        lsp_position_for_byte_column(text, position.line, position.character),
    )?;
    let symbols = parse_source(module_name_for_path(path), path, text).symbols;
    symbols
        .iter()
        .find(|symbol| {
            symbol.name == word && source_range_contains_position(&symbol.range, position)
        })
        .or_else(|| {
            symbols
                .iter()
                .find(|symbol| symbol.name == word && symbol.range.start_line == position.line)
        })
        .cloned()
        .map(documentation_record_from_symbol)
}

fn source_range_contains_position(
    range: &sage_index::SourceRange,
    position: QueryPosition,
) -> bool {
    let starts_before = range.start_line < position.line
        || (range.start_line == position.line && range.start_character <= position.character);
    let ends_after = range.end_line > position.line
        || (range.end_line == position.line && range.end_character >= position.character);
    starts_before && ends_after
}

fn documentation_record_from_symbol(symbol: SymbolRecord) -> DocumentationRecord {
    let summary = symbol
        .docstring
        .as_deref()
        .and_then(first_docstring_summary_line)
        .unwrap_or_else(|| symbol.detail.clone());
    DocumentationRecord {
        name: symbol.name,
        module_name: symbol.module,
        kind: format!("{:?}", symbol.kind),
        detail: symbol
            .signature
            .clone()
            .unwrap_or_else(|| symbol.detail.clone()),
        summary,
        docstring: symbol.docstring,
        uri: Url::from_file_path(&symbol.path)
            .ok()
            .map(|uri| uri.to_string()),
        markers: Vec::new(),
        sections: Vec::new(),
    }
}

fn first_docstring_summary_line(docstring: &str) -> Option<String> {
    docstring
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(str::to_string)
}

pub(super) fn is_runtime_placeholder_documentation(record: &DocumentationRecord) -> bool {
    record
        .docstring
        .as_deref()
        .is_some_and(|docstring| docstring.contains("Runtime documentation worker can provide"))
}

pub(super) fn runtime_docs_symbol_for_query(
    query: &QueryResult,
    preferred_source: DocumentationPreferredSource,
) -> Option<&str> {
    match preferred_source {
        DocumentationPreferredSource::Workspace | DocumentationPreferredSource::Reference => None,
        DocumentationPreferredSource::Auto => {
            let documentation = query.documentation.as_ref()?;
            if !is_runtime_placeholder_documentation(documentation) {
                return None;
            }
            query
                .target
                .as_ref()
                .and_then(|target| target.dotted_symbol.as_deref())
                .or(Some(documentation.name.as_str()))
        }
        DocumentationPreferredSource::Runtime => query
            .target
            .as_ref()
            .and_then(|target| target.dotted_symbol.as_deref())
            .or_else(|| query.target.as_ref().map(|target| target.symbol.as_str()))
            .or_else(|| {
                query
                    .documentation
                    .as_ref()
                    .map(|record| record.name.as_str())
            }),
    }
}

pub(super) fn hover_markdown_for_documentation(record: &DocumentationRecord) -> String {
    let mut lines = vec![
        "```sage".to_string(),
        if record.detail.is_empty() {
            record.name.clone()
        } else {
            record.detail.clone()
        },
        "```".to_string(),
        String::new(),
        format!("Module: `{}`", record.module_name),
    ];
    let body = record
        .docstring
        .as_deref()
        .filter(|docstring| !docstring.trim().is_empty())
        .unwrap_or(&record.summary);
    if !body.trim().is_empty() {
        lines.push(String::new());
        lines.push(compact_hover_docstring(body));
    }
    lines.join("\n")
}

pub(super) fn hover_markdown_for_hover_setting(markdown: &str, show_docs_on_hover: bool) -> String {
    if show_docs_on_hover {
        return markdown.to_string();
    }
    hover_markdown_without_doc_preview(markdown)
}

fn hover_markdown_without_doc_preview(markdown: &str) -> String {
    let mut sections = markdown.split("\n\n");
    let signature = sections.next().unwrap_or_default().trim_end();
    let module = sections
        .find(|section| section.trim_start().starts_with("Module:"))
        .map(str::trim_end);

    match (signature.is_empty(), module) {
        (false, Some(module)) => format!("{signature}\n\n{module}"),
        (false, None) => signature.to_string(),
        (true, Some(module)) => module.to_string(),
        (true, None) => markdown.lines().take(3).collect::<Vec<_>>().join("\n"),
    }
}

fn compact_hover_docstring(docstring: &str) -> String {
    const MAX_LINES: usize = 24;
    const MAX_BYTES: usize = 2400;

    let trimmed = docstring.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    let mut output = String::new();
    let mut truncated = false;
    for (line_count, line) in trimmed.lines().enumerate() {
        if line_count >= MAX_LINES || output.len() + line.len() + 1 > MAX_BYTES {
            truncated = true;
            break;
        }
        if !output.is_empty() {
            output.push('\n');
        }
        output.push_str(line);
    }

    if truncated {
        output.push_str("\n\n... (open Sage documentation for the full docstring)");
    }
    output
}
