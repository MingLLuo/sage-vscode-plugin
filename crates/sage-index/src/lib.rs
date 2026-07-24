use anyhow::{bail, Context, Result};
use ignore::WalkBuilder;
use rayon::prelude::*;
use regex::Regex;
use rusqlite::{params, params_from_iter, Connection, OptionalExtension};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, UNIX_EPOCH};
use tree_sitter::Parser;

mod cache_metadata;
mod cache_persistence;
mod cache_queries;
mod identifier_filter;
mod lookup_state;
mod materialized_cache;
mod model;
mod query_support;
mod sage_specs;
mod source_analysis;
mod source_paths;
mod symbol_resolution;
mod symbol_support;
mod syntax_support;
mod workspace_lifecycle;
mod workspace_queries;

use cache_metadata::*;
use cache_persistence::*;
use cache_queries::*;
use identifier_filter::*;
use materialized_cache::*;
pub use model::*;
use query_support::*;
pub use query_support::{
    function_call_at_position, local_import_alias_symbol_from_source,
    local_import_alias_symbol_from_source_name, local_import_alias_symbol_from_symbols,
    sage_prewarm_modules_for_source,
};
use sage_specs::{
    SAGE_EXPORT_MAP, SAGE_METHOD_ALIAS_SPECS, SAGE_METHOD_SPECS, SAGE_OWNER_METHOD_MODULES,
};
use source_analysis::{
    dedupe_reference_records, diagnostics_for_source, is_sage_source_path, line_offsets,
    reference_spans_in_source, sage_load_attach_paths_before_line,
    scope_references_for_resolved_symbol, source_aliased_import_at_range,
    source_import_from_at_range, source_imported_sage_all_star_lookup, SourceImportLookup,
};
use source_paths::*;
use symbol_support::*;
use syntax_support::*;

pub use source_analysis::{
    collect_indexable_paths, is_code_reference_at_range, parse_file_for_roots, parse_source,
    preprocess_sage_source, references_in_source, semantic_spans, CodeReferenceMap,
};

pub fn source_definition_header_end(source: &str, offset: usize) -> Option<usize> {
    syntax_support::definition_header_end(source, offset)
}

const CACHE_FORMAT_VERSION: &str = "sage-index-v28-compatible-identifier-filter";
const MAX_IMPORT_RESOLUTION_DEPTH: usize = 8;
const MAX_DYNAMIC_HOT_EXPORT_NAMES: usize = 256;
const SAGE_STAR_IMPORT_SENTINEL: &str = "__sage_star_import__";
const SAGE_ALL_EXPORT_SENTINEL: &str = "__sage_all_export__";
const SAGE_ALL_EXPORT_MARKER: &str = "__all__::*";
const METHOD_CACHE_ORIGIN_SOURCE_DERIVED: &str = "source-derived";
const METHOD_CACHE_ORIGIN_STATIC_SPEC: &str = "static-spec";
const METHOD_CACHE_ORIGIN_STATIC_ALIAS: &str = "static-alias";

pub fn default_cache_dir() -> PathBuf {
    static DEFAULT_CACHE_DIR: OnceLock<PathBuf> = OnceLock::new();
    DEFAULT_CACHE_DIR
        .get_or_init(resolve_default_cache_dir)
        .clone()
}

fn resolve_default_cache_dir() -> PathBuf {
    let preferred = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))
        .unwrap_or_else(std::env::temp_dir)
        .join("sage-vscode-plugin")
        .join("rust-index-v2");
    if cache_dir_is_usable(&preferred) {
        preferred
    } else {
        fallback_cache_dir()
    }
}

fn fallback_cache_dir() -> PathBuf {
    std::env::temp_dir()
        .join("sage-vscode-plugin")
        .join("rust-index-v2")
}

fn cache_dir_is_usable(path: &Path) -> bool {
    if fs::create_dir_all(path).is_err() {
        return false;
    }
    let probe = path.join(".write-test");
    match fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&probe)
    {
        Ok(_) => {
            let _ = fs::remove_file(probe);
            true
        }
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests;
