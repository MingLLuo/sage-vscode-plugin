use anyhow::{Context, Result};
use rayon::prelude::*;
use sage_index::{
    collect_indexable_paths, default_cache_dir, parse_file_for_roots, IndexOptions, WorkspaceIndex,
};
use std::env;
use std::path::PathBuf;
use std::time::Instant;

fn main() -> Result<()> {
    let roots: Vec<PathBuf> = env::args().skip(1).map(PathBuf::from).collect();
    if roots.is_empty() {
        anyhow::bail!("usage: sage-index-bench <source-root> [source-root...]");
    }

    let options = IndexOptions {
        roots,
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
    let mut index = WorkspaceIndex::new(options);
    if env::var_os("SAGE_INDEX_BENCH_HYDRATE_ONLY").is_some() {
        let started = Instant::now();
        let status = index.hydrate_from_cache().context("hydrate index")?;
        let hydrate_ms = started.elapsed().as_millis();
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "schema_version": 1,
                "mode": "hydrate",
                "hydrate_ms": hydrate_ms,
                "status": status,
            }))?
        );
        return Ok(());
    }
    if env::var_os("SAGE_INDEX_BENCH_PARSE_ONLY").is_some() {
        let started = Instant::now();
        let paths = collect_indexable_paths(index.options());
        let collect_ms = started.elapsed().as_millis();
        let started = Instant::now();
        let parsed: Vec<_> = paths
            .par_iter()
            .filter_map(|path| parse_file_for_roots(path, &index.options().roots).ok())
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "schema_version": 1,
                "mode": "parse",
                "file_count": paths.len(),
                "parsed_file_count": parsed.len(),
                "symbol_count": parsed.iter().map(|file| file.symbols.len()).sum::<usize>(),
                "doc_count": parsed
                    .iter()
                    .flat_map(|file| &file.symbols)
                    .filter(|symbol| symbol.docstring.as_ref().is_some_and(|doc| !doc.is_empty()))
                    .count(),
                "collect_ms": collect_ms,
                "parse_ms": started.elapsed().as_millis(),
            }))?
        );
        return Ok(());
    }
    let started = Instant::now();
    let status = index.rebuild().context("rebuild index")?;
    let elapsed_ms = started.elapsed().as_millis();
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "schema_version": 1,
            "mode": "rebuild",
            "elapsed_ms": elapsed_ms,
            "status": status,
        }))?
    );
    Ok(())
}
