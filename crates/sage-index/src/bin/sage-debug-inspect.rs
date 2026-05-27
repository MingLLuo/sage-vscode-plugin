use anyhow::{bail, Context, Result};
use sage_index::{
    default_cache_dir, parse_file_for_roots, preprocess_sage_source, semantic_spans, IndexOptions,
    QueryExecutionOptions, QueryFeatures, QueryPosition, WorkspaceIndex,
};
use serde::Deserialize;
use serde_json::json;
use std::env;
use std::fs;
use std::path::PathBuf;
use std::time::Instant;

#[derive(Debug, Deserialize)]
struct BatchQuery {
    id: String,
    line: Option<u32>,
    character: Option<u32>,
    symbol: Option<String>,
    rename_to: Option<String>,
}

fn main() -> Result<()> {
    let mut roots: Vec<PathBuf> = Vec::new();
    let mut editable_roots: Vec<PathBuf> = Vec::new();
    let mut file: Option<PathBuf> = None;
    let mut rebuild_index = false;
    let mut line: Option<u32> = None;
    let mut character: Option<u32> = None;
    let mut symbol: Option<String> = None;
    let mut rename_to: Option<String> = None;
    let mut batch_file: Option<PathBuf> = None;

    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--root" => roots.push(
                args.next()
                    .map(PathBuf::from)
                    .context("missing --root value")?,
            ),
            "--editable-root" => editable_roots.push(
                args.next()
                    .map(PathBuf::from)
                    .context("missing --editable-root value")?,
            ),
            "--file" => file = args.next().map(PathBuf::from),
            "--line" => line = args.next().and_then(|value| value.parse().ok()),
            "--character" => character = args.next().and_then(|value| value.parse().ok()),
            "--symbol" => symbol = args.next(),
            "--rename-to" => rename_to = args.next(),
            "--batch-file" => batch_file = args.next().map(PathBuf::from),
            "--rebuild-index" => rebuild_index = true,
            "--help" | "-h" => {
                print_usage();
                return Ok(());
            }
            other => bail!("unknown argument: {other}"),
        }
    }

    if roots.is_empty() {
        bail!("missing --root");
    }
    let roots: Vec<PathBuf> = roots
        .into_iter()
        .map(|root| root.canonicalize())
        .collect::<std::io::Result<_>>()?;
    let editable_roots: Vec<PathBuf> = editable_roots
        .into_iter()
        .map(|root| root.canonicalize())
        .collect::<std::io::Result<_>>()?;
    let file = file.context("missing --file")?.canonicalize()?;
    if !roots.iter().any(|root| file.starts_with(root)) {
        bail!("file must be inside indexed roots: {}", file.display());
    }

    let source = fs::read_to_string(&file).with_context(|| format!("read {}", file.display()))?;
    let parsed = parse_file_for_roots(&file, &roots)?;
    let mut index = WorkspaceIndex::new(IndexOptions {
        roots: roots.clone(),
        editable_roots: editable_roots.clone(),
        exclude_globs: default_excludes(),
        cache_dir: default_cache_dir(),
        enable_pyx: true,
    });
    let index_status = if rebuild_index {
        index.rebuild()?
    } else {
        index.hydrate_from_cache()?
    };
    let diagnostics = index.diagnostics_for_source(&file, &source);
    let query = match (line, character, symbol.as_deref()) {
        (Some(line), Some(character), _) => Some(index.query_source_at(
            &file,
            &source,
            QueryPosition { line, character },
            rename_to.as_deref(),
        )),
        (_, _, Some(symbol)) => Some(index.query_source_symbol(
            &file,
            &source,
            symbol,
            None,
            rename_to.as_deref(),
            diagnostics.clone(),
        )),
        _ => None,
    };
    let batch_queries = if let Some(batch_file) = batch_file {
        let batch_source = fs::read_to_string(&batch_file)
            .with_context(|| format!("read batch query file {}", batch_file.display()))?;
        let batch: Vec<BatchQuery> = serde_json::from_str(&batch_source)
            .with_context(|| format!("parse batch query file {}", batch_file.display()))?;
        let mut results = Vec::new();
        for item in batch {
            let started = Instant::now();
            let result = match (item.line, item.character, item.symbol.as_deref()) {
                (Some(line), Some(character), _) => index.query_source_at_navigation(
                    &file,
                    &source,
                    QueryPosition { line, character },
                ),
                (_, _, Some(symbol)) => index.query_source_symbol_with_options(
                    &file,
                    &source,
                    symbol,
                    None,
                    QueryExecutionOptions {
                        rename_to: item.rename_to.as_deref(),
                        diagnostics: diagnostics.clone(),
                        features: QueryFeatures::navigation(),
                    },
                ),
                _ => sage_index::QueryResult {
                    fallback_reason: Some("batch-query-missing-position-or-symbol".to_string()),
                    diagnostics: diagnostics.clone(),
                    ..sage_index::QueryResult::default()
                },
            };
            results.push(json!({
                "id": item.id,
                "timing_ms": started.elapsed().as_millis(),
                "query": result,
            }));
        }
        results
    } else {
        Vec::new()
    };
    let preprocess = if file
        .extension()
        .is_some_and(|extension| extension == "sage")
    {
        Some(preprocess_sage_source(&source))
    } else {
        None
    };

    let primary_root = roots.first().cloned();
    let payload = json!({
        "root": primary_root,
        "roots": roots,
        "editableRoots": editable_roots,
        "file": file,
        "source": source,
        "parsed": parsed,
        "semanticSpans": semantic_spans(&source),
        "diagnostics": diagnostics,
        "preprocess": preprocess,
        "query": query,
        "batchQueries": batch_queries,
        "indexStatus": index_status,
        "docsStatus": index.docs_status(),
    });
    println!("{}", serde_json::to_string_pretty(&payload)?);
    Ok(())
}

fn default_excludes() -> Vec<String> {
    vec![
        "**/.git/**".to_string(),
        "**/__pycache__/**".to_string(),
        "**/.venv/**".to_string(),
        "**/build/**".to_string(),
        "**/target/**".to_string(),
    ]
}

fn print_usage() {
    eprintln!("usage: sage-debug-inspect --root <index-root> [--root <extra-root>...] [--editable-root <workspace-root>...] --file <file> [--line <line> --character <character> | --symbol <symbol> | --batch-file <json>] [--rename-to <name>] [--rebuild-index]");
}
