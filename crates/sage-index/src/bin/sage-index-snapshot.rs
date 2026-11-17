//! Deterministic analysis snapshot for regression comparison.
//!
//! Emits one JSON line per indexable file with digests of everything the analyzer derives from
//! source text alone: symbols, semantic spans, and syntax diagnostics. Two snapshots of the same
//! source tree can be diffed line by line to prove that a parser or dependency change did not
//! alter analysis results (`scripts/index-snapshot-diff.mjs`).

use anyhow::{Context, Result};
use rayon::prelude::*;
use sage_index::{
    collect_indexable_paths, default_cache_dir, parse_file_for_roots, semantic_spans, IndexOptions,
    WorkspaceIndex,
};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::env;
use std::io::Write;
use std::path::PathBuf;

fn digest(parts: impl IntoIterator<Item = String>) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part.as_bytes());
        hasher.update([0]);
    }
    let bytes = hasher.finalize();
    bytes[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn main() -> Result<()> {
    // `--diagnostics` adds every diagnostic (code, message, range) so remaining reports on
    // library code can be reviewed one by one, not only counted.
    let with_diagnostics = env::args().any(|arg| arg == "--diagnostics");
    let roots: Vec<PathBuf> = env::args()
        .skip(1)
        .filter(|arg| arg != "--diagnostics")
        .map(|root| sage_index::canonicalize_path(&PathBuf::from(root)))
        .collect::<std::io::Result<_>>()
        .context("resolve source roots")?;
    if roots.is_empty() {
        anyhow::bail!("usage: sage-index-snapshot <source-root> [source-root...] > snapshot.jsonl");
    }
    let options = IndexOptions {
        roots: roots.clone(),
        editable_roots: Vec::new(),
        exclude_globs: vec![
            "**/.git/**".to_string(),
            "**/__pycache__/**".to_string(),
            "**/.venv/**".to_string(),
            "**/build/**".to_string(),
        ],
        cache_dir: default_cache_dir(),
        enable_pyx: true,
    };
    let index = WorkspaceIndex::new(options);
    let mut paths = collect_indexable_paths(index.options());
    paths.sort();

    let mut lines: Vec<(String, String)> = paths
        .par_iter()
        .filter_map(|path| {
            let source = std::fs::read_to_string(path).ok()?;
            let parsed = parse_file_for_roots(path, &roots).ok()?;
            let relative = roots
                .iter()
                .find_map(|root| path.strip_prefix(root).ok())
                .unwrap_or(path)
                .to_string_lossy()
                .replace('\\', "/");
            let symbols = digest(parsed.symbols.iter().map(|symbol| {
                format!(
                    "{}|{:?}|{}|{}:{}-{}:{}|{}|{}|{}|{}",
                    symbol.name,
                    symbol.kind,
                    symbol.module,
                    symbol.range.start_line,
                    symbol.range.start_character,
                    symbol.range.end_line,
                    symbol.range.end_character,
                    symbol.detail,
                    symbol.docstring.as_deref().unwrap_or(""),
                    symbol.import_from.as_deref().unwrap_or(""),
                    symbol.signature.as_deref().unwrap_or(""),
                )
            }));
            let spans = semantic_spans(&source);
            let span_digest = digest(spans.iter().map(|span| {
                format!(
                    "{}:{}+{}|{}|{}",
                    span.line,
                    span.start,
                    span.length,
                    span.token_type,
                    span.modifiers.join(",")
                )
            }));
            let diagnostics = index.diagnostics_for_source(path, &source);
            let diagnostic_digest = digest(diagnostics.iter().map(|diagnostic| {
                format!(
                    "{}|{}|{}:{}-{}:{}",
                    diagnostic.code,
                    diagnostic.message,
                    diagnostic.range.start_line,
                    diagnostic.range.start_character,
                    diagnostic.range.end_line,
                    diagnostic.range.end_character,
                )
            }));
            let mut line = json!({
                "path": relative,
                "module": parsed.module,
                "symbols": parsed.symbols.len(),
                "symbol_digest": symbols,
                "semantic_spans": spans.len(),
                "semantic_digest": span_digest,
                "diagnostics": diagnostics.len(),
                "diagnostic_digest": diagnostic_digest,
            });
            if with_diagnostics && !diagnostics.is_empty() {
                line["diagnostic_details"] = json!(diagnostics
                    .iter()
                    .map(|diagnostic| json!({
                        "code": diagnostic.code,
                        "message": diagnostic.message,
                        "line": diagnostic.range.start_line + 1,
                        "column": diagnostic.range.start_character,
                    }))
                    .collect::<Vec<_>>());
            }
            Some((relative, line.to_string()))
        })
        .collect();
    lines.sort();

    let stdout = std::io::stdout();
    let mut out = std::io::BufWriter::new(stdout.lock());
    for (_, line) in &lines {
        writeln!(out, "{line}")?;
    }
    out.flush()?;
    eprintln!("snapshot: {} files", lines.len());
    Ok(())
}
