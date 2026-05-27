use anyhow::{Context, Result};
use ignore::WalkBuilder;
use rayon::prelude::*;
use regex::Regex;
use rusqlite::{params, params_from_iter, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, UNIX_EPOCH};
use tree_sitter::Parser;

const CACHE_FORMAT_VERSION: &str = "sage-index-v25-canonical-method-priority";
const MAX_IMPORT_RESOLUTION_DEPTH: usize = 8;
const MAX_DYNAMIC_HOT_EXPORT_NAMES: usize = 256;
const SAGE_STAR_IMPORT_SENTINEL: &str = "__sage_star_import__";
const SAGE_ALL_EXPORT_SENTINEL: &str = "__sage_all_export__";
const SAGE_ALL_EXPORT_MARKER: &str = "__all__::*";
const METHOD_CACHE_ORIGIN_SOURCE_DERIVED: &str = "source-derived";
const METHOD_CACHE_ORIGIN_STATIC_SPEC: &str = "static-spec";
const METHOD_CACHE_ORIGIN_STATIC_ALIAS: &str = "static-alias";

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct IndexOptions {
    pub roots: Vec<PathBuf>,
    pub editable_roots: Vec<PathBuf>,
    pub exclude_globs: Vec<String>,
    pub cache_dir: PathBuf,
    pub enable_pyx: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct IndexStatus {
    pub roots: Vec<String>,
    pub loaded_roots: Vec<String>,
    pub editable_roots: Vec<String>,
    pub cache_namespace: String,
    pub source_root_fingerprints: Vec<SourceRootFingerprint>,
    pub cache_stale: bool,
    pub stale_source_roots: Vec<StaleSourceRootFingerprint>,
    pub indexed_file_count: usize,
    pub deferred_file_count: usize,
    pub symbol_count: usize,
    pub doc_count: usize,
    pub generation: u64,
    pub cache_path: String,
    pub last_index_ms: u128,
    pub last_operation: Option<String>,
    pub last_hydrate_ms: u128,
    pub last_reconcile_ms: u128,
    pub last_persist_ms: u128,
    pub last_hot_cache_ms: u128,
    pub last_peer_seed_ms: u128,
    pub peer_seed_file_count: usize,
    pub sage_method_cache_count: usize,
    pub source_derived_method_cache_count: usize,
    pub static_method_cache_count: usize,
    pub cache_hit_count: usize,
    pub cache_miss_count: usize,
    pub hot_symbol_cache_count: usize,
    pub pending_jobs: usize,
    pub last_error: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct SourceRootFingerprint {
    pub root: String,
    pub exists: bool,
    pub digest: String,
    pub marker: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct StaleSourceRootFingerprint {
    pub root: String,
    pub cached_digest: String,
    pub current_digest: String,
    pub cached_marker: Option<String>,
    pub current_marker: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DocsStatus {
    pub doc_db_path: String,
    pub offline_doc_count: usize,
    pub preferred_source: String,
    pub runtime_worker_state: String,
    pub runtime_degraded_reason: Option<String>,
    pub runtime_queue_depth: usize,
    pub runtime_timeout_count: usize,
    pub runtime_cache_hits: usize,
    pub runtime_cache_misses: usize,
}

#[derive(Clone, Debug, Default)]
struct SageMethodCacheStats {
    total: usize,
    source_derived: usize,
    static_fallback: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum SymbolKind {
    Module,
    Class,
    Function,
    Variable,
    Import,
    CythonDeclaration,
    PreparserGenerator,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct SourceRange {
    pub start_line: u32,
    pub start_character: u32,
    pub end_line: u32,
    pub end_character: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SymbolRecord {
    pub name: String,
    pub kind: SymbolKind,
    pub module: String,
    pub path: PathBuf,
    pub range: SourceRange,
    pub detail: String,
    pub docstring: Option<String>,
    pub import_from: Option<String>,
    pub signature: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct IndexedFile {
    pub module: String,
    pub path: PathBuf,
    pub symbols: Vec<SymbolRecord>,
    pub module_docstring: Option<String>,
}

type SymbolLookupCache = Arc<Mutex<HashMap<String, Vec<SymbolRecord>>>>;
type FileLookupCache = Arc<Mutex<HashMap<PathBuf, IndexedFile>>>;
type SageMethodLookupCache = Arc<Mutex<HashMap<(String, String), Option<SymbolRecord>>>>;
type ReferenceLookupCache = Arc<Mutex<HashMap<String, Vec<ReferenceRecord>>>>;

#[derive(Clone, Debug, Default)]
pub struct WorkspaceIndex {
    options: IndexOptions,
    db_path: PathBuf,
    files: BTreeMap<PathBuf, IndexedFile>,
    symbols_by_name: HashMap<String, Vec<SymbolRecord>>,
    generation: u64,
    last_index_ms: u128,
    last_operation: Option<String>,
    last_hydrate_ms: u128,
    last_reconcile_ms: u128,
    last_persist_ms: u128,
    last_hot_cache_ms: u128,
    last_peer_seed_ms: u128,
    peer_seed_file_count: usize,
    cache_hit_count: usize,
    cache_miss_count: usize,
    loaded_roots: Vec<PathBuf>,
    last_error: Option<String>,
    cached_file_count: usize,
    cached_symbol_count: usize,
    cached_doc_count: usize,
    source_root_fingerprints: Vec<SourceRootFingerprint>,
    cached_root_fingerprint_mismatches: Vec<StaleSourceRootFingerprint>,
    symbol_lookup_cache: SymbolLookupCache,
    file_lookup_cache: FileLookupCache,
    sage_method_lookup_cache: SageMethodLookupCache,
    reference_lookup_cache: ReferenceLookupCache,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum SageOwnerType {
    MatrixConstructor,
    Matrix,
    FreeModule,
    PolynomialRing,
    PolynomialElement,
    Ideal,
    Field,
    FieldElement,
    Vector,
    Graph,
    EllipticCurve,
    NumberField,
}

impl SageOwnerType {
    fn as_str(self) -> &'static str {
        match self {
            Self::MatrixConstructor => "MatrixConstructor",
            Self::Matrix => "Matrix",
            Self::FreeModule => "FreeModule",
            Self::PolynomialRing => "PolynomialRing",
            Self::PolynomialElement => "PolynomialElement",
            Self::Ideal => "Ideal",
            Self::Field => "Field",
            Self::FieldElement => "FieldElement",
            Self::Vector => "Vector",
            Self::Graph => "Graph",
            Self::EllipticCurve => "EllipticCurve",
            Self::NumberField => "NumberField",
        }
    }
}

fn sage_method_cache_key(owner_type: SageOwnerType, member: &str) -> (String, String) {
    (owner_type.as_str().to_string(), member.to_ascii_lowercase())
}

#[derive(Clone, Debug)]
struct MemberResolution {
    record: Option<SymbolRecord>,
    owner_type: Option<SageOwnerType>,
    confidence: &'static str,
    reason: String,
    candidate_count: usize,
    suppress_global_fallback: bool,
}

#[derive(Clone, Debug)]
struct SageExportResolution {
    record: SymbolRecord,
    reason: &'static str,
}

#[derive(Clone, Copy, Debug)]
struct SourceDerivedMethodOwner {
    owner_type: SageOwnerType,
    priority: u8,
}

#[derive(Clone, Copy, Debug)]
struct SageMethodSpec {
    owner_type: SageOwnerType,
    member: &'static str,
    module: &'static str,
}

#[derive(Clone, Copy, Debug)]
struct SageMethodAliasSpec {
    owner_type: SageOwnerType,
    member: &'static str,
    source_name: &'static str,
    module: &'static str,
}

#[derive(Clone, Copy, Debug)]
struct SageOwnerModuleSpec {
    owner_type: SageOwnerType,
    module: &'static str,
    recursive: bool,
    priority: u8,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreprocessEdit {
    pub line: u32,
    pub source_character: u32,
    pub generated_character: u32,
    pub source_text: String,
    pub generated_text: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreprocessResult {
    pub generated: String,
    pub edits: Vec<PreprocessEdit>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SemanticSpan {
    pub line: u32,
    pub start: u32,
    pub length: u32,
    pub token_type: String,
    pub modifiers: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DiagnosticRecord {
    pub message: String,
    pub range: SourceRange,
    pub code: String,
    #[serde(default = "default_diagnostic_severity")]
    pub severity: String,
}

fn default_diagnostic_severity() -> String {
    "error".to_string()
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReferenceRecord {
    pub path: PathBuf,
    pub range: SourceRange,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub struct QueryPosition {
    pub line: u32,
    pub character: u32,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct QueryTarget {
    pub symbol: String,
    pub dotted_symbol: Option<String>,
    pub range: SourceRange,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DocumentationRecord {
    pub name: String,
    pub module_name: String,
    pub kind: String,
    pub detail: String,
    pub summary: String,
    pub docstring: Option<String>,
    pub uri: Option<String>,
    pub markers: Vec<String>,
    pub sections: Vec<DocumentationSection>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DocumentationSection {
    pub title: String,
    pub body: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct QueryHover {
    pub markdown: String,
    pub range: SourceRange,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct QueryDefinition {
    pub name: String,
    pub path: PathBuf,
    pub range: SourceRange,
    pub detail: String,
    pub module: String,
}

fn query_definition_from_record(record: &SymbolRecord) -> Option<QueryDefinition> {
    if record.path.as_os_str().is_empty() {
        return None;
    }
    Some(QueryDefinition {
        name: record.name.clone(),
        path: record.path.clone(),
        range: record.range.clone(),
        detail: record.detail.clone(),
        module: record.module.clone(),
    })
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct QueryCompletion {
    pub label: String,
    pub kind: String,
    pub detail: String,
    pub signature: Option<String>,
    pub documentation: Option<String>,
    pub resolve_name: Option<String>,
    pub module: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct QueryTextEdit {
    pub path: PathBuf,
    pub range: SourceRange,
    pub new_text: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct QuerySignature {
    pub label: String,
    pub active_parameter: u32,
    pub documentation: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct QueryResult {
    pub target: Option<QueryTarget>,
    pub hover: Option<QueryHover>,
    pub documentation: Option<DocumentationRecord>,
    pub definition: Option<QueryDefinition>,
    pub completions: Vec<QueryCompletion>,
    pub references: Vec<ReferenceRecord>,
    pub rename_preview: Vec<QueryTextEdit>,
    pub signature: Option<QuerySignature>,
    pub diagnostics: Vec<DiagnosticRecord>,
    pub fallback_reason: Option<String>,
    #[serde(rename = "resolutionConfidence")]
    pub resolution_confidence: Option<String>,
    #[serde(rename = "resolutionReason")]
    pub resolution_reason: Option<String>,
    #[serde(rename = "ownerType")]
    pub owner_type: Option<String>,
    #[serde(rename = "candidateCount")]
    pub candidate_count: usize,
}

#[derive(Clone, Copy, Debug)]
pub struct QueryFeatures {
    pub completions: bool,
    pub references: bool,
    pub rename_preview: bool,
    pub signature: bool,
    pub diagnostics: bool,
}

impl QueryFeatures {
    pub const fn full() -> Self {
        Self {
            completions: true,
            references: true,
            rename_preview: true,
            signature: true,
            diagnostics: true,
        }
    }

    pub const fn navigation() -> Self {
        Self {
            completions: false,
            references: false,
            rename_preview: false,
            signature: true,
            diagnostics: false,
        }
    }

    pub const fn hover() -> Self {
        Self {
            completions: false,
            references: false,
            rename_preview: false,
            signature: false,
            diagnostics: false,
        }
    }
}

#[derive(Clone, Debug)]
pub struct QueryExecutionOptions<'a> {
    pub rename_to: Option<&'a str>,
    pub diagnostics: Vec<DiagnosticRecord>,
    pub features: QueryFeatures,
}

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

impl WorkspaceIndex {
    pub fn new(mut options: IndexOptions) -> Self {
        options.roots = normalize_existing_paths(options.roots);
        options.editable_roots = normalize_existing_paths(options.editable_roots);
        let digest =
            cache_namespace_digest(&options.roots, &options.exclude_globs, options.enable_pyx);
        let db_path = options
            .cache_dir
            .join(format!("sage-index-{digest}.sqlite"));
        Self {
            options,
            db_path,
            ..Self::default()
        }
    }

    pub fn options(&self) -> &IndexOptions {
        &self.options
    }

    pub fn clone_for_background_work(&self) -> Self {
        let mut clone = self.clone();
        clone.symbol_lookup_cache = Arc::new(Mutex::new(
            self.symbol_lookup_cache
                .lock()
                .map(|cache| cache.clone())
                .unwrap_or_default(),
        ));
        clone.file_lookup_cache = Arc::new(Mutex::new(
            self.file_lookup_cache
                .lock()
                .map(|cache| cache.clone())
                .unwrap_or_default(),
        ));
        clone.sage_method_lookup_cache = Arc::new(Mutex::new(
            self.sage_method_lookup_cache
                .lock()
                .map(|cache| cache.clone())
                .unwrap_or_default(),
        ));
        clone.reference_lookup_cache = Arc::new(Mutex::new(
            self.reference_lookup_cache
                .lock()
                .map(|cache| cache.clone())
                .unwrap_or_default(),
        ));
        clone
    }

    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

    pub fn rebuild(&mut self) -> Result<IndexStatus> {
        let started = Instant::now();
        self.reset_operation_timings("rebuild");
        self.ensure_cache_dir()?;
        let paths = collect_indexable_paths(&self.options);
        let parsed: Vec<IndexedFile> = paths
            .par_iter()
            .filter_map(|path| parse_file_for_roots(path, &self.options.roots).ok())
            .collect();

        let mut files = BTreeMap::new();
        let mut symbols_by_name: HashMap<String, Vec<SymbolRecord>> = HashMap::new();
        for file in parsed {
            for symbol in &file.symbols {
                symbols_by_name
                    .entry(symbol.name.to_ascii_lowercase())
                    .or_default()
                    .push(symbol.clone());
            }
            files.insert(file.path.clone(), file);
        }

        self.files = files;
        self.symbols_by_name = symbols_by_name;
        self.generation = self.generation.saturating_add(1);
        self.last_index_ms = started.elapsed().as_millis();
        self.loaded_roots = self.options.roots.clone();
        self.cached_file_count = 0;
        self.cached_symbol_count = 0;
        self.cached_doc_count = 0;
        self.cached_root_fingerprint_mismatches.clear();
        self.clear_lookup_cache();
        self.last_error = None;
        let persist_started = Instant::now();
        if let Err(error) = self.persist_all() {
            let primary_error = error.to_string();
            if let Err(fallback_error) = self.persist_all_with_fallback() {
                self.last_error = Some(format!(
                    "{primary_error}; fallback cache failed: {fallback_error}"
                ));
            }
        }
        self.last_persist_ms = persist_started.elapsed().as_millis();
        Ok(self.status())
    }

    pub fn reconcile_with_cache(&mut self) -> Result<IndexStatus> {
        let started = Instant::now();
        self.reset_operation_timings("reconcile");
        self.ensure_cache_dir()?;
        self.seed_shared_roots_from_peer_caches();
        if self.cached_file_count > 0 && self.db_path.exists() {
            if let Ok(connection) = Connection::open(&self.db_path) {
                if let Ok(Some((file_count, symbol_count, doc_count))) =
                    load_cached_counts_from_metadata(&connection, &self.options.roots)
                {
                    let mismatches = load_root_fingerprint_mismatches_from_metadata(
                        &connection,
                        &self.options.roots,
                    )
                    .unwrap_or_default();
                    if mismatches.is_empty() && file_count > 0 {
                        self.files.clear();
                        self.symbols_by_name.clear();
                        self.clear_lookup_cache();
                        self.loaded_roots = self.options.roots.clone();
                        self.cached_file_count = file_count;
                        self.cached_symbol_count = symbol_count;
                        self.cached_doc_count = doc_count;
                        self.cached_root_fingerprint_mismatches.clear();
                        self.cache_hit_count = self.cache_hit_count.saturating_add(file_count);
                        self.last_persist_ms = 0;
                        let hot_started = Instant::now();
                        self.prewarm_hot_symbol_cache(false);
                        self.last_hot_cache_ms = hot_started.elapsed().as_millis();
                        self.last_reconcile_ms = started.elapsed().as_millis();
                        self.last_index_ms = self.last_reconcile_ms;
                        self.last_operation = Some("fast-reconcile".to_string());
                        self.generation = self.generation.saturating_add(1);
                        self.last_error = None;
                        return Ok(self.status());
                    }
                    self.cached_root_fingerprint_mismatches = mismatches;
                }
            }
        }
        let current_paths = collect_indexable_paths(&self.options);
        let current_path_set: BTreeSet<PathBuf> = current_paths.iter().cloned().collect();
        let cached_fingerprints = self.load_cached_fingerprints_for_current_roots()?;
        let mut unchanged_count = 0usize;
        let mut changed_paths = Vec::new();

        for path in &current_paths {
            let current_fingerprint = match file_fingerprint(path) {
                Ok(fingerprint) => fingerprint,
                Err(_) => {
                    changed_paths.push(path.clone());
                    continue;
                }
            };
            match cached_fingerprints.get(path) {
                Some(cached_fingerprint) if cached_fingerprint == &current_fingerprint => {
                    unchanged_count = unchanged_count.saturating_add(1);
                }
                _ => changed_paths.push(path.clone()),
            }
        }

        let deleted_paths: Vec<PathBuf> = cached_fingerprints
            .keys()
            .filter(|path| !current_path_set.contains(*path))
            .cloned()
            .collect();

        let changed_files: Vec<IndexedFile> = changed_paths
            .par_iter()
            .filter_map(|path| parse_file_for_roots(path, &self.options.roots).ok())
            .collect();

        self.files.clear();
        self.symbols_by_name.clear();
        self.clear_lookup_cache();
        self.loaded_roots = self.options.roots.clone();
        self.last_index_ms = started.elapsed().as_millis();
        self.generation = self.generation.saturating_add(1);
        self.last_error = None;

        let persist_started = Instant::now();
        let mut refresh_materialized = false;
        if changed_files.is_empty() && deleted_paths.is_empty() {
            self.last_persist_ms = 0;
        } else {
            let materialize_from_changed = deleted_paths.is_empty()
                && !changed_files.is_empty()
                && changed_files.len() == current_paths.len();
            refresh_materialized = materialize_from_changed
                || paths_need_materialized_cache_refresh(
                    &changed_files,
                    &deleted_paths,
                    &self.options.roots,
                );
            if let Err(error) = self.persist_paths(
                &changed_files,
                &deleted_paths,
                materialize_from_changed,
                refresh_materialized,
            ) {
                let primary_error = error.to_string();
                if let Err(fallback_error) = self.persist_paths_with_fallback(
                    &changed_files,
                    &deleted_paths,
                    materialize_from_changed,
                    refresh_materialized,
                ) {
                    self.last_error = Some(format!(
                        "{primary_error}; fallback cache failed: {fallback_error}"
                    ));
                }
            }
            self.last_persist_ms = persist_started.elapsed().as_millis();
        }

        if let Ok((file_count, symbol_count, doc_count)) = self.cached_counts_for_current_roots() {
            self.cached_file_count = file_count;
            self.cached_symbol_count = symbol_count;
            self.cached_doc_count = doc_count;
        }
        self.cached_root_fingerprint_mismatches.clear();
        self.cache_hit_count = self.cache_hit_count.saturating_add(unchanged_count);
        self.cache_miss_count = self
            .cache_miss_count
            .saturating_add(changed_paths.len().saturating_add(deleted_paths.len()));
        let hot_started = Instant::now();
        self.prewarm_hot_symbol_cache(refresh_materialized);
        self.last_hot_cache_ms = hot_started.elapsed().as_millis();
        self.last_reconcile_ms = started.elapsed().as_millis();
        Ok(self.status())
    }

    pub fn hydrate_from_cache(&mut self) -> Result<IndexStatus> {
        let started = Instant::now();
        self.reset_operation_timings("hydrate");
        self.ensure_cache_dir()?;
        if !self.db_path.exists() {
            self.cache_miss_count = self.cache_miss_count.saturating_add(1);
            self.last_index_ms = started.elapsed().as_millis();
            self.last_hydrate_ms = self.last_index_ms;
            return Ok(self.status());
        }
        let connection = match Connection::open(&self.db_path) {
            Ok(connection) => connection,
            Err(error) => {
                if self.switch_to_fallback_cache().is_ok() && self.db_path.exists() {
                    if let Ok(connection) = Connection::open(&self.db_path) {
                        return self.hydrate_from_connection(started, connection);
                    }
                }
                self.cache_miss_count = self.cache_miss_count.saturating_add(1);
                self.last_error = Some(error.to_string());
                self.last_operation = Some("hydrate".to_string());
                self.last_hydrate_ms = started.elapsed().as_millis();
                self.last_index_ms = self.last_hydrate_ms;
                return Ok(self.status());
            }
        };
        self.hydrate_from_connection(started, connection)
    }

    fn hydrate_from_connection(
        &mut self,
        started: Instant,
        connection: Connection,
    ) -> Result<IndexStatus> {
        let (file_count, symbol_count, doc_count) =
            match cached_counts_for_roots(&connection, &self.options.roots) {
                Ok(counts) => counts,
                Err(error) => {
                    self.cache_miss_count = self.cache_miss_count.saturating_add(1);
                    self.last_error = Some(error.to_string());
                    self.last_hydrate_ms = started.elapsed().as_millis();
                    self.last_index_ms = self.last_hydrate_ms;
                    return Ok(self.status());
                }
            };
        if file_count == 0 {
            self.cache_miss_count = self.cache_miss_count.saturating_add(1);
        } else {
            self.cache_hit_count = self.cache_hit_count.saturating_add(file_count);
        }
        self.files.clear();
        self.symbols_by_name.clear();
        self.clear_lookup_cache();
        self.cached_file_count = file_count;
        self.cached_symbol_count = symbol_count;
        self.cached_doc_count = doc_count;
        let (source_root_fingerprints, stale_source_roots) =
            load_root_fingerprint_status_from_metadata(&connection, &self.options.roots)
                .unwrap_or_else(|_| {
                    (
                        source_root_fingerprints_for_roots(&self.options.roots),
                        Vec::new(),
                    )
                });
        self.source_root_fingerprints = source_root_fingerprints;
        self.cached_root_fingerprint_mismatches = stale_source_roots;
        self.loaded_roots = self.options.roots.clone();
        self.last_hydrate_ms = started.elapsed().as_millis();
        self.last_index_ms = self.last_hydrate_ms;
        Ok(self.status())
    }

    pub fn refresh_paths(
        &mut self,
        changed: &[PathBuf],
        deleted: &[PathBuf],
    ) -> Result<IndexStatus> {
        let started = Instant::now();
        self.reset_operation_timings("refresh");
        let mut changed_files = Vec::new();
        let mut dirty_lookup_names = BTreeSet::new();
        let changed = normalize_existing_paths(changed.to_vec());
        let deleted = normalize_paths(deleted.to_vec());
        let deleted_set: BTreeSet<_> = deleted.iter().cloned().collect();
        for path in &deleted {
            if let Some(file) = self.files.remove(path) {
                insert_file_symbol_names(&mut dirty_lookup_names, &file);
            }
        }
        for path in &changed {
            if deleted_set.contains(path)
                || !path.exists()
                || !is_indexable(path, self.options.enable_pyx)
                || is_excluded(path, &self.options.exclude_globs)
            {
                if let Some(file) = self.files.remove(path) {
                    insert_file_symbol_names(&mut dirty_lookup_names, &file);
                }
                continue;
            }
            match parse_file_for_roots(path, &self.options.roots) {
                Ok(file) => {
                    if let Some(previous) = self.files.insert(file.path.clone(), file.clone()) {
                        insert_file_symbol_names(&mut dirty_lookup_names, &previous);
                    }
                    insert_file_symbol_names(&mut dirty_lookup_names, &file);
                    changed_files.push(file);
                }
                Err(error) => {
                    self.last_error = Some(error.to_string());
                }
            }
        }
        let refresh_materialized =
            paths_need_materialized_cache_refresh(&changed_files, &deleted, &self.options.roots);
        self.rebuild_symbol_map();
        if refresh_materialized {
            self.clear_lookup_cache();
        } else {
            self.clear_lookup_cache_entries(&dirty_lookup_names);
        }
        self.generation = self.generation.saturating_add(1);
        self.last_index_ms = started.elapsed().as_millis();
        self.ensure_cache_dir()?;
        let persist_started = Instant::now();
        if let Err(error) =
            self.persist_paths(&changed_files, &deleted, false, refresh_materialized)
        {
            let primary_error = error.to_string();
            if let Err(fallback_error) = self.persist_paths_with_fallback(
                &changed_files,
                &deleted,
                false,
                refresh_materialized,
            ) {
                self.last_error = Some(format!(
                    "{primary_error}; fallback cache failed: {fallback_error}"
                ));
            }
        }
        self.last_persist_ms = persist_started.elapsed().as_millis();
        self.cached_root_fingerprint_mismatches.clear();
        let hot_started = Instant::now();
        if refresh_materialized {
            self.prewarm_hot_symbol_cache(true);
        }
        self.last_hot_cache_ms = hot_started.elapsed().as_millis();
        Ok(self.status())
    }

    pub fn preload_paths(&mut self, paths: &[PathBuf]) -> usize {
        let mut files = Vec::new();
        for path in normalize_existing_paths(paths.to_vec()) {
            if !path_is_under_roots(&path, &self.options.roots)
                || !is_indexable(&path, self.options.enable_pyx)
                || is_excluded(&path, &self.options.exclude_globs)
            {
                continue;
            }
            match parse_file_for_roots(&path, &self.options.roots) {
                Ok(file) => files.push(file),
                Err(error) => {
                    self.last_error = Some(error.to_string());
                }
            }
        }
        self.preload_indexed_files(files)
    }

    pub fn preload_indexed_files(&mut self, files: Vec<IndexedFile>) -> usize {
        let mut loaded = 0;
        let mut dirty_lookup_names = BTreeSet::new();
        for file in files {
            if !path_is_under_roots(&file.path, &self.options.roots)
                || is_excluded(&file.path, &self.options.exclude_globs)
            {
                continue;
            }
            if let Some(previous) = self.files.insert(file.path.clone(), file.clone()) {
                insert_file_symbol_names(&mut dirty_lookup_names, &previous);
            }
            insert_file_symbol_names(&mut dirty_lookup_names, &file);
            loaded += 1;
        }
        if loaded > 0 {
            self.rebuild_symbol_map();
            self.clear_lookup_cache_entries(&dirty_lookup_names);
        }
        loaded
    }

    pub fn status(&self) -> IndexStatus {
        let method_cache_stats = self.sage_method_cache_stats();
        IndexStatus {
            roots: self
                .options
                .roots
                .iter()
                .map(|root| root.display().to_string())
                .collect(),
            loaded_roots: self
                .loaded_roots
                .iter()
                .map(|root| root.display().to_string())
                .collect(),
            editable_roots: self
                .effective_editable_roots()
                .iter()
                .map(|root| root.display().to_string())
                .collect(),
            cache_namespace: cache_namespace_digest(
                &self.options.roots,
                &self.options.exclude_globs,
                self.options.enable_pyx,
            ),
            source_root_fingerprints: if self.source_root_fingerprints.len()
                == self.options.roots.len()
            {
                self.source_root_fingerprints.clone()
            } else {
                source_root_fingerprints_for_roots(&self.options.roots)
            },
            cache_stale: !self.cached_root_fingerprint_mismatches.is_empty(),
            stale_source_roots: self.cached_root_fingerprint_mismatches.clone(),
            indexed_file_count: self.cached_file_count.max(self.files.len()),
            deferred_file_count: 0,
            symbol_count: self
                .cached_symbol_count
                .max(self.symbols_by_name.values().map(Vec::len).sum()),
            doc_count: self.cached_doc_count.max(
                self.files
                    .values()
                    .flat_map(|file| &file.symbols)
                    .filter(|symbol| symbol.docstring.as_ref().is_some_and(|doc| !doc.is_empty()))
                    .count(),
            ),
            generation: self.generation,
            cache_path: self.db_path.display().to_string(),
            last_index_ms: self.last_index_ms,
            last_operation: self.last_operation.clone(),
            last_hydrate_ms: self.last_hydrate_ms,
            last_reconcile_ms: self.last_reconcile_ms,
            last_persist_ms: self.last_persist_ms,
            last_hot_cache_ms: self.last_hot_cache_ms,
            last_peer_seed_ms: self.last_peer_seed_ms,
            peer_seed_file_count: self.peer_seed_file_count,
            sage_method_cache_count: method_cache_stats.total,
            source_derived_method_cache_count: method_cache_stats.source_derived,
            static_method_cache_count: method_cache_stats.static_fallback,
            cache_hit_count: self.cache_hit_count,
            cache_miss_count: self.cache_miss_count,
            hot_symbol_cache_count: self.lookup_cache_len(),
            pending_jobs: 0,
            last_error: self.last_error.clone(),
        }
    }

    pub fn docs_status(&self) -> DocsStatus {
        DocsStatus {
            doc_db_path: self.db_path.display().to_string(),
            offline_doc_count: self.status().doc_count,
            preferred_source: "auto".to_string(),
            runtime_worker_state: "static-fallback".to_string(),
            runtime_degraded_reason: Some(
                "persistent Sage runtime docs worker is not enabled in Rust V2; static index and known Sage fallback are active".to_string(),
            ),
            runtime_queue_depth: 0,
            runtime_timeout_count: 0,
            runtime_cache_hits: 0,
            runtime_cache_misses: 0,
        }
    }

    pub fn symbols_with_prefix(&self, prefix: &str, limit: usize) -> Vec<SymbolRecord> {
        let needle = prefix.to_ascii_lowercase();
        let mut results = if self.cached_symbol_count > 0 {
            load_symbols_with_prefix_from_db(&self.db_path, prefix, limit, &self.options.roots)
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        results.extend(
            self.symbols_by_name
                .iter()
                .filter(|(name, _)| needle.is_empty() || name.starts_with(&needle))
                .filter_map(|(_, symbols)| best_symbol(symbols.clone())),
        );
        dedupe_best_symbols(results, limit)
    }

    pub fn completion_items_at_source(
        &self,
        source: &str,
        position: QueryPosition,
        limit: usize,
    ) -> Vec<QueryCompletion> {
        self.completion_items_at_source_with_fallback(source, position, limit, None)
    }

    fn completion_items_at_source_with_fallback(
        &self,
        source: &str,
        position: QueryPosition,
        limit: usize,
        fallback_prefix: Option<&str>,
    ) -> Vec<QueryCompletion> {
        if limit == 0 || !is_code_completion_position(source, position) {
            return Vec::new();
        }
        if let Some(context) = member_completion_context(source, position) {
            if let Some(owner_type) =
                infer_completion_owner_type(source, &context.owner, position.line)
            {
                let completions =
                    self.known_sage_method_completions(owner_type, &context.prefix, limit);
                if !completions.is_empty() {
                    return completions;
                }
            }
        }
        let prefix = current_prefix(source, position.line, position.character).unwrap_or_default();
        let prefix = if prefix.is_empty() {
            fallback_prefix.unwrap_or("")
        } else {
            prefix.as_str()
        };
        let mut results = local_completion_items(source, position, prefix, limit);
        let mut seen: BTreeSet<String> = results
            .iter()
            .map(|completion| completion.label.to_ascii_lowercase())
            .collect();
        for record in self.symbols_with_prefix(prefix, limit) {
            if results.len() >= limit {
                break;
            }
            if seen.insert(record.name.to_ascii_lowercase()) {
                results.push(completion_from_symbol(record));
            }
        }
        results
    }

    pub fn workspace_symbols(&self, query: &str, limit: usize) -> Vec<SymbolRecord> {
        let needle = query.to_ascii_lowercase();
        if limit == 0 {
            return Vec::new();
        }
        if is_valid_identifier(query) {
            let mut exact = self.symbol_candidates_without_docs(query);
            if exact.is_empty() {
                if let Some(resolution) = self.resolve_sage_exported_symbol(query) {
                    exact.push(resolution.record);
                }
            }
            if !exact.is_empty() {
                exact = suppress_workspace_import_noise(exact);
                exact.sort_by(|left, right| {
                    workspace_symbol_sort_key(left, &needle)
                        .cmp(&workspace_symbol_sort_key(right, &needle))
                        .then(left.name.cmp(&right.name))
                        .then(left.module.cmp(&right.module))
                });
                exact.truncate(limit);
                return exact;
            }
        }
        let fetch_limit = limit.saturating_mul(12).max(limit).max(200);
        let mut results = if self.cached_symbol_count > 0 {
            load_workspace_symbols_from_db(&self.db_path, query, fetch_limit, &self.options.roots)
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        results.extend(
            self.symbols_by_name
                .iter()
                .filter(|(name, symbols)| {
                    needle.is_empty()
                        || name.contains(&needle)
                        || symbols.first().is_some_and(|symbol| {
                            symbol.module.to_ascii_lowercase().contains(&needle)
                        })
                })
                .flat_map(|(_, symbols)| symbols.clone()),
        );
        let mut results = dedupe_symbol_records(results);
        results = suppress_workspace_import_noise(results);
        results.sort_by(|left, right| {
            workspace_symbol_sort_key(left, &needle)
                .cmp(&workspace_symbol_sort_key(right, &needle))
                .then(left.name.cmp(&right.name))
                .then(left.module.cmp(&right.module))
        });
        results.truncate(limit);
        results
    }

    pub fn symbol(&self, name: &str) -> Option<SymbolRecord> {
        best_symbol(self.symbol_candidates(name))
    }

    pub fn resolve_symbol(&self, name: &str, module_hint: Option<&str>) -> Option<SymbolRecord> {
        let candidates = self.symbol_candidates(name);
        if candidates.is_empty() {
            return None;
        }
        let resolved = resolve_from_candidates(module_hint, candidates)?;
        if resolved.kind == SymbolKind::Import {
            self.resolve_import_record(&resolved).or(Some(resolved))
        } else {
            Some(resolved)
        }
    }

    fn resolve_sage_exported_symbol(&self, name: &str) -> Option<SageExportResolution> {
        self.resolve_sage_exported_symbol_from("sage.all", name)
    }

    fn resolve_sage_exported_symbol_from(
        &self,
        import_module: &str,
        name: &str,
    ) -> Option<SageExportResolution> {
        if let Some(resolution) = self.resolve_hot_sage_export(import_module, name) {
            return Some(resolution);
        }
        if self.cached_symbol_count > 0 || self.db_path.exists() {
            if let Ok(Some(resolution)) = load_materialized_sage_export_from_db(
                &self.db_path,
                import_module,
                name,
                &self.options.roots,
            ) {
                return Some(resolution);
            }
        }
        if let Some(import_symbol) = self
            .symbol_candidates(name)
            .into_iter()
            .filter(|candidate| candidate.kind == SymbolKind::Import)
            .find(|candidate| candidate.module == import_module)
        {
            if let Some(record) = self.resolve_import_record(&import_symbol) {
                return Some(SageExportResolution {
                    record,
                    reason: "indexed sage.all re-export chain",
                });
            }
            return Some(SageExportResolution {
                record: import_symbol,
                reason: "indexed sage.all import binding",
            });
        }
        if module_is_sage_all_export_module(import_module) {
            if let Some(record) =
                self.resolve_module_symbol_from_roots(import_module, name, 0, &mut BTreeSet::new())
            {
                return Some(SageExportResolution {
                    record,
                    reason: "source-derived sage.all export chain",
                });
            }
        }
        if let Some(target) = SAGE_EXPORT_MAP
            .iter()
            .find(|target| target.import_module == import_module && target.name == name)
        {
            if let Some(record) = self
                .symbol_candidates(target.source_name)
                .into_iter()
                .filter(|candidate| {
                    import_target_definition_matches(
                        candidate,
                        target.source_module,
                        target.source_name,
                    )
                })
                .min_by_key(symbol_choice_key)
                .or_else(|| {
                    self.resolve_module_symbol_from_roots(
                        target.source_module,
                        target.source_name,
                        0,
                        &mut BTreeSet::new(),
                    )
                })
            {
                return Some(SageExportResolution {
                    record,
                    reason: "built-in sage.all export fallback",
                });
            }
        }
        None
    }

    fn resolve_hot_sage_export(
        &self,
        import_module: &str,
        name: &str,
    ) -> Option<SageExportResolution> {
        if import_module != "sage.all" {
            return None;
        }
        let target = SAGE_EXPORT_MAP
            .iter()
            .find(|target| target.import_module == "sage.all" && target.name == name)?;
        let key = name.to_ascii_lowercase();
        let symbols = self
            .symbol_lookup_cache
            .lock()
            .ok()
            .and_then(|cache| cache.get(&key).cloned())?;
        let record = best_symbol(
            symbols
                .into_iter()
                .filter(|symbol| {
                    symbol.name == name
                        && symbol.kind != SymbolKind::Import
                        && !module_is_sage_all_export_module(&symbol.module)
                        && path_is_under_roots(&symbol.path, &self.options.roots)
                        && import_target_definition_matches(
                            symbol,
                            target.source_module,
                            target.source_name,
                        )
                })
                .collect(),
        )?;
        Some(SageExportResolution {
            record,
            reason: "materialized sage.all export cache (hot)",
        })
    }

    fn resolve_import_record(&self, symbol: &SymbolRecord) -> Option<SymbolRecord> {
        let mut seen = BTreeSet::new();
        self.resolve_import_record_with_depth(symbol, 0, &mut seen)
    }

    fn resolve_import_record_with_depth(
        &self,
        symbol: &SymbolRecord,
        depth: usize,
        seen: &mut BTreeSet<String>,
    ) -> Option<SymbolRecord> {
        if symbol.kind != SymbolKind::Import || depth >= MAX_IMPORT_RESOLUTION_DEPTH {
            return None;
        }
        let import_from = symbol.import_from.as_ref()?;
        let (source_module, source_name) =
            import_target_in_context(import_from, &symbol.name, &symbol.module);
        if !seen.insert(format!("{source_module}::{source_name}")) {
            return None;
        }

        let candidates = self.symbol_candidates(&source_name);
        if let Some(definition) = candidates
            .iter()
            .filter(|candidate| {
                import_target_definition_matches(candidate, &source_module, &source_name)
            })
            .min_by_key(|candidate| symbol_choice_key(candidate))
            .cloned()
        {
            return Some(definition);
        }

        if let Some(definition) =
            self.resolve_module_symbol_from_roots(&source_module, &source_name, depth + 1, seen)
        {
            return Some(definition);
        }

        let next_import = candidates
            .iter()
            .filter(|candidate| {
                candidate.kind == SymbolKind::Import
                    && candidate.name == source_name
                    && module_matches_import(&candidate.module, &source_module)
            })
            .min_by_key(|candidate| symbol_choice_key(candidate))?;
        self.resolve_import_record_with_depth(next_import, depth + 1, seen)
            .or_else(|| Some(next_import.clone()))
    }

    fn resolve_module_symbol_from_roots(
        &self,
        module: &str,
        name: &str,
        depth: usize,
        seen: &mut BTreeSet<String>,
    ) -> Option<SymbolRecord> {
        self.resolve_module_symbol_from_roots_with_exports(module, name, depth, seen, false)
    }

    fn resolve_module_symbol_from_roots_with_exports(
        &self,
        module: &str,
        name: &str,
        depth: usize,
        seen: &mut BTreeSet<String>,
        require_exported: bool,
    ) -> Option<SymbolRecord> {
        if depth >= MAX_IMPORT_RESOLUTION_DEPTH {
            return None;
        }
        let path =
            module_source_path_from_roots(module, &self.options.roots, self.options.enable_pyx)?;
        let file = parse_file_for_roots(&path, &self.options.roots).ok()?;
        let symbols = file.symbols;
        if require_exported {
            if let Some(exported_names) = explicit_all_names_from_symbols(symbols.iter()) {
                if !exported_names.contains(name) {
                    return None;
                }
            } else if name.starts_with('_') {
                return None;
            }
        }
        let candidates: Vec<_> = symbols
            .iter()
            .filter(|symbol| symbol.name == name)
            .cloned()
            .collect();
        if let Some(definition) = candidates
            .iter()
            .filter(|candidate| import_target_definition_matches(candidate, module, name))
            .min_by_key(|candidate| symbol_choice_key(candidate))
            .cloned()
        {
            return Some(definition);
        }
        for star_import in symbols
            .iter()
            .filter(|symbol| is_star_import_symbol(symbol))
        {
            let import_from = star_import.import_from.as_deref()?;
            let star_module = import_from.strip_suffix("::*").unwrap_or(import_from);
            let star_module = resolve_relative_module(star_module, module);
            if !seen.insert(format!("{star_module}::{name}")) {
                continue;
            }
            if let Some(definition) = self.resolve_module_symbol_from_roots_with_exports(
                &star_module,
                name,
                depth + 1,
                seen,
                true,
            ) {
                return Some(definition);
            }
        }
        let next_import = candidates
            .iter()
            .filter(|candidate| candidate.kind == SymbolKind::Import)
            .min_by_key(|candidate| symbol_choice_key(candidate))?
            .clone();
        self.resolve_import_record_with_depth(&next_import, depth + 1, seen)
            .or(Some(next_import))
    }

    fn symbol_candidates(&self, name: &str) -> Vec<SymbolRecord> {
        let key = name.to_ascii_lowercase();
        if let Ok(cache) = self.symbol_lookup_cache.lock() {
            if let Some(cached) = cache.get(&key) {
                let mut symbols = cached.clone();
                if let Some(memory_symbols) = self.symbols_by_name.get(&key) {
                    symbols.extend(memory_symbols.clone());
                }
                return dedupe_symbol_records(symbols);
            }
        }
        let mut symbols = if self.cached_symbol_count > 0 {
            load_symbols_by_name_from_db(&self.db_path, name, &self.options.roots)
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let persistent_symbols = symbols.clone();
        if let Ok(mut cache) = self.symbol_lookup_cache.lock() {
            cache.insert(key.clone(), persistent_symbols);
        }
        if let Some(memory_symbols) = self.symbols_by_name.get(&key) {
            symbols.extend(memory_symbols.clone());
        }
        dedupe_symbol_records(symbols)
    }

    fn symbol_candidates_without_docs(&self, name: &str) -> Vec<SymbolRecord> {
        let key = name.to_ascii_lowercase();
        if let Ok(cache) = self.symbol_lookup_cache.lock() {
            if let Some(cached) = cache.get(&key) {
                let mut symbols = cached.clone();
                if let Some(memory_symbols) = self.symbols_by_name.get(&key) {
                    symbols.extend(memory_symbols.clone());
                }
                return dedupe_symbol_records(symbols);
            }
        }
        let mut symbols = if self.cached_symbol_count > 0 {
            load_symbols_by_name_from_db_without_docs(&self.db_path, name, &self.options.roots)
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let persistent_symbols = symbols.clone();
        if let Ok(mut cache) = self.symbol_lookup_cache.lock() {
            cache.insert(key.clone(), persistent_symbols);
        }
        if let Some(memory_symbols) = self.symbols_by_name.get(&key) {
            symbols.extend(memory_symbols.clone());
        }
        dedupe_symbol_records(symbols)
    }

    fn resolve_member_symbol(
        &self,
        source: &str,
        owner: &str,
        member: &str,
        module_hint: Option<&str>,
        target_line: u32,
    ) -> MemberResolution {
        let owner_type = infer_owner_type_before(source, owner, member, target_line)
            .or_else(|| infer_owner_type_from_member_hint(member));
        if let Some(owner_type) = owner_type {
            if let Some(record) = self.resolve_known_sage_method_record(owner_type, member) {
                return MemberResolution {
                    record: Some(record),
                    owner_type: Some(owner_type),
                    confidence: "high",
                    reason: format!(
                        "resolved Sage {} method `{}` from owner `{}`",
                        owner_type.as_str(),
                        member,
                        owner
                    ),
                    candidate_count: 1,
                    suppress_global_fallback: true,
                };
            }
        }
        if let Some(owner_resolution) =
            self.resolve_source_derived_namespace_owner(source, owner, module_hint)
        {
            let record = self.resolve_member_in_namespace_owner(&owner_resolution.record, member);
            let found = record.is_some();
            return MemberResolution {
                record,
                owner_type,
                confidence: if found { "high" } else { "ambiguous" },
                reason: format!(
                    "resolved Sage namespace member `{owner}.{member}` through {}",
                    owner_resolution.reason
                ),
                candidate_count: self.symbol_candidates(member).len(),
                suppress_global_fallback: true,
            };
        }
        if is_sage_namespace_owner(owner) {
            let record = self
                .resolve_symbol(member, module_hint)
                .or_else(|| self.resolve_symbol(member, None));
            let confidence = if record.is_some() {
                "high"
            } else {
                "ambiguous"
            };
            return MemberResolution {
                record,
                owner_type,
                confidence,
                reason: format!("resolved Sage namespace member `{owner}.{member}`"),
                candidate_count: self.symbol_candidates(member).len(),
                suppress_global_fallback: true,
            };
        }
        let candidates: Vec<_> = self
            .symbol_candidates(member)
            .into_iter()
            .filter(|candidate| candidate.kind != SymbolKind::Import)
            .collect();
        let candidate_count = candidates.len();
        let Some(constructor) = assignment_constructor_before_line(source, owner, target_line)
        else {
            return MemberResolution {
                record: None,
                owner_type,
                confidence: if owner_type.is_some() {
                    "ambiguous"
                } else {
                    "none"
                },
                reason: if let Some(owner_type) = owner_type {
                    format!(
                        "no static target for Sage {} method `{}`",
                        owner_type.as_str(),
                        member
                    )
                } else if is_known_sage_method(member) {
                    format!("ambiguous Sage method `{member}` without a known owner type")
                } else {
                    format!("no owner type for dotted member `{owner}.{member}`")
                },
                candidate_count,
                suppress_global_fallback: true,
            };
        };
        let constructor_name = constructor.rsplit('.').next().unwrap_or(&constructor);
        let Some(owner_symbol) = self
            .resolve_symbol(constructor_name, module_hint)
            .or_else(|| self.resolve_symbol(constructor_name, None))
        else {
            return MemberResolution {
                record: None,
                owner_type,
                confidence: "ambiguous",
                reason: format!("constructor `{constructor}` for `{owner}` was not indexed"),
                candidate_count,
                suppress_global_fallback: true,
            };
        };
        if candidates.is_empty() {
            return MemberResolution {
                record: None,
                owner_type,
                confidence: "ambiguous",
                reason: format!("no indexed candidates for member `{member}`"),
                candidate_count,
                suppress_global_fallback: true,
            };
        }
        let same_path: Vec<_> = candidates
            .iter()
            .filter(|candidate| {
                !owner_symbol.path.as_os_str().is_empty() && candidate.path == owner_symbol.path
            })
            .cloned()
            .collect();
        if !same_path.is_empty() {
            return MemberResolution {
                record: best_symbol(same_path),
                owner_type,
                confidence: "high",
                reason: format!("member `{member}` matched constructor source path"),
                candidate_count,
                suppress_global_fallback: true,
            };
        }
        let same_module: Vec<_> = candidates
            .iter()
            .filter(|candidate| candidate.module == owner_symbol.module)
            .cloned()
            .collect();
        if !same_module.is_empty() {
            return MemberResolution {
                record: best_symbol(same_module),
                owner_type,
                confidence: "high",
                reason: format!("member `{member}` matched constructor module"),
                candidate_count,
                suppress_global_fallback: true,
            };
        }
        MemberResolution {
            record: None,
            owner_type,
            confidence: "ambiguous",
            reason: format!("member `{member}` did not match constructor module or source path"),
            candidate_count,
            suppress_global_fallback: true,
        }
    }

    fn resolve_loaded_symbol_before_line(
        &self,
        query_path: &Path,
        source: &str,
        name: &str,
        max_line: u32,
    ) -> Option<SymbolRecord> {
        let loaded_paths = sage_load_attach_paths_before_line(query_path, source, max_line);
        for loaded_path in loaded_paths.into_iter().rev() {
            let record = self
                .file_for_path(&loaded_path)
                .or_else(|| self.parse_indexable_file_on_demand(&loaded_path))
                .and_then(|file| {
                    best_symbol(
                        file.symbols
                            .into_iter()
                            .filter(|symbol| {
                                symbol.name == name
                                    && !matches!(
                                        symbol.kind,
                                        SymbolKind::Import | SymbolKind::Module
                                    )
                            })
                            .collect(),
                    )
                });
            if record.is_some() {
                return record;
            }
        }
        None
    }

    fn parse_indexable_file_on_demand(&self, path: &Path) -> Option<IndexedFile> {
        if !is_indexable(path, self.options.enable_pyx)
            || !path_is_under_roots(path, &self.options.roots)
        {
            return None;
        }
        let source = fs::read_to_string(path).ok()?;
        let root = self
            .options
            .roots
            .iter()
            .find(|root| path.strip_prefix(root).is_ok())?;
        let module = module_name_from_path(root, path);
        Some(parse_source(&module, path, &source))
    }

    fn resolve_known_sage_method_record(
        &self,
        owner_type: SageOwnerType,
        member: &str,
    ) -> Option<SymbolRecord> {
        let cache_key = sage_method_cache_key(owner_type, member);
        if let Ok(cache) = self.sage_method_lookup_cache.lock() {
            if let Some(cached) = cache.get(&cache_key) {
                return cached.clone();
            }
        }
        if self.cached_symbol_count > 0 || self.db_path.exists() {
            if let Ok(Some(record)) = load_materialized_sage_method_from_db(
                &self.db_path,
                owner_type,
                member,
                &self.options.roots,
            ) {
                self.insert_sage_method_lookup_cache(owner_type, member, Some(record.clone()));
                return Some(record);
            }
        }
        let resolved = if let Some(spec) = SAGE_METHOD_SPECS
            .iter()
            .find(|spec| spec.owner_type == owner_type && spec.member == member)
        {
            self.resolve_symbol_in_module(member, spec.module)
        } else {
            SAGE_METHOD_ALIAS_SPECS
                .iter()
                .find(|spec| spec.owner_type == owner_type && spec.member == member)
                .and_then(|alias| self.resolve_symbol_in_module(alias.source_name, alias.module))
        };
        self.insert_sage_method_lookup_cache(owner_type, member, resolved.clone());
        resolved
    }

    fn known_sage_method_completions(
        &self,
        owner_type: SageOwnerType,
        prefix: &str,
        limit: usize,
    ) -> Vec<QueryCompletion> {
        let needle = prefix.to_ascii_lowercase();
        let mut completions = BTreeMap::<String, QueryCompletion>::new();
        if self.cached_symbol_count > 0 || self.db_path.exists() {
            if let Ok(entries) = load_materialized_sage_method_completions_from_db(
                &self.db_path,
                owner_type,
                prefix,
                &self.options.roots,
                limit,
            ) {
                for (member, record) in entries {
                    if member.starts_with('_') && !prefix.starts_with('_') {
                        continue;
                    }
                    self.insert_sage_method_lookup_cache(owner_type, &member, Some(record.clone()));
                    completions.entry(member.clone()).or_insert_with(|| {
                        method_completion_from_record(owner_type, &member, Some(&record))
                    });
                }
            }
        }
        for spec in SAGE_METHOD_SPECS
            .iter()
            .filter(|spec| spec.owner_type == owner_type)
            .filter(|spec| spec.member.starts_with(&needle))
        {
            let record = self.resolve_known_sage_method_record(owner_type, spec.member);
            completions
                .entry(spec.member.to_string())
                .or_insert_with(|| {
                    method_completion_from_record(owner_type, spec.member, record.as_ref())
                });
        }
        for spec in SAGE_METHOD_ALIAS_SPECS
            .iter()
            .filter(|spec| spec.owner_type == owner_type)
            .filter(|spec| spec.member.starts_with(&needle))
        {
            let record = self.resolve_known_sage_method_record(owner_type, spec.member);
            completions
                .entry(spec.member.to_string())
                .or_insert_with(|| {
                    method_completion_from_record(owner_type, spec.member, record.as_ref())
                });
        }
        completions.into_values().take(limit).collect()
    }

    fn resolve_symbol_in_module(&self, name: &str, module: &str) -> Option<SymbolRecord> {
        let mut symbols = if self.cached_symbol_count > 0 {
            load_symbols_by_name_and_module_from_db(
                &self.db_path,
                name,
                module,
                &self.options.roots,
            )
            .unwrap_or_default()
        } else {
            Vec::new()
        };
        if let Some(memory_symbols) = self.symbols_by_name.get(&name.to_ascii_lowercase()) {
            symbols.extend(
                memory_symbols
                    .iter()
                    .filter(|symbol| {
                        symbol.kind != SymbolKind::Import
                            && symbol.name == name
                            && module_matches_import(&symbol.module, module)
                    })
                    .cloned(),
            );
        }
        best_symbol(dedupe_symbol_records(symbols))
    }

    fn resolve_symbol_in_module_without_docs(
        &self,
        name: &str,
        module: &str,
    ) -> Option<SymbolRecord> {
        if let Ok(cache) = self.symbol_lookup_cache.lock() {
            if let Some(symbols) = cache.get(&name.to_ascii_lowercase()) {
                if let Some(symbol) = best_symbol(
                    symbols
                        .iter()
                        .filter(|symbol| {
                            symbol.kind != SymbolKind::Import
                                && symbol.name == name
                                && module_matches_import(&symbol.module, module)
                                && path_is_under_roots(&symbol.path, &self.options.roots)
                        })
                        .cloned()
                        .collect(),
                ) {
                    return Some(symbol);
                }
            }
        }
        let mut symbols = if self.cached_symbol_count > 0 {
            load_symbols_by_name_and_module_from_db_without_docs(
                &self.db_path,
                name,
                module,
                &self.options.roots,
            )
            .unwrap_or_default()
        } else {
            Vec::new()
        };
        if let Some(memory_symbols) = self.symbols_by_name.get(&name.to_ascii_lowercase()) {
            symbols.extend(
                memory_symbols
                    .iter()
                    .filter(|symbol| {
                        symbol.kind != SymbolKind::Import
                            && symbol.name == name
                            && module_matches_import(&symbol.module, module)
                    })
                    .cloned(),
            );
        }
        best_symbol(dedupe_symbol_records(symbols))
    }

    fn resolve_source_derived_namespace_owner(
        &self,
        source: &str,
        owner: &str,
        module_hint: Option<&str>,
    ) -> Option<SageExportResolution> {
        if let Some(lookup) = source_imported_sage_all_lookup(source, owner) {
            if let Some(resolution) =
                self.resolve_sage_exported_symbol_from(&lookup.import_module, &lookup.source_name)
            {
                if is_namespace_owner_record(&resolution.record) {
                    return Some(resolution);
                }
            }
        }
        if let Some(resolution) = self.resolve_sage_exported_symbol(owner) {
            if is_namespace_owner_record(&resolution.record) {
                return Some(resolution);
            }
        }
        let record = self
            .resolve_symbol(owner, module_hint)
            .or_else(|| self.resolve_symbol(owner, None))?;
        (record.kind == SymbolKind::Module && record.module.starts_with("sage.")).then_some(
            SageExportResolution {
                record,
                reason: "indexed namespace owner",
            },
        )
    }

    fn resolve_member_in_namespace_owner(
        &self,
        owner_record: &SymbolRecord,
        member: &str,
    ) -> Option<SymbolRecord> {
        let candidates = self
            .symbol_candidates(member)
            .into_iter()
            .filter(|candidate| namespace_member_matches_owner(candidate, owner_record, member))
            .collect::<Vec<_>>();
        let mut resolved = Vec::new();
        for candidate in candidates {
            if candidate.kind == SymbolKind::Import {
                resolved.push(
                    self.resolve_import_record(&candidate)
                        .unwrap_or_else(|| candidate.clone()),
                );
            } else {
                resolved.push(candidate);
            }
        }
        best_symbol(dedupe_symbol_records(resolved))
    }

    pub fn file_for_path(&self, path: &Path) -> Option<IndexedFile> {
        let path = normalize_path(path.to_path_buf());
        if !path_is_under_roots(&path, &self.options.roots) {
            return None;
        }
        if let Ok(cache) = self.file_lookup_cache.lock() {
            if let Some(file) = cache.get(&path) {
                return Some(file.clone());
            }
        }
        let mut file = if let Some(file) = self.files.get(&path).cloned() {
            file
        } else if self.cached_file_count > 0 {
            load_file_from_db(&self.db_path, &path).ok()?
        } else {
            return None;
        };
        if file.symbols.is_empty() && self.cached_symbol_count > 0 {
            if let Ok(symbols) = load_symbols_for_path_from_db(&self.db_path, &path) {
                file.symbols = symbols;
            }
        }
        if let Ok(mut cache) = self.file_lookup_cache.lock() {
            cache.insert(path, file.clone());
        }
        Some(file)
    }

    pub fn source_path_for_module(&self, module: &str) -> Option<PathBuf> {
        module_source_path_from_roots(module, &self.options.roots, self.options.enable_pyx)
    }

    pub fn diagnostics_for_source(&self, path: &Path, source: &str) -> Vec<DiagnosticRecord> {
        diagnostics_for_source(path, source)
    }

    pub fn references(&self, name: &str) -> Vec<ReferenceRecord> {
        self.references_matching(name, |_| true)
    }

    pub fn editable_references(&self, name: &str) -> Vec<ReferenceRecord> {
        if self.options.editable_roots.is_empty() {
            return self.references_matching(name, |path| self.is_editable_path(path));
        }
        if let Ok(cache) = self.reference_lookup_cache.lock() {
            if let Some(references) = cache.get(name) {
                return references.clone();
            }
        }
        let mut results = Vec::new();
        let mut loaded_from_db = false;
        if self.cached_file_count > 0 || self.db_path.exists() {
            if let Ok(cached) =
                load_reference_spans_from_db(&self.db_path, name, &self.options.editable_roots)
            {
                loaded_from_db = true;
                results.extend(cached);
            }
        }
        if !loaded_from_db {
            for file in self.files.values() {
                if !self.is_editable_path(&file.path) {
                    continue;
                }
                if let Ok(source) = fs::read_to_string(&file.path) {
                    results.extend(references_in_source(&file.path, &source, name));
                }
            }
        }
        let results = dedupe_reference_records(results);
        if let Ok(mut cache) = self.reference_lookup_cache.lock() {
            cache.insert(name.to_string(), results.clone());
        }
        results
    }

    fn references_matching<F>(&self, name: &str, include_path: F) -> Vec<ReferenceRecord>
    where
        F: Fn(&Path) -> bool,
    {
        let mut results = Vec::new();
        if self.cached_file_count > 0 {
            if let Ok(paths) = load_file_paths_from_db(&self.db_path, &self.options.roots) {
                for path in paths {
                    if !include_path(&path) {
                        continue;
                    }
                    if let Ok(source) = fs::read_to_string(&path) {
                        results.extend(references_in_source(&path, &source, name));
                    }
                }
            }
        }
        for file in self.files.values() {
            if !include_path(&file.path) {
                continue;
            }
            if let Ok(source) = fs::read_to_string(&file.path) {
                results.extend(references_in_source(&file.path, &source, name));
            }
        }
        dedupe_reference_records(results)
    }

    fn effective_editable_roots(&self) -> Vec<PathBuf> {
        if self.options.editable_roots.is_empty() {
            self.options.roots.clone()
        } else {
            self.options.editable_roots.clone()
        }
    }

    pub fn is_editable_path(&self, path: &Path) -> bool {
        self.effective_editable_roots()
            .iter()
            .any(|root| path.starts_with(root))
    }

    fn should_persist_reference_spans(&self, path: &Path) -> bool {
        !self.options.editable_roots.is_empty() && self.is_editable_path(path)
    }

    fn reset_operation_timings(&mut self, operation: &str) {
        self.last_operation = Some(operation.to_string());
        self.last_index_ms = 0;
        match operation {
            "hydrate" => {
                self.last_hydrate_ms = 0;
            }
            "reconcile" => {
                self.last_reconcile_ms = 0;
                self.last_persist_ms = 0;
                self.last_hot_cache_ms = 0;
            }
            "rebuild" | "refresh" => {
                self.last_hydrate_ms = 0;
                self.last_reconcile_ms = 0;
                self.last_persist_ms = 0;
                self.last_hot_cache_ms = 0;
            }
            _ => {}
        }
    }

    fn clear_lookup_cache(&self) {
        if let Ok(mut cache) = self.symbol_lookup_cache.lock() {
            cache.clear();
        }
        if let Ok(mut cache) = self.file_lookup_cache.lock() {
            cache.clear();
        }
        if let Ok(mut cache) = self.sage_method_lookup_cache.lock() {
            cache.clear();
        }
        if let Ok(mut cache) = self.reference_lookup_cache.lock() {
            cache.clear();
        }
    }

    fn clear_lookup_cache_entries(&self, names: &BTreeSet<String>) {
        if names.is_empty() {
            return;
        }
        if let Ok(mut cache) = self.symbol_lookup_cache.lock() {
            for name in names {
                cache.remove(&name.to_ascii_lowercase());
            }
        }
        if let Ok(mut cache) = self.file_lookup_cache.lock() {
            cache.clear();
        }
        if let Ok(mut cache) = self.sage_method_lookup_cache.lock() {
            cache.clear();
        }
        if let Ok(mut cache) = self.reference_lookup_cache.lock() {
            cache.clear();
        }
    }

    fn lookup_cache_len(&self) -> usize {
        self.symbol_lookup_cache
            .lock()
            .map(|cache| cache.len())
            .unwrap_or_default()
    }

    fn sage_method_cache_stats(&self) -> SageMethodCacheStats {
        if !self.db_path.exists() {
            return SageMethodCacheStats::default();
        }
        load_sage_method_cache_stats_from_db(&self.db_path).unwrap_or_default()
    }

    fn insert_sage_method_lookup_cache(
        &self,
        owner_type: SageOwnerType,
        member: &str,
        record: Option<SymbolRecord>,
    ) {
        if let Ok(mut cache) = self.sage_method_lookup_cache.lock() {
            cache.insert(sage_method_cache_key(owner_type, member), record);
        }
    }

    fn prewarm_hot_symbol_cache(&self, include_dynamic_exports: bool) {
        let names = hot_sage_symbol_names();
        let names: Vec<_> = names.into_iter().collect();
        let mut grouped = if self.cached_symbol_count > 0 || self.db_path.exists() {
            load_materialized_sage_export_groups_by_names_from_db(
                &self.db_path,
                "sage.all",
                &names,
                &self.options.roots,
            )
            .unwrap_or_default()
        } else {
            HashMap::new()
        };
        if include_dynamic_exports && (self.cached_symbol_count > 0 || self.db_path.exists()) {
            for (name, records) in
                load_hot_sage_export_groups_from_db(&self.db_path, "sage.all", &self.options.roots)
                    .unwrap_or_default()
            {
                grouped.entry(name).or_default().extend(records);
            }
        }
        if include_dynamic_exports {
            for name in self.hot_sage_export_names_from_memory() {
                if let Some(memory_symbols) = self.symbols_by_name.get(&name.to_ascii_lowercase()) {
                    grouped
                        .entry(name.to_ascii_lowercase())
                        .or_default()
                        .extend(memory_symbols.clone());
                }
            }
        }
        for name in &names {
            if let Some(memory_symbols) = self.symbols_by_name.get(&name.to_ascii_lowercase()) {
                grouped
                    .entry(name.to_ascii_lowercase())
                    .or_default()
                    .extend(memory_symbols.clone());
            }
        }
        if let Ok(mut cache) = self.symbol_lookup_cache.lock() {
            for name in names {
                let key = name.to_ascii_lowercase();
                let symbols = grouped.remove(&key).unwrap_or_default();
                cache.insert(key, dedupe_symbol_records(symbols));
            }
        }
        self.prewarm_hot_sage_method_cache();
    }

    fn prewarm_hot_sage_method_cache(&self) {
        if !(self.cached_symbol_count > 0 || self.db_path.exists()) {
            return;
        }
        let keys = hot_sage_method_keys();
        let methods =
            load_materialized_sage_methods_from_db(&self.db_path, &keys, &self.options.roots)
                .unwrap_or_default();
        if methods.is_empty() {
            return;
        }
        if let Ok(mut cache) = self.sage_method_lookup_cache.lock() {
            for (owner_type, member, record) in methods {
                cache.insert(sage_method_cache_key(owner_type, &member), Some(record));
            }
        }
    }

    fn hot_sage_export_names_from_memory(&self) -> BTreeSet<String> {
        let mut names = BTreeSet::new();
        for symbol in self.files.values().flat_map(|file| &file.symbols) {
            if symbol.kind == SymbolKind::Import && module_is_sage_all_export_module(&symbol.module)
            {
                insert_import_symbol_hot_names(&mut names, symbol);
                if names.len() >= MAX_DYNAMIC_HOT_EXPORT_NAMES {
                    break;
                }
            }
        }
        names
    }

    pub fn documentation_for_symbol(&self, name: &str) -> Option<DocumentationRecord> {
        if let Some(export) = self.resolve_sage_exported_symbol(name) {
            let documentation = self.documentation_for_resolved_symbol(&export.record);
            if documentation_has_specific_docstring(&documentation) {
                return Some(documentation);
            }
        }
        let static_documentation = self
            .resolve_symbol(name, None)
            .or_else(|| builtin_symbol_record(name))
            .map(|symbol| self.documentation_for_resolved_symbol(&symbol));
        if static_documentation
            .as_ref()
            .is_some_and(documentation_has_specific_docstring)
        {
            return static_documentation;
        }
        if let Ok(Some(runtime_documentation)) =
            load_runtime_documentation_from_db(&self.db_path, name)
        {
            return Some(runtime_documentation);
        }
        static_documentation
    }

    pub fn documentation_for_symbol_with_module(
        &self,
        name: &str,
        module_hint: Option<&str>,
    ) -> Option<DocumentationRecord> {
        let static_documentation = self
            .resolve_symbol(name, module_hint)
            .or_else(|| builtin_symbol_record(name))
            .map(|symbol| self.documentation_for_resolved_symbol(&symbol));
        if static_documentation
            .as_ref()
            .is_some_and(documentation_has_specific_docstring)
        {
            return static_documentation;
        }
        self.documentation_for_symbol(name).or(static_documentation)
    }

    pub fn write_runtime_documentation(
        &self,
        symbol: &str,
        record: &DocumentationRecord,
    ) -> Result<()> {
        if symbol.trim().is_empty() {
            return Ok(());
        }
        if record
            .docstring
            .as_deref()
            .is_none_or(|docstring| docstring.trim().is_empty())
            && record.summary.trim().is_empty()
        {
            return Ok(());
        }
        let connection = Connection::open(&self.db_path)
            .with_context(|| format!("open index db {}", self.db_path.display()))?;
        create_schema(&connection)?;
        upsert_runtime_documentation(&connection, symbol, record)
    }

    fn documentation_for_resolved_symbol(&self, symbol: &SymbolRecord) -> DocumentationRecord {
        let mut documentation = documentation_for_symbol(symbol);
        if symbol.kind == SymbolKind::Variable
            && symbol.docstring.as_ref().is_none_or(String::is_empty)
        {
            for related_name in [
                format!("{}Factory", symbol.name),
                format!("{}_class", symbol.name),
            ] {
                let related = self
                    .resolve_symbol(&related_name, Some(&symbol.module))
                    .or_else(|| self.resolve_symbol(&related_name, None));
                if let Some(related) = related {
                    if let Some(docstring) = related
                        .docstring
                        .as_ref()
                        .filter(|docstring| !docstring.is_empty())
                    {
                        documentation.summary =
                            documentation_summary(docstring).unwrap_or_else(|| docstring.clone());
                        documentation.docstring = Some(docstring.clone());
                        documentation
                            .markers
                            .push(format!("related-doc:{}:{}", related.module, related.name));
                        break;
                    }
                }
            }
        }
        documentation
    }

    pub fn query_source_at(
        &self,
        path: &Path,
        source: &str,
        position: QueryPosition,
        rename_to: Option<&str>,
    ) -> QueryResult {
        self.query_source_at_with_features(path, source, position, rename_to, QueryFeatures::full())
    }

    pub fn query_source_at_navigation(
        &self,
        path: &Path,
        source: &str,
        position: QueryPosition,
    ) -> QueryResult {
        self.query_source_at_with_features(
            path,
            source,
            position,
            None,
            QueryFeatures::navigation(),
        )
    }

    pub fn type_definition_at_source(
        &self,
        _path: &Path,
        source: &str,
        position: QueryPosition,
    ) -> Option<QueryDefinition> {
        let (word, _range) = word_at_source_position(source, position.line, position.character)?;
        let constructor = assignment_constructor_before_line(source, &word, position.line);
        let type_symbol = constructor
            .as_deref()
            .and_then(type_symbol_for_constructor)
            .or_else(|| {
                infer_owner_type_before(source, &word, "", position.line)
                    .and_then(type_symbol_for_owner_type)
            })?;
        self.type_definition_for_symbol(type_symbol)
    }

    fn type_definition_for_symbol(&self, type_symbol: &str) -> Option<QueryDefinition> {
        let target = SAGE_EXPORT_MAP
            .iter()
            .find(|target| target.import_module == "sage.all" && target.name == type_symbol);
        let record = target
            .and_then(|target| {
                self.resolve_symbol_in_module_without_docs(target.source_name, target.source_module)
                    .or_else(|| {
                        self.resolve_module_symbol_from_roots(
                            target.source_module,
                            target.source_name,
                            0,
                            &mut BTreeSet::new(),
                        )
                    })
            })
            .or_else(|| self.resolve_symbol(type_symbol, None))?;
        query_definition_from_record(&record)
    }

    pub fn query_source_at_with_features(
        &self,
        path: &Path,
        source: &str,
        position: QueryPosition,
        rename_to: Option<&str>,
        features: QueryFeatures,
    ) -> QueryResult {
        let diagnostics = if features.diagnostics {
            self.diagnostics_for_source(path, source)
        } else {
            Vec::new()
        };
        let Some((word, range)) =
            word_at_source_position(source, position.line, position.character)
        else {
            return QueryResult {
                diagnostics,
                fallback_reason: Some("no-symbol-at-position".to_string()),
                ..QueryResult::default()
            };
        };
        self.query_source_symbol_with_options(
            path,
            source,
            &word,
            Some(range),
            QueryExecutionOptions {
                rename_to,
                diagnostics,
                features,
            },
        )
    }

    pub fn query_source_symbol(
        &self,
        path: &Path,
        source: &str,
        symbol: &str,
        known_range: Option<SourceRange>,
        rename_to: Option<&str>,
        diagnostics: Vec<DiagnosticRecord>,
    ) -> QueryResult {
        self.query_source_symbol_with_options(
            path,
            source,
            symbol,
            known_range,
            QueryExecutionOptions {
                rename_to,
                diagnostics,
                features: QueryFeatures::full(),
            },
        )
    }

    pub fn query_source_symbol_with_options(
        &self,
        path: &Path,
        source: &str,
        symbol: &str,
        known_range: Option<SourceRange>,
        options: QueryExecutionOptions<'_>,
    ) -> QueryResult {
        let QueryExecutionOptions {
            rename_to,
            diagnostics,
            features,
        } = options;
        let query_path = normalize_path(path.to_path_buf());
        let target_range = known_range
            .or_else(|| range_for_first_symbol(source, symbol))
            .unwrap_or_default();
        let source_map = CodeMap::new(source);
        let target_is_code = source_map
            .offset(target_range.start_line, target_range.start_character)
            .is_some_and(|offset| source_map.is_code_offset(offset));
        let dotted_symbol = dotted_symbol_at_range(source, &target_range);
        let lookup_name = dotted_symbol
            .as_deref()
            .and_then(|value| value.rsplit('.').next())
            .filter(|value| !value.is_empty())
            .unwrap_or(symbol);
        let module_hint = self
            .file_for_path(&query_path)
            .map(|file| file.module)
            .or_else(|| {
                self.options
                    .roots
                    .iter()
                    .find(|root| query_path.strip_prefix(root).is_ok())
                    .map(|root| module_name_from_path(root, &query_path))
            });
        let dotted_owner_member = dotted_symbol.as_deref().and_then(dotted_owner_member);
        let source_import_lookup = source_explicit_import_lookup(source, lookup_name);
        let sage_all_export_lookup = source_imported_sage_all_lookup(source, lookup_name);
        let implicit_sage_all_lookup =
            is_sage_source_path(&query_path) && dotted_symbol.is_none() && target_is_code;
        let member_resolution = dotted_owner_member.map(|(owner, member)| {
            self.resolve_member_symbol(
                source,
                owner,
                member,
                module_hint.as_deref(),
                target_range.start_line,
            )
        });
        let mut suppress_global_fallback = member_resolution
            .as_ref()
            .is_some_and(|resolution| resolution.suppress_global_fallback);
        let mut resolution_confidence = member_resolution
            .as_ref()
            .map(|resolution| resolution.confidence.to_string());
        let mut resolution_reason = member_resolution
            .as_ref()
            .map(|resolution| resolution.reason.clone());
        let owner_type = member_resolution
            .as_ref()
            .and_then(|resolution| resolution.owner_type)
            .map(|owner_type| owner_type.as_str().to_string());
        let mut candidate_count = member_resolution
            .as_ref()
            .map(|resolution| resolution.candidate_count)
            .unwrap_or(0);
        let mut resolved = member_resolution
            .as_ref()
            .and_then(|resolution| resolution.record.clone());
        if resolved.is_none()
            && !suppress_global_fallback
            && dotted_symbol.is_none()
            && target_is_code
            && (implicit_sage_all_lookup
                || sage_all_export_lookup.is_some()
                || source_import_lookup.is_some())
        {
            let local_module = module_hint.as_deref().unwrap_or("document");
            if let Some(local_symbol) = local_shadow_symbol_from_source(
                local_module,
                &query_path,
                source,
                lookup_name,
                &target_range,
            ) {
                resolution_confidence = Some("high".to_string());
                resolution_reason = Some(format!(
                    "current document local symbol `{lookup_name}` shadows Sage import/export"
                ));
                resolved = Some(local_symbol);
            }
        }
        if resolved.is_none() && !suppress_global_fallback && implicit_sage_all_lookup {
            if let Some(export) = self.resolve_sage_exported_symbol(lookup_name) {
                resolution_confidence = Some("high".to_string());
                resolution_reason = Some(format!(
                    "resolved `{lookup_name}` through implicit .sage {}",
                    export.reason
                ));
                candidate_count = 1;
                resolved = Some(export.record);
            }
        }
        if resolved.is_none() && !suppress_global_fallback {
            if let Some(export_lookup) = sage_all_export_lookup.as_ref() {
                if let Some(export) = self.resolve_sage_exported_symbol_from(
                    &export_lookup.import_module,
                    &export_lookup.source_name,
                ) {
                    resolution_confidence = Some("high".to_string());
                    resolution_reason = Some(format!(
                        "resolved `{lookup_name}` through {}",
                        export.reason
                    ));
                    resolved = Some(export.record);
                } else {
                    suppress_global_fallback = true;
                    resolution_confidence = Some("ambiguous".to_string());
                    resolution_reason = Some(format!(
                        "`{lookup_name}` is imported from {} but is not present in the materialized Sage export cache",
                        export_lookup.import_module
                    ));
                }
            }
        }
        if resolved.is_none() && !suppress_global_fallback {
            if let Some(import_lookup) = source_import_lookup
                .as_ref()
                .filter(|lookup| !module_is_sage_all_export_module(&lookup.import_module))
            {
                let import_module = module_hint
                    .as_deref()
                    .map(|module| resolve_relative_module(&import_lookup.import_module, module))
                    .unwrap_or_else(|| import_lookup.import_module.clone());
                let mut seen = BTreeSet::new();
                if let Some(record) = self
                    .symbol_candidates(&import_lookup.source_name)
                    .into_iter()
                    .filter(|candidate| {
                        import_target_definition_matches(
                            candidate,
                            &import_module,
                            &import_lookup.source_name,
                        )
                    })
                    .min_by_key(symbol_choice_key)
                    .or_else(|| {
                        self.resolve_module_symbol_from_roots(
                            &import_module,
                            &import_lookup.source_name,
                            0,
                            &mut seen,
                        )
                    })
                {
                    resolution_confidence = Some("high".to_string());
                    resolution_reason = Some(format!(
                        "resolved `{lookup_name}` from explicit import target {}",
                        import_module
                    ));
                    resolved = Some(record);
                } else {
                    suppress_global_fallback = true;
                    resolution_confidence = Some("ambiguous".to_string());
                    resolution_reason = Some(format!(
                        "`{lookup_name}` is explicitly imported from {} but the target module is not indexed or resolvable",
                        import_module
                    ));
                }
            }
        }
        if resolved.is_none()
            && !suppress_global_fallback
            && dotted_symbol.is_none()
            && target_is_code
        {
            if let Some(record) = self.resolve_loaded_symbol_before_line(
                &query_path,
                source,
                lookup_name,
                target_range.start_line,
            ) {
                resolution_confidence = Some("high".to_string());
                resolution_reason = Some(format!(
                    "resolved `{lookup_name}` from a Sage load/attach target"
                ));
                candidate_count = 1;
                resolved = Some(record);
            }
        }
        if resolved.is_none() && !suppress_global_fallback {
            resolved = self
                .resolve_symbol(lookup_name, module_hint.as_deref())
                .or_else(|| self.resolve_symbol(symbol, module_hint.as_deref()))
                .or_else(|| builtin_symbol_record(dotted_symbol.as_deref().unwrap_or(symbol)))
                .or_else(|| builtin_symbol_record(lookup_name));
        }
        if resolved.is_some() && resolution_confidence.is_none() {
            resolution_confidence = Some("medium".to_string());
            resolution_reason = Some("resolved by indexed symbol/import lookup".to_string());
        }
        if let Some(record) = &resolved {
            if record.kind == SymbolKind::Import {
                if let Some(source_record) = self.resolve_import_record(record) {
                    resolved = Some(source_record);
                }
            }
        }
        let precise_lookup = resolution_reason.as_deref().is_some_and(|reason| {
            reason.contains("sage.all")
                || reason.contains("explicit import target")
                || reason.contains("shadows Sage import/export")
                || reason.contains("load/attach target")
                || reason.contains("resolved Sage ")
        });
        if !precise_lookup {
            if let Some(record) = &resolved {
                candidate_count = candidate_count.max(self.symbol_candidates(&record.name).len());
            }
        }
        let mut documentation = resolved
            .as_ref()
            .map(|record| self.documentation_for_resolved_symbol(record));
        let mut hover = resolved.as_ref().map(|record| QueryHover {
            markdown: hover_markdown_for_symbol(record, documentation.as_ref()),
            range: target_range.clone(),
        });
        if resolved.is_none() {
            if let Some(ambiguous_documentation) =
                member_resolution.as_ref().and_then(|resolution| {
                    self.ambiguous_member_documentation(
                        lookup_name,
                        &resolution.reason,
                        resolution.candidate_count,
                    )
                })
            {
                hover = Some(QueryHover {
                    markdown: hover_markdown_for_ambiguous_member(&ambiguous_documentation),
                    range: target_range.clone(),
                });
                documentation = Some(ambiguous_documentation);
            }
        }
        let definition = resolved.as_ref().and_then(query_definition_from_record);
        let completions = if features.completions {
            self.completion_items_at_source_with_fallback(
                source,
                QueryPosition {
                    line: target_range.start_line,
                    character: target_range.start_character,
                },
                80,
                Some(lookup_name),
            )
        } else {
            Vec::new()
        };
        let should_collect_references = features.references || features.rename_preview;
        let read_only_definition = resolved.as_ref().is_some_and(|record| {
            !record.path.as_os_str().is_empty() && !self.is_editable_path(&record.path)
        });
        let references = if should_collect_references && !read_only_definition {
            scope_references_for_resolved_symbol(
                self.editable_references(lookup_name),
                resolved.as_ref(),
                &query_path,
            )
        } else {
            Vec::new()
        };
        let rename_preview = if features.rename_preview {
            rename_to
                .filter(|new_name| is_valid_identifier(new_name))
                .map(|new_name| {
                    references
                        .iter()
                        .map(|reference| QueryTextEdit {
                            path: reference.path.clone(),
                            range: reference.range.clone(),
                            new_text: new_name.to_string(),
                        })
                        .collect()
                })
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let signature = if features.signature {
            target_is_code
                .then(|| {
                    function_call_at_position_with_code_map(
                        source,
                        target_range.start_line,
                        target_range.start_character,
                        &source_map,
                    )
                })
                .flatten()
                .and_then(|(name, active_parameter)| {
                    self.resolve_symbol(&name, module_hint.as_deref())
                        .or_else(|| builtin_symbol_record(&name))
                        .and_then(|record| {
                            record.signature.clone().map(|label| QuerySignature {
                                label,
                                active_parameter,
                                documentation: record.docstring,
                            })
                        })
                })
                .or_else(|| {
                    resolved.as_ref().and_then(|record| {
                        record.signature.clone().map(|label| QuerySignature {
                            label,
                            active_parameter: 0,
                            documentation: record.docstring.clone(),
                        })
                    })
                })
        } else {
            None
        };
        let fallback_reason = resolved.as_ref().is_none().then(|| {
            resolution_reason.clone().unwrap_or_else(|| {
                member_resolution
                    .as_ref()
                    .filter(|resolution| resolution.suppress_global_fallback)
                    .map(|resolution| resolution.reason.clone())
                    .unwrap_or_else(|| "symbol-not-in-index-or-known-sage-set".to_string())
            })
        });

        QueryResult {
            target: Some(QueryTarget {
                symbol: symbol.to_string(),
                dotted_symbol,
                range: target_range,
            }),
            hover,
            documentation,
            definition,
            completions,
            references,
            rename_preview,
            signature,
            diagnostics,
            fallback_reason,
            resolution_confidence,
            resolution_reason,
            owner_type,
            candidate_count,
        }
    }

    pub fn all_files(&self) -> Vec<IndexedFile> {
        self.files.values().cloned().collect()
    }

    fn ambiguous_member_documentation(
        &self,
        member: &str,
        reason: &str,
        candidate_count: usize,
    ) -> Option<DocumentationRecord> {
        let mut candidates: Vec<_> = self
            .symbol_candidates(member)
            .into_iter()
            .filter(|symbol| symbol.kind != SymbolKind::Import)
            .filter(|symbol| symbol.signature.is_some() || symbol.docstring.is_some())
            .collect();
        if candidates.is_empty() {
            return None;
        }
        candidates.sort_by_key(symbol_choice_key);
        candidates.dedup_by(|left, right| {
            left.name == right.name
                && left.module == right.module
                && left.path == right.path
                && left.range == right.range
        });
        let sections = candidates
            .iter()
            .take(5)
            .map(|candidate| {
                let mut body = Vec::new();
                if let Some(signature) = &candidate.signature {
                    body.push(format!("```sage\n{signature}\n```"));
                }
                if let Some(summary) = candidate
                    .docstring
                    .as_deref()
                    .and_then(documentation_summary)
                {
                    body.push(summary.to_string());
                }
                body.push(format!("Module: `{}`", candidate.module));
                if !candidate.path.as_os_str().is_empty() {
                    body.push(format!("Source: `{}`", candidate.path.display()));
                }
                DocumentationSection {
                    title: candidate.detail.clone(),
                    body: body.join("\n\n"),
                }
            })
            .collect();
        Some(DocumentationRecord {
            name: member.to_string(),
            module_name: "ambiguous".to_string(),
            kind: "AmbiguousMember".to_string(),
            detail: format!("Ambiguous Sage member `{member}`"),
            summary: format!(
                "Ambiguous Sage member `{member}` has {candidate_count} indexed candidates; no definition jump was returned to avoid a wrong target."
            ),
            docstring: Some(format!(
                "Reason: {reason}\n\nUse completion or refine the receiver type to choose a specific implementation."
            )),
            uri: None,
            markers: vec![
                "ambiguous".to_string(),
                "source:rust-index-v2".to_string(),
            ],
            sections,
        })
    }

    fn rebuild_symbol_map(&mut self) {
        let files: Vec<_> = self.files.values().cloned().collect();
        self.symbols_by_name = symbol_map_from_files(&files);
    }

    fn persist_all(&self) -> Result<()> {
        let mut connection = Connection::open(&self.db_path)
            .with_context(|| format!("open index db {}", self.db_path.display()))?;
        tune_cache_connection(&connection)?;
        create_schema(&connection)?;
        let tx = connection.transaction()?;
        delete_roots_from_db(&tx, &self.options.roots)?;
        {
            let mut file_statement =
                tx.prepare("insert into files(path, module, fingerprint) values(?1, ?2, ?3)")?;
            let mut symbol_statement = tx.prepare(
                "insert into symbols(name, kind, module, path, start_line, start_character, end_line, end_character, detail, import_from, signature) values(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            )?;
            let mut doc_statement = tx.prepare(
                "insert into docs(name, module, path, detail, docstring) values(?1, ?2, ?3, ?4, ?5)",
            )?;
            let mut reference_statement = tx.prepare(
                "insert into reference_spans(name, path, start_line, start_character, end_line, end_character) values(?1, ?2, ?3, ?4, ?5, ?6)",
            )?;
            for file in self.files.values() {
                let references = self
                    .should_persist_reference_spans(&file.path)
                    .then_some(&mut reference_statement);
                insert_file_rows(
                    file,
                    &mut file_statement,
                    &mut symbol_statement,
                    &mut doc_statement,
                    references,
                )?;
            }
        }
        clear_doc_fts(&tx)?;
        refresh_materialized_caches_from_symbols(&tx, &self.symbols_by_name)?;
        update_root_metadata(&tx, &self.options.roots)?;
        tx.commit()?;
        Ok(())
    }

    fn persist_paths(
        &self,
        changed: &[IndexedFile],
        deleted: &[PathBuf],
        materialize_from_changed: bool,
        refresh_materialized: bool,
    ) -> Result<()> {
        let mut connection = Connection::open(&self.db_path)
            .with_context(|| format!("open index db {}", self.db_path.display()))?;
        tune_cache_connection(&connection)?;
        create_schema(&connection)?;
        let tx = connection.transaction()?;
        let metadata_deltas = if materialize_from_changed || refresh_materialized {
            None
        } else {
            Some(metadata_deltas_for_path_refresh(
                &tx,
                changed,
                deleted,
                &self.options.roots,
            )?)
        };
        for path in deleted {
            delete_path_from_db(&tx, &path.display().to_string())?;
        }
        for file in changed {
            persist_file(&tx, file, self.should_persist_reference_spans(&file.path))?;
        }
        clear_doc_fts(&tx)?;
        if materialize_from_changed {
            let symbols_by_name = symbol_map_from_files(changed);
            refresh_materialized_caches_from_symbols(&tx, &symbols_by_name)?;
        } else if refresh_materialized {
            refresh_materialized_caches(&tx, &self.options.roots)?;
        }
        if let Some(deltas) = metadata_deltas {
            update_root_metadata_with_deltas(&tx, &self.options.roots, deltas)?;
        } else {
            update_root_metadata(&tx, &self.options.roots)?;
        }
        tx.commit()?;
        Ok(())
    }

    fn persist_all_with_fallback(&mut self) -> Result<()> {
        self.switch_to_fallback_cache()?;
        self.persist_all()
    }

    fn persist_paths_with_fallback(
        &mut self,
        changed: &[IndexedFile],
        deleted: &[PathBuf],
        materialize_from_changed: bool,
        refresh_materialized: bool,
    ) -> Result<()> {
        self.switch_to_fallback_cache()?;
        self.persist_paths(
            changed,
            deleted,
            materialize_from_changed,
            refresh_materialized,
        )
    }

    fn seed_shared_roots_from_peer_caches(&mut self) {
        let started = Instant::now();
        if let Ok(imported) = self.try_seed_shared_roots_from_peer_caches() {
            if imported > 0 {
                self.peer_seed_file_count = self.peer_seed_file_count.saturating_add(imported);
                self.last_peer_seed_ms = started.elapsed().as_millis();
            }
        }
    }

    fn try_seed_shared_roots_from_peer_caches(&mut self) -> Result<usize> {
        self.ensure_cache_dir()?;
        let peer_paths = peer_cache_paths(&self.options.cache_dir, &self.db_path)?;
        if peer_paths.is_empty() {
            return Ok(0);
        }
        let seed_roots: Vec<PathBuf> = self
            .options
            .roots
            .iter()
            .filter(|root| {
                !self
                    .options
                    .editable_roots
                    .iter()
                    .any(|editable| root.starts_with(editable))
            })
            .cloned()
            .collect();
        if seed_roots.is_empty() {
            return Ok(0);
        }

        let mut connection = Connection::open(&self.db_path)
            .with_context(|| format!("open index db {}", self.db_path.display()))?;
        tune_cache_connection(&connection)?;
        create_schema(&connection)?;

        let mut imported = 0usize;
        for peer_path in peer_paths {
            if let Ok(count) =
                seed_shared_roots_from_peer_cache(&mut connection, &peer_path, &seed_roots)
            {
                imported = imported.saturating_add(count);
                if count > 0 && shared_roots_are_seeded(&connection, &seed_roots)? {
                    break;
                }
            }
        }
        if imported > 0 {
            refresh_materialized_caches(&connection, &self.options.roots)?;
        }
        Ok(imported)
    }

    fn ensure_cache_dir(&mut self) -> Result<()> {
        match fs::create_dir_all(&self.options.cache_dir) {
            Ok(()) => Ok(()),
            Err(primary_error) => self.switch_to_fallback_cache().with_context(|| {
                format!(
                    "create cache dir {}; fallback after: {primary_error}",
                    self.options.cache_dir.display()
                )
            }),
        }
    }

    fn switch_to_fallback_cache(&mut self) -> Result<()> {
        let fallback = fallback_cache_dir();
        fs::create_dir_all(&fallback)
            .with_context(|| format!("create fallback cache dir {}", fallback.display()))?;
        self.options.cache_dir = fallback;
        let digest = cache_namespace_digest(
            &self.options.roots,
            &self.options.exclude_globs,
            self.options.enable_pyx,
        );
        self.db_path = self
            .options
            .cache_dir
            .join(format!("sage-index-{digest}.sqlite"));
        Ok(())
    }

    fn load_cached_fingerprints_for_current_roots(&self) -> Result<BTreeMap<PathBuf, String>> {
        if !self.db_path.exists() {
            return Ok(BTreeMap::new());
        }
        let connection = Connection::open(&self.db_path)
            .with_context(|| format!("open index db {}", self.db_path.display()))?;
        create_schema(&connection)?;
        load_file_fingerprints_from_db(&connection, &self.options.roots)
    }

    fn cached_counts_for_current_roots(&self) -> Result<(usize, usize, usize)> {
        let connection = Connection::open(&self.db_path)
            .with_context(|| format!("open index db {}", self.db_path.display()))?;
        cached_counts_for_roots(&connection, &self.options.roots)
    }
}

pub fn sage_prewarm_modules_for_source(source: &str) -> Vec<&'static str> {
    let mut owner_types = BTreeSet::new();
    if source.contains("matrix(")
        || source.contains("Matrix(")
        || source.contains("zero_matrix")
        || source.contains(".rank(")
        || source.contains(".det(")
        || source.contains(".solve_right(")
        || source.contains(".right_kernel(")
    {
        owner_types.insert(SageOwnerType::Matrix);
        owner_types.insert(SageOwnerType::FreeModule);
    }
    if source.contains("PolynomialRing")
        || source.contains(".ideal(")
        || source.contains(".gens(")
        || source.contains(".gen(")
    {
        owner_types.insert(SageOwnerType::PolynomialRing);
        owner_types.insert(SageOwnerType::PolynomialElement);
        owner_types.insert(SageOwnerType::Ideal);
    }
    if source.contains("GF(")
        || source.contains("FiniteField(")
        || source.contains("NumberField(")
        || source.contains("CyclotomicField(")
        || source.contains("QuadraticField(")
    {
        owner_types.insert(SageOwnerType::Field);
        owner_types.insert(SageOwnerType::FieldElement);
        owner_types.insert(SageOwnerType::NumberField);
    }
    if source.contains("vector(") || source.contains("zero_vector") {
        owner_types.insert(SageOwnerType::Vector);
    }
    if source.contains("Graph(")
        || source.contains("DiGraph(")
        || source.contains("graphs.")
        || source.contains("graphs_")
    {
        owner_types.insert(SageOwnerType::Graph);
    }
    if source.contains("EllipticCurve(") {
        owner_types.insert(SageOwnerType::EllipticCurve);
    }
    if owner_types.is_empty() {
        return Vec::new();
    }
    let mut modules = BTreeSet::new();
    for spec in SAGE_METHOD_SPECS {
        if owner_types.contains(&spec.owner_type) {
            modules.insert(spec.module);
        }
    }
    for spec in SAGE_METHOD_ALIAS_SPECS {
        if owner_types.contains(&spec.owner_type) {
            modules.insert(spec.module);
        }
    }
    modules.into_iter().collect()
}

fn resolve_from_candidates(
    module_hint: Option<&str>,
    candidates: Vec<SymbolRecord>,
) -> Option<SymbolRecord> {
    if let Some(module_hint) = module_hint {
        if let Some(symbol) = candidates
            .iter()
            .find(|symbol| symbol.kind == SymbolKind::Import && symbol.module == module_hint)
        {
            if let Some(import_from) = &symbol.import_from {
                let (source_module, source_name) =
                    import_target_in_context(import_from, &symbol.name, &symbol.module);
                if let Some(resolved) = candidates
                    .iter()
                    .filter(|candidate| {
                        import_target_definition_matches(candidate, &source_module, &source_name)
                    })
                    .min_by_key(|candidate| symbol_choice_key(candidate))
                    .cloned()
                {
                    return Some(resolved);
                }
            }
            return Some(symbol.clone());
        }
        if let Some(symbol) = candidates
            .iter()
            .filter(|symbol| symbol.kind != SymbolKind::Import && symbol.module == module_hint)
            .min_by_key(|candidate| symbol_choice_key(candidate))
            .cloned()
        {
            return Some(symbol);
        }
    }
    best_symbol(candidates)
}

fn import_target(import_from: &str, fallback_name: &str) -> (String, String) {
    if let Some((module, name)) = import_from.split_once("::") {
        (module.to_string(), name.to_string())
    } else {
        (import_from.to_string(), fallback_name.to_string())
    }
}

fn import_target_in_context(
    import_from: &str,
    fallback_name: &str,
    importer_module: &str,
) -> (String, String) {
    let (module, name) = import_target(import_from, fallback_name);
    (resolve_relative_module(&module, importer_module), name)
}

fn normalize_import_from(import_from: &str, importer_module: &str, fallback_name: &str) -> String {
    let (module, name) = import_target_in_context(import_from, fallback_name, importer_module);
    if import_from.contains("::") {
        format!("{module}::{name}")
    } else {
        module
    }
}

fn resolve_relative_module(module: &str, importer_module: &str) -> String {
    if !module.starts_with('.') {
        return module.to_string();
    }
    let level = module.chars().take_while(|ch| *ch == '.').count();
    let rest = module[level..].trim_matches('.');
    let parts = importer_module.split('.').collect::<Vec<_>>();
    let base_len = parts.len().saturating_sub(level);
    let mut resolved = parts[..base_len].join(".");
    if !rest.is_empty() {
        if !resolved.is_empty() {
            resolved.push('.');
        }
        resolved.push_str(rest);
    }
    resolved
}

fn best_symbol(symbols: Vec<SymbolRecord>) -> Option<SymbolRecord> {
    symbols.into_iter().min_by_key(symbol_choice_key)
}

fn dedupe_best_symbols(symbols: Vec<SymbolRecord>, limit: usize) -> Vec<SymbolRecord> {
    let mut grouped: BTreeMap<String, Vec<SymbolRecord>> = BTreeMap::new();
    for symbol in dedupe_symbol_records(symbols) {
        grouped
            .entry(symbol.name.to_ascii_lowercase())
            .or_default()
            .push(symbol);
    }
    let mut results: Vec<_> = grouped.into_values().filter_map(best_symbol).collect();
    results.sort_by(|left, right| {
        left.name
            .cmp(&right.name)
            .then(left.module.cmp(&right.module))
    });
    results.truncate(limit);
    results
}

fn dedupe_symbol_records(symbols: Vec<SymbolRecord>) -> Vec<SymbolRecord> {
    let mut seen = BTreeSet::new();
    let mut deduped = Vec::new();
    for symbol in symbols {
        let key = (
            symbol.name.clone(),
            symbol_kind_as_str(&symbol.kind),
            symbol.module.clone(),
            symbol.path.clone(),
            symbol.range.start_line,
            symbol.range.start_character,
            symbol.range.end_line,
            symbol.range.end_character,
        );
        if seen.insert(key) {
            deduped.push(symbol);
        }
    }
    deduped
}

fn suppress_workspace_import_noise(symbols: Vec<SymbolRecord>) -> Vec<SymbolRecord> {
    let names_with_definitions: BTreeSet<String> = symbols
        .iter()
        .filter(|symbol| symbol.kind != SymbolKind::Import)
        .map(|symbol| symbol.name.to_ascii_lowercase())
        .collect();
    if names_with_definitions.is_empty() {
        return symbols;
    }
    symbols
        .into_iter()
        .filter(|symbol| {
            symbol.kind != SymbolKind::Import
                || !names_with_definitions.contains(&symbol.name.to_ascii_lowercase())
        })
        .collect()
}

fn workspace_symbol_sort_key(symbol: &SymbolRecord, needle: &str) -> (u8, u8, u8, usize) {
    let name = symbol.name.to_ascii_lowercase();
    let module = symbol.module.to_ascii_lowercase();
    let match_rank = if needle.is_empty() {
        3
    } else if name == needle {
        0
    } else if name.starts_with(needle) {
        1
    } else if symbol_word_boundary_match(&name, needle) {
        2
    } else if name.contains(needle) {
        3
    } else if module.contains(needle) {
        4
    } else {
        5
    };
    (
        match_rank,
        symbol_resolution_rank(&symbol.kind),
        symbol_path_rank(&symbol.path),
        symbol.name.len(),
    )
}

fn symbol_word_boundary_match(name: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }
    name.split('_').any(|part| part.starts_with(needle))
}

fn documentation_for_symbol(symbol: &SymbolRecord) -> DocumentationRecord {
    let uri =
        (!symbol.path.as_os_str().is_empty()).then(|| format!("file://{}", symbol.path.display()));
    DocumentationRecord {
        name: symbol.name.clone(),
        module_name: symbol.module.clone(),
        kind: format!("{:?}", symbol.kind),
        detail: symbol.detail.clone(),
        summary: symbol
            .docstring
            .as_deref()
            .and_then(documentation_summary)
            .unwrap_or_else(|| format!("{} from {}", symbol.name, symbol.module)),
        docstring: symbol.docstring.clone(),
        uri,
        markers: vec!["source:rust-index-v2".to_string()],
        sections: Vec::new(),
    }
}

fn documentation_has_specific_docstring(record: &DocumentationRecord) -> bool {
    record.docstring.as_deref().is_some_and(|docstring| {
        let docstring = docstring.trim();
        !docstring.is_empty() && !docstring.contains("Runtime documentation worker can provide")
    })
}

fn hover_markdown_for_symbol(
    symbol: &SymbolRecord,
    documentation: Option<&DocumentationRecord>,
) -> String {
    let mut lines = vec![
        "```sage".to_string(),
        symbol.detail.clone(),
        "```".to_string(),
        String::new(),
        format!("Module: `{}`", symbol.module),
    ];
    let docstring = documentation
        .and_then(|documentation| documentation.docstring.as_ref())
        .or(symbol.docstring.as_ref());
    if let Some(docstring) = docstring {
        if !docstring.is_empty() {
            lines.push(String::new());
            lines.push(compact_hover_docstring(docstring));
        }
    }
    lines.join("\n")
}

fn hover_markdown_for_ambiguous_member(documentation: &DocumentationRecord) -> String {
    let mut lines = vec![
        "```sage".to_string(),
        documentation.detail.clone(),
        "```".to_string(),
        String::new(),
        documentation.summary.clone(),
    ];
    if let Some(reason) = &documentation.docstring {
        lines.push(String::new());
        lines.push(reason.clone());
    }
    if !documentation.sections.is_empty() {
        lines.push(String::new());
        lines.push("Top indexed candidates:".to_string());
        for section in documentation.sections.iter().take(3) {
            lines.push(format!("- {}", section.title));
        }
    }
    lines.join("\n")
}

fn symbol_map_from_files(files: &[IndexedFile]) -> HashMap<String, Vec<SymbolRecord>> {
    let mut symbols_by_name: HashMap<String, Vec<SymbolRecord>> = HashMap::new();
    for file in files {
        for symbol in &file.symbols {
            symbols_by_name
                .entry(symbol.name.to_ascii_lowercase())
                .or_default()
                .push(symbol.clone());
        }
    }
    symbols_by_name
}

fn insert_file_symbol_names(names: &mut BTreeSet<String>, file: &IndexedFile) {
    for symbol in &file.symbols {
        names.insert(symbol.name.to_ascii_lowercase());
        if let Some(import_from) = symbol.import_from.as_deref() {
            let (_module, source_name) =
                import_target_in_context(import_from, &symbol.name, &symbol.module);
            names.insert(source_name.to_ascii_lowercase());
        }
    }
}

fn paths_need_materialized_cache_refresh(
    changed: &[IndexedFile],
    deleted: &[PathBuf],
    roots: &[PathBuf],
) -> bool {
    changed
        .iter()
        .any(|file| module_needs_materialized_cache_refresh(&file.module))
        || deleted.iter().any(|path| {
            roots
                .iter()
                .find(|root| path.starts_with(root))
                .map(|root| module_name_from_path(root, path))
                .is_some_and(|module| module_needs_materialized_cache_refresh(&module))
        })
}

fn module_needs_materialized_cache_refresh(module: &str) -> bool {
    module == "sage.all" || module.starts_with("sage.")
}

fn documentation_summary(docstring: &str) -> Option<String> {
    docstring
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(str::to_string)
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

fn builtin_symbol_record(name: &str) -> Option<SymbolRecord> {
    let short_name = name.rsplit('.').next().unwrap_or(name);
    let (kind, module, detail) = if SAGE_NAMESPACES.contains(&short_name) {
        (
            SymbolKind::Module,
            "sage.all",
            format!("namespace {short_name}"),
        )
    } else if SAGE_TYPES.contains(&short_name) {
        (
            SymbolKind::Class,
            "sage.all",
            format!("constructor {short_name}"),
        )
    } else if SAGE_FUNCTIONS.contains(&short_name) {
        (
            SymbolKind::Function,
            "sage.all",
            format!("function {short_name}"),
        )
    } else if SAGE_READONLY.contains(&short_name) {
        (
            SymbolKind::Variable,
            "sage.all",
            format!("constant {short_name}"),
        )
    } else {
        return None;
    };
    Some(SymbolRecord {
        name: short_name.to_string(),
        kind,
        module: module.to_string(),
        path: PathBuf::new(),
        range: SourceRange::default(),
        detail,
        docstring: Some(format!(
            "Known Sage symbol `{}`. Runtime documentation worker can provide the full Sage documentation when enabled.",
            name
        )),
        import_from: None,
        signature: None,
    })
}

fn word_at_source_position(text: &str, line: u32, character: u32) -> Option<(String, SourceRange)> {
    let source_line = text.lines().nth(line as usize)?;
    let mut character = character.min(source_line.len() as u32) as usize;
    if character == source_line.len() && character > 0 {
        character -= 1;
    }
    let bytes = source_line.as_bytes();
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
        source_line[start..end].to_string(),
        SourceRange {
            start_line: line,
            start_character: start as u32,
            end_line: line,
            end_character: end as u32,
        },
    ))
}

fn range_for_first_symbol(source: &str, symbol: &str) -> Option<SourceRange> {
    for (line_index, line) in source.lines().enumerate() {
        if let Some(start) = line.find(symbol) {
            return Some(SourceRange {
                start_line: line_index as u32,
                start_character: start as u32,
                end_line: line_index as u32,
                end_character: (start + symbol.len()) as u32,
            });
        }
    }
    None
}

fn dotted_symbol_at_range(source: &str, range: &SourceRange) -> Option<String> {
    let line = source.lines().nth(range.start_line as usize)?;
    let bytes = line.as_bytes();
    let mut start = range.start_character as usize;
    let mut end = range.end_character as usize;
    while start > 0 && is_word_byte(bytes[start - 1]) {
        start -= 1;
    }
    while end < bytes.len() && is_word_byte(bytes[end]) {
        end += 1;
    }
    if start > 0 && bytes[start - 1] == b'.' {
        let dot = start - 1;
        if let Some(owner_start) = python_primary_start(line, dot) {
            if owner_start < dot {
                let owner = line[owner_start..dot].trim();
                let member = line[start..end].trim();
                if !owner.is_empty() && !member.is_empty() {
                    return Some(format!("{owner}.{member}"));
                }
            }
        }
    }
    while start > 0 {
        let byte = bytes[start - 1];
        if is_word_byte(byte) || byte == b'.' {
            start -= 1;
        } else {
            break;
        }
    }
    while end < bytes.len() {
        let byte = bytes[end];
        if is_word_byte(byte) || byte == b'.' {
            end += 1;
        } else {
            break;
        }
    }
    let value = line[start..end].trim_matches('.');
    value.contains('.').then(|| value.to_string())
}

fn python_primary_start(line: &str, end: usize) -> Option<usize> {
    let bytes = line.as_bytes();
    let mut pos = end.min(bytes.len());
    while pos > 0 && bytes[pos - 1].is_ascii_whitespace() {
        pos -= 1;
    }
    loop {
        if pos == 0 {
            break;
        }
        match bytes[pos - 1] {
            b']' => pos = matching_open_bracket(bytes, pos - 1, b'[', b']')?,
            b')' => pos = matching_open_bracket(bytes, pos - 1, b'(', b')')?,
            byte if is_word_byte(byte) => {
                while pos > 0 && is_word_byte(bytes[pos - 1]) {
                    pos -= 1;
                }
            }
            b'.' => pos -= 1,
            _ => break,
        }
    }
    (pos < end).then_some(pos)
}

fn matching_open_bracket(bytes: &[u8], close_index: usize, open: u8, close: u8) -> Option<usize> {
    let mut depth = 0usize;
    for index in (0..=close_index).rev() {
        match bytes[index] {
            byte if byte == close => depth = depth.saturating_add(1),
            byte if byte == open => {
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

fn dotted_owner_member(value: &str) -> Option<(&str, &str)> {
    let (owner, member) = value.rsplit_once('.')?;
    (!owner.is_empty() && !member.is_empty()).then_some((owner, member))
}

fn assignment_constructor_before_line(
    source: &str,
    variable: &str,
    max_line: u32,
) -> Option<String> {
    let mut constructor = None;
    for (line_index, line) in source.lines().enumerate() {
        if line_index as u32 > max_line {
            break;
        }
        let trimmed = line.trim_start();
        if trimmed.starts_with('#') {
            continue;
        }
        let Some(captures) = assignment_constructor_re().captures(trimmed) else {
            continue;
        };
        if captures
            .name("name")
            .is_some_and(|name| name.as_str() == variable)
        {
            constructor = captures.name("ctor").map(|ctor| ctor.as_str().to_string());
        }
    }
    constructor
}

fn is_known_sage_method(member: &str) -> bool {
    SAGE_METHOD_SPECS.iter().any(|spec| spec.member == member)
        || SAGE_METHOD_ALIAS_SPECS
            .iter()
            .any(|spec| spec.member == member)
}

fn infer_owner_type_from_member_hint(member: &str) -> Option<SageOwnerType> {
    if is_matrix_member(member) {
        return Some(SageOwnerType::Matrix);
    }
    if is_free_module_member(member) {
        return Some(SageOwnerType::FreeModule);
    }
    if is_unique_graph_member(member) {
        return Some(SageOwnerType::Graph);
    }
    None
}

fn is_matrix_member(member: &str) -> bool {
    matches!(
        member,
        "adjugate"
            | "augment"
            | "change_ring"
            | "charpoly"
            | "column"
            | "column_space"
            | "det"
            | "dimensions"
            | "inverse"
            | "matrix_from_columns"
            | "matrix_from_rows"
            | "matrix_from_rows_and_columns"
            | "ncols"
            | "nrows"
            | "pivots"
            | "rank"
            | "right_kernel"
            | "row"
            | "rows"
            | "solve_right"
            | "subs"
            | "transpose"
    )
}

fn is_free_module_member(member: &str) -> bool {
    matches!(member, "basis" | "basis_matrix" | "dimension")
}

fn is_polynomial_element_member(member: &str) -> bool {
    matches!(
        member,
        "base_ring"
            | "constant_coefficient"
            | "degree"
            | "dict"
            | "factor"
            | "gcd"
            | "is_constant"
            | "is_zero"
            | "list"
            | "map_coefficients"
            | "monic"
            | "monomial_coefficient"
            | "parent"
            | "resultant"
            | "roots"
            | "subs"
            | "total_degree"
    )
}

fn is_vector_member(member: &str) -> bool {
    matches!(
        member,
        "base_ring" | "change_ring" | "column" | "list" | "row"
    )
}

fn is_field_member(member: &str) -> bool {
    matches!(member, "from_integer" | "order" | "random_element")
}

fn is_field_element_member(member: &str) -> bool {
    matches!(
        member,
        "integer_representation"
            | "parent"
            | "polynomial"
            | "to_integer"
            | "_integer_representation"
    )
}

fn is_graph_member(member: &str) -> bool {
    matches!(
        member,
        "adjacency_matrix"
            | "degree"
            | "edges"
            | "is_connected"
            | "neighbors"
            | "plot"
            | "shortest_path"
            | "vertices"
    )
}

fn is_unique_graph_member(member: &str) -> bool {
    matches!(
        member,
        "adjacency_matrix" | "edges" | "is_connected" | "neighbors" | "shortest_path" | "vertices"
    )
}

fn is_elliptic_curve_member(member: &str) -> bool {
    matches!(
        member,
        "base_ring"
            | "cardinality"
            | "gens"
            | "integral_points"
            | "order"
            | "plot"
            | "points"
            | "rank"
            | "torsion_subgroup"
    )
}

fn is_number_field_member(member: &str) -> bool {
    matches!(
        member,
        "absolute_degree"
            | "base_ring"
            | "class_group"
            | "degree"
            | "discriminant"
            | "embeddings"
            | "gen"
            | "gens"
            | "is_isomorphic"
            | "places"
            | "relative_degree"
            | "ring_of_integers"
            | "signature"
            | "unit_group"
    )
}

fn is_matrix_context_member(member: &str) -> bool {
    is_matrix_member(member) || matches!(member, "base_ring" | "list")
}

fn is_sage_namespace_owner(owner: &str) -> bool {
    owner
        .trim()
        .rsplit('.')
        .next()
        .is_some_and(|name| SAGE_STATIC_NAV_NAMESPACES.contains(&name))
}

fn infer_owner_type_before(
    source: &str,
    owner: &str,
    member: &str,
    max_line: u32,
) -> Option<SageOwnerType> {
    let local_function_returns = infer_local_function_return_types(source);
    let mut known_types: HashMap<String, SageOwnerType> = HashMap::new();
    let owner_base = owner_base_identifier(owner);
    for (line_index, line) in source.lines().enumerate() {
        if line_index as u32 > max_line {
            break;
        }
        let trimmed = line.trim_start();
        if trimmed.starts_with('#') {
            continue;
        }
        let Some((name, rhs)) = parse_simple_assignment(trimmed) else {
            continue;
        };
        if let Some(owner_type) = infer_type_from_rhs(rhs, &known_types, &local_function_returns)
            .or_else(|| infer_owner_type_from_name(name))
        {
            known_types.insert(name.to_string(), owner_type);
        }
    }
    let exact_owner_type = known_types.get(owner).copied();
    let expression_owner_type = infer_owner_type_from_owner_expression(owner, member);
    let base_owner_type = owner_base.and_then(|name| known_types.get(name).copied());
    exact_owner_type
        .or_else(|| {
            owner_is_compound(owner)
                .then_some(expression_owner_type)
                .flatten()
        })
        .or(base_owner_type)
        .or(expression_owner_type)
        .or_else(|| owner_base.and_then(|name| infer_owner_type_from_name_for_member(name, member)))
        .or_else(|| infer_owner_type_from_name_for_member(owner, member))
}

fn owner_base_identifier(owner: &str) -> Option<&str> {
    let owner = owner.trim_start();
    let bytes = owner.as_bytes();
    let mut end = 0usize;
    while end < bytes.len() && is_word_byte(bytes[end]) {
        end += 1;
    }
    (end > 0).then_some(&owner[..end])
}

fn owner_is_compound(owner: &str) -> bool {
    owner.contains('.') || owner.contains('(') || owner.contains('[')
}

fn infer_owner_type_from_owner_expression(owner: &str, member: &str) -> Option<SageOwnerType> {
    let compact: String = owner.chars().filter(|ch| !ch.is_whitespace()).collect();
    if compact.contains("[\"R\"]")
        || compact.contains("['R']")
        || compact.contains("[\"ring\"]")
        || compact.contains("['ring']")
    {
        return Some(SageOwnerType::PolynomialRing);
    }
    if compact.contains("[\"Q\"]")
        || compact.contains("['Q']")
        || compact.contains("[\"g\"]")
        || compact.contains("['g']")
    {
        return Some(SageOwnerType::PolynomialElement);
    }
    if compact.contains("[\"A\"]")
        || compact.contains("['A']")
        || compact.contains("[\"W\"]")
        || compact.contains("['W']")
    {
        return Some(SageOwnerType::Matrix);
    }
    if compact.contains("[\"b\"]") || compact.contains("['b']") {
        return Some(SageOwnerType::Vector);
    }
    if compact.contains("Graph(")
        || compact.contains("DiGraph(")
        || compact.contains("PetersenGraph(")
        || compact.contains("CompleteGraph(")
        || compact.contains("CycleGraph(")
    {
        return Some(SageOwnerType::Graph);
    }
    if compact.contains("EllipticCurve(") {
        return Some(SageOwnerType::EllipticCurve);
    }
    if compact.contains("NumberField(")
        || compact.contains("CyclotomicField(")
        || compact.contains("QuadraticField(")
    {
        return Some(SageOwnerType::NumberField);
    }
    if compact.contains('[') {
        if let Some(base) = owner_base_identifier(owner) {
            let lower = base.to_ascii_lowercase();
            if matches!(
                lower.as_str(),
                "qs" | "qhs" | "mats" | "matrices" | "gs" | "gtildes" | "ordinary_target"
            ) || lower.ends_with("_mats")
                || lower.ends_with("_matrices")
                || lower.ends_with("_qs")
            {
                return Some(SageOwnerType::Matrix);
            }
            if matches!(lower.as_str(), "eqs" | "polys" | "vars" | "xs" | "z_polys")
                || lower.ends_with("_eqs")
                || lower.ends_with("_polys")
            {
                return Some(SageOwnerType::PolynomialElement);
            }
            if matches!(lower.as_str(), "kernel" | "rows") || lower.ends_with("_rows") {
                return Some(SageOwnerType::Vector);
            }
        }
    }
    if compact.contains(".ideal(") {
        return Some(SageOwnerType::Ideal);
    }
    if compact.contains(".base_ring(") {
        return Some(SageOwnerType::Field);
    }
    if compact.contains(".polynomial(") {
        return Some(SageOwnerType::PolynomialElement);
    }
    if compact.contains('*') && is_matrix_context_member(member) {
        return Some(SageOwnerType::Matrix);
    }
    if compact.contains(".parent(") {
        if is_field_member(member) {
            return Some(SageOwnerType::Field);
        }
        if matches!(
            member,
            "gen" | "gens" | "hom" | "ideal" | "lagrange_polynomial"
        ) {
            return Some(SageOwnerType::PolynomialRing);
        }
    }
    if compact.contains(".charpoly(") {
        return Some(SageOwnerType::PolynomialElement);
    }
    if compact.contains(".gen(") || compact.contains(".gens(") {
        return Some(SageOwnerType::PolynomialElement);
    }
    if compact.contains(".transpose(")
        || compact.contains(".solve_right(")
        || compact.contains(".matrix_from_rows(")
        || compact.contains(".matrix_from_columns(")
        || compact.contains(".matrix_from_rows_and_columns(")
        || compact.contains(".adjugate(")
    {
        return Some(SageOwnerType::Matrix);
    }
    if compact.contains(".right_kernel(")
        || compact.contains(".column_space(")
        || compact.contains(".kernel(")
    {
        return Some(SageOwnerType::FreeModule);
    }
    if compact.contains(".basis_matrix(") {
        return Some(SageOwnerType::Matrix);
    }
    if compact.contains(".adjacency_matrix(") {
        return Some(SageOwnerType::Matrix);
    }
    None
}

fn infer_local_function_return_types(source: &str) -> HashMap<String, SageOwnerType> {
    let mut returns = HashMap::new();
    let lines: Vec<&str> = source.lines().collect();
    for (line_index, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        let Some(captures) = function_header_re().captures(trimmed) else {
            continue;
        };
        let Some(name) = captures.name("name").map(|name| name.as_str()) else {
            continue;
        };
        let indent = line.len() - trimmed.len();
        let mut known_types = HashMap::new();
        for body_line in lines.iter().skip(line_index + 1) {
            let body_trimmed = body_line.trim_start();
            if body_trimmed.is_empty() || body_trimmed.starts_with('#') {
                continue;
            }
            let body_indent = body_line.len() - body_trimmed.len();
            if body_indent <= indent {
                break;
            }
            if let Some((assigned, rhs)) = parse_simple_assignment(body_trimmed) {
                if let Some(owner_type) = infer_type_from_rhs(rhs, &known_types, &returns)
                    .or_else(|| infer_owner_type_from_name(assigned))
                {
                    known_types.insert(assigned.to_string(), owner_type);
                }
            }
            if let Some(return_expr) = body_trimmed.strip_prefix("return ") {
                if let Some(owner_type) = infer_type_from_rhs(return_expr, &known_types, &returns) {
                    returns.insert(name.to_string(), owner_type);
                    break;
                }
            }
        }
    }
    returns
}

fn parse_simple_assignment(line: &str) -> Option<(&str, &str)> {
    let captures = simple_assignment_re().captures(line)?;
    let name = captures.name("name")?.as_str();
    let rhs = captures.name("rhs")?.as_str();
    Some((name, rhs))
}

fn infer_type_from_rhs(
    rhs: &str,
    known_types: &HashMap<String, SageOwnerType>,
    local_function_returns: &HashMap<String, SageOwnerType>,
) -> Option<SageOwnerType> {
    let value = rhs.trim();
    if value.is_empty() {
        return None;
    }
    if value.contains(".ideal(") {
        return Some(SageOwnerType::Ideal);
    }
    if value.contains("[\"R\"]") || value.contains("['R']") {
        return Some(SageOwnerType::PolynomialRing);
    }
    if value.contains("[\"Q\"]") || value.contains("['Q']") {
        return Some(SageOwnerType::PolynomialElement);
    }
    if let Some(product_type) = infer_product_type_from_rhs(value, known_types) {
        return Some(product_type);
    }
    let callee = assignment_call_re()
        .captures(value)
        .and_then(|captures| captures.name("callee"))
        .map(|callee| callee.as_str());
    if let Some(callee) = callee {
        let short = callee.rsplit('.').next().unwrap_or(callee);
        if let Some(owner_type) = local_function_returns.get(short).copied() {
            return Some(owner_type);
        }
        if let Some(owner_type) = sage_constructor_return_type(short) {
            return Some(owner_type);
        }
        if known_types
            .get(short)
            .is_some_and(|owner_type| *owner_type == SageOwnerType::PolynomialRing)
        {
            return Some(SageOwnerType::PolynomialElement);
        }
        if let Some(owner_type) = known_types.get(callee).copied() {
            return Some(owner_type);
        }
        if callee.contains('.') {
            let member = callee.rsplit('.').next().unwrap_or(callee);
            if let Some(owner_type) = sage_method_return_type(member) {
                return Some(owner_type);
            }
        }
    }
    if identifier_re().is_match(value) {
        return known_types.get(value).copied();
    }
    None
}

fn infer_product_type_from_rhs(
    value: &str,
    known_types: &HashMap<String, SageOwnerType>,
) -> Option<SageOwnerType> {
    if !value.contains('*') {
        return None;
    }
    let mut saw_matrix = false;
    let mut saw_vector = false;
    for captures in word_re().captures_iter(value) {
        let Some(name) = captures.name("name").map(|name| name.as_str()) else {
            continue;
        };
        let owner_type = known_types
            .get(name)
            .copied()
            .or_else(|| infer_owner_type_from_name(name));
        match owner_type {
            Some(SageOwnerType::Matrix) => saw_matrix = true,
            Some(SageOwnerType::Vector) => saw_vector = true,
            _ => {}
        }
    }
    match (saw_matrix, saw_vector) {
        (true, true) => Some(SageOwnerType::Vector),
        (true, false) => Some(SageOwnerType::Matrix),
        (false, true) => Some(SageOwnerType::Vector),
        (false, false) => None,
    }
}

fn sage_constructor_return_type(name: &str) -> Option<SageOwnerType> {
    match name {
        "matrix" | "zero_matrix" | "identity_matrix" | "random_matrix" | "block_matrix" => {
            Some(SageOwnerType::Matrix)
        }
        "vector" | "zero_vector" => Some(SageOwnerType::Vector),
        "GF" | "FiniteField" | "QQ" | "ZZ" | "RR" => Some(SageOwnerType::Field),
        "Graph" | "DiGraph" | "PetersenGraph" | "CompleteGraph" | "CycleGraph" => {
            Some(SageOwnerType::Graph)
        }
        "EllipticCurve" | "EllipticCurve_from_j" | "EllipticCurve_from_c4c6" => {
            Some(SageOwnerType::EllipticCurve)
        }
        "NumberField" | "CyclotomicField" | "QuadraticField" => Some(SageOwnerType::NumberField),
        "PolynomialRing"
        | "LaurentPolynomialRing"
        | "PowerSeriesRing"
        | "BooleanPolynomialRing" => Some(SageOwnerType::PolynomialRing),
        _ => None,
    }
}

fn type_symbol_for_constructor(constructor: &str) -> Option<&'static str> {
    let short = constructor.rsplit('.').next().unwrap_or(constructor);
    match short {
        "Graph" | "DiGraph" | "PetersenGraph" | "CompleteGraph" | "CycleGraph" => Some("Graph"),
        "EllipticCurve" | "EllipticCurve_from_j" | "EllipticCurve_from_c4c6" => {
            Some("EllipticCurve")
        }
        "NumberField" | "CyclotomicField" | "QuadraticField" => Some("NumberField"),
        "PolynomialRing"
        | "LaurentPolynomialRing"
        | "PowerSeriesRing"
        | "BooleanPolynomialRing" => Some("PolynomialRing"),
        "GF" | "FiniteField" => Some("GF"),
        "matrix" | "zero_matrix" | "identity_matrix" | "random_matrix" | "block_matrix" => {
            Some("matrix")
        }
        "vector" | "zero_vector" => Some("vector"),
        _ => None,
    }
}

fn type_symbol_for_owner_type(owner_type: SageOwnerType) -> Option<&'static str> {
    match owner_type {
        SageOwnerType::Graph => Some("Graph"),
        SageOwnerType::EllipticCurve => Some("EllipticCurve"),
        SageOwnerType::NumberField => Some("NumberField"),
        SageOwnerType::PolynomialRing => Some("PolynomialRing"),
        SageOwnerType::Field => Some("GF"),
        SageOwnerType::MatrixConstructor => Some("matrix"),
        SageOwnerType::Matrix => Some("matrix"),
        SageOwnerType::Vector => Some("vector"),
        SageOwnerType::FreeModule
        | SageOwnerType::PolynomialElement
        | SageOwnerType::Ideal
        | SageOwnerType::FieldElement => None,
    }
}

fn sage_method_return_type(member: &str) -> Option<SageOwnerType> {
    match member {
        "ideal" => Some(SageOwnerType::Ideal),
        "adjugate"
        | "basis_matrix"
        | "change_ring"
        | "matrix_from_columns"
        | "matrix_from_rows"
        | "matrix_from_rows_and_columns"
        | "transpose" => Some(SageOwnerType::Matrix),
        "right_kernel" | "column_space" | "kernel" => Some(SageOwnerType::FreeModule),
        "adjacency_matrix" => Some(SageOwnerType::Matrix),
        "charpoly" | "gen" | "gens" | "gcd" | "resultant" | "derivative" => {
            Some(SageOwnerType::PolynomialElement)
        }
        "base_ring" => Some(SageOwnerType::Field),
        _ => None,
    }
}

fn infer_owner_type_from_name(name: &str) -> Option<SageOwnerType> {
    let lower = name.to_ascii_lowercase();
    if matches!(name, "R" | "S") || lower.ends_with("ring") {
        return Some(SageOwnerType::PolynomialRing);
    }
    if matches!(name, "I" | "ideal") || lower.ends_with("ideal") {
        return Some(SageOwnerType::Ideal);
    }
    if matches!(name, "F" | "K") || lower == "field" || lower.ends_with("_field") {
        return Some(SageOwnerType::Field);
    }
    if matches!(name, "E") || lower == "curve" || lower.ends_with("_curve") {
        return Some(SageOwnerType::EllipticCurve);
    }
    if lower == "graph" || lower.ends_with("_graph") || lower == "digraph" {
        return Some(SageOwnerType::Graph);
    }
    if lower == "number_field" || lower.ends_with("_number_field") {
        return Some(SageOwnerType::NumberField);
    }
    if lower.starts_with("vec") || lower.ends_with("vec") || lower.ends_with("vector") {
        return Some(SageOwnerType::Vector);
    }
    if matches!(name, "jac") || lower.contains("mat") || lower.ends_with("matrix") {
        return Some(SageOwnerType::Matrix);
    }
    if matches!(name, "cp" | "f1" | "f2" | "fac" | "factor" | "pivot")
        || lower.contains("poly")
        || lower.ends_with("_factor")
        || lower.ends_with("_factors")
        || lower.ends_with("_polynomial")
    {
        return Some(SageOwnerType::PolynomialElement);
    }
    if matches!(name, "eq" | "q" | "g") || lower.ends_with("_poly") {
        return Some(SageOwnerType::PolynomialElement);
    }
    None
}

fn infer_owner_type_from_name_for_member(name: &str, member: &str) -> Option<SageOwnerType> {
    if name == "matrix" {
        return Some(SageOwnerType::MatrixConstructor);
    }
    if (matches!(name, "G") || name.eq_ignore_ascii_case("graph")) && is_graph_member(member) {
        return Some(SageOwnerType::Graph);
    }
    if (matches!(name, "E") || name.eq_ignore_ascii_case("curve"))
        && is_elliptic_curve_member(member)
    {
        return Some(SageOwnerType::EllipticCurve);
    }
    if (matches!(name, "K") || name.eq_ignore_ascii_case("number_field"))
        && is_number_field_member(member)
    {
        return Some(SageOwnerType::NumberField);
    }
    if name == "f" && is_polynomial_element_member(member) {
        return Some(SageOwnerType::PolynomialElement);
    }
    if matches!(
        name,
        "A" | "G" | "P" | "Q" | "Q0" | "Q0inv" | "Qa" | "S1" | "T" | "base" | "base_inv"
    ) && is_matrix_context_member(member)
    {
        return Some(SageOwnerType::Matrix);
    }
    if matches!(name, "symbolic_obj" | "numeric_obj") && is_matrix_context_member(member) {
        return Some(SageOwnerType::Matrix);
    }
    if matches!(name, "u" | "v" | "target_u" | "u_candidate" | "vec") && is_vector_member(member) {
        return Some(SageOwnerType::Vector);
    }
    if matches!(name, "element" | "entry" | "root" | "value" | "x" | "y")
        && is_field_element_member(member)
    {
        return Some(SageOwnerType::FieldElement);
    }
    if matches!(name, "expr" | "equation" | "entry" | "poly" | "polynomial")
        && is_polynomial_element_member(member)
    {
        return Some(SageOwnerType::PolynomialElement);
    }
    infer_owner_type_from_name(name)
}

fn assignment_detail(name: &str, annotation: Option<&str>, rhs: &str) -> String {
    if let Some(annotation) = annotation.map(str::trim).filter(|value| !value.is_empty()) {
        return format!("Variable {name}: {annotation}");
    }

    if let Some(inferred) = infer_assignment_value_kind(rhs) {
        return format!("Variable {name}: {inferred}");
    }

    format!("Variable {name}")
}

fn infer_assignment_value_kind(rhs: &str) -> Option<String> {
    let value = rhs.trim();
    if value.is_empty() {
        return None;
    }

    if value.starts_with('"') || value.starts_with('\'') {
        return Some("str".to_string());
    }
    if value.starts_with('[') {
        return Some("list".to_string());
    }
    if value.starts_with('{') {
        return Some("dict/set".to_string());
    }
    if value.starts_with('(') {
        return Some("tuple/group".to_string());
    }
    if value == "True" || value == "False" {
        return Some("bool".to_string());
    }
    if value.parse::<i64>().is_ok() {
        return Some("Integer".to_string());
    }
    if value.parse::<f64>().is_ok() {
        return Some("RealNumber".to_string());
    }
    if let Some(callee) = assignment_call_re()
        .captures(value)
        .and_then(|captures| captures.name("callee"))
        .map(|callee| callee.as_str())
    {
        if callee
            .chars()
            .next()
            .is_some_and(|first| first.is_ascii_uppercase())
            || SAGE_TYPES.contains(&callee)
        {
            return Some(callee.to_string());
        }
        return Some(format!("result of {callee}(...)"));
    }
    if identifier_re().is_match(value) {
        return Some(format!("value of {value}"));
    }
    None
}

fn current_prefix(text: &str, line: u32, character: u32) -> Option<String> {
    let source_line = text.lines().nth(line as usize)?;
    let character = character.min(source_line.len() as u32) as usize;
    let bytes = source_line.as_bytes();
    let mut start = character;
    while start > 0 && is_word_byte(bytes[start - 1]) {
        start -= 1;
    }
    Some(source_line[start..character].to_string())
}

fn is_code_completion_position(source: &str, position: QueryPosition) -> bool {
    if source.is_empty() {
        return true;
    }
    let code_map = CodeMap::new(source);
    let Some(offset) = code_map.offset(position.line, position.character) else {
        return false;
    };
    let check_offset = if offset >= source.len() {
        offset.saturating_sub(1)
    } else {
        offset
    };
    code_map.is_code_offset(check_offset)
}

fn local_completion_items(
    source: &str,
    position: QueryPosition,
    prefix: &str,
    limit: usize,
) -> Vec<QueryCompletion> {
    if limit == 0 {
        return Vec::new();
    }
    let mut records = parse_source("document", Path::new("document.py"), source).symbols;
    records.extend(scoped_local_symbols(source, position));

    let needle = prefix.to_ascii_lowercase();
    let mut seen = BTreeSet::new();
    let mut completions = Vec::new();
    for record in records {
        if completions.len() >= limit {
            break;
        }
        if !needle.is_empty() && !record.name.to_ascii_lowercase().starts_with(&needle) {
            continue;
        }
        if !should_offer_document_symbol(&record, position) {
            continue;
        }
        if seen.insert(record.name.to_ascii_lowercase()) {
            completions.push(completion_from_symbol(record));
        }
    }
    completions
}

fn local_shadow_symbol_from_source(
    module: &str,
    path: &Path,
    source: &str,
    name: &str,
    target_range: &SourceRange,
) -> Option<SymbolRecord> {
    parse_source(module, path, source)
        .symbols
        .into_iter()
        .filter(|record| {
            record.name == name
                && record.kind != SymbolKind::Import
                && is_local_shadow_before_or_at_target(record, target_range)
        })
        .min_by_key(|record| {
            (
                target_range
                    .start_line
                    .saturating_sub(record.range.start_line),
                symbol_choice_key(record),
            )
        })
}

fn is_local_shadow_before_or_at_target(record: &SymbolRecord, target_range: &SourceRange) -> bool {
    let same_range = record.range == *target_range;
    match record.kind {
        SymbolKind::Function | SymbolKind::Class | SymbolKind::CythonDeclaration => {
            same_range || record.range.start_line < target_range.start_line
        }
        SymbolKind::Variable | SymbolKind::PreparserGenerator => {
            same_range || record.range.start_line < target_range.start_line
        }
        SymbolKind::Import | SymbolKind::Module => false,
    }
}

fn should_offer_document_symbol(record: &SymbolRecord, position: QueryPosition) -> bool {
    match record.kind {
        SymbolKind::Class
        | SymbolKind::Function
        | SymbolKind::CythonDeclaration
        | SymbolKind::PreparserGenerator => true,
        SymbolKind::Import => !is_star_import_symbol(record) && !is_all_export_symbol(record),
        SymbolKind::Variable => record.range.start_line <= position.line,
        SymbolKind::Module => false,
    }
}

fn completion_from_symbol(record: SymbolRecord) -> QueryCompletion {
    let documentation = record.docstring.as_ref().map(|docstring| {
        if let Some(signature) = &record.signature {
            format!("```sage\n{signature}\n```\n\n{docstring}")
        } else {
            docstring.clone()
        }
    });
    QueryCompletion {
        label: record.name.clone(),
        kind: format!("{:?}", record.kind),
        detail: record.detail.clone(),
        signature: record.signature,
        documentation,
        resolve_name: Some(record.name),
        module: Some(record.module),
    }
}

fn scoped_local_symbols(source: &str, position: QueryPosition) -> Vec<SymbolRecord> {
    let code_map = CodeMap::new(source);
    let scope = enclosing_function_scope(source, position);
    let mut symbols = Vec::new();
    symbols.extend(parameter_symbols_for_scope(source, &code_map, scope));
    symbols.extend(local_assignment_symbols_for_scope(
        source, &code_map, position, scope,
    ));
    symbols
}

fn enclosing_function_scope(source: &str, position: QueryPosition) -> Option<(u32, usize)> {
    let current_indent = source
        .lines()
        .nth(position.line as usize)
        .map(line_indent)
        .unwrap_or(0);
    if current_indent == 0 {
        return None;
    }
    let mut best = None;
    for (line_index, line) in source.lines().enumerate().take(position.line as usize + 1) {
        let trimmed = line.trim_start();
        if !(trimmed.starts_with("def ")
            || trimmed.starts_with("async def ")
            || trimmed.starts_with("cpdef ")
            || trimmed.starts_with("cdef "))
        {
            continue;
        }
        let indent = line_indent(line);
        if indent < current_indent {
            best = Some((line_index as u32, indent));
        }
    }
    best
}

fn parameter_symbols_for_scope(
    source: &str,
    code_map: &CodeMap,
    scope: Option<(u32, usize)>,
) -> Vec<SymbolRecord> {
    let Some((scope_line, _)) = scope else {
        return Vec::new();
    };
    let Some((line_start, line)) = line_offsets(source).into_iter().nth(scope_line as usize) else {
        return Vec::new();
    };
    let header_end = definition_header_end(source, line_start).unwrap_or(line_start + line.len());
    let header = &source[line_start..header_end];
    let Some(open) = header.find('(') else {
        return Vec::new();
    };
    let Some(close) = matching_close_paren(header, open) else {
        return Vec::new();
    };
    let mut symbols = Vec::new();
    for (raw, relative_start) in split_parameter_segments(&header[open + 1..close], open + 1) {
        let Some(name) = parameter_name(raw) else {
            continue;
        };
        if matches!(name, "self" | "cls") {
            continue;
        }
        let Some(name_relative) = raw.find(name) else {
            continue;
        };
        let absolute = line_start + relative_start + name_relative;
        let (line, character) = code_map.line_col(absolute);
        symbols.push(SymbolRecord {
            name: name.to_string(),
            kind: SymbolKind::Variable,
            module: "document".to_string(),
            path: PathBuf::from("document.py"),
            range: SourceRange {
                start_line: line,
                start_character: character,
                end_line: line,
                end_character: character + name.len() as u32,
            },
            detail: format!("Local parameter {name}"),
            docstring: None,
            import_from: None,
            signature: None,
        });
    }
    symbols
}

fn matching_close_paren(text: &str, open: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut quote = None;
    for (index, ch) in text[open..].char_indices() {
        let absolute = open + index;
        if ch == '\'' || ch == '"' {
            quote = match quote {
                Some(current) if current == ch => None,
                None => Some(ch),
                current => current,
            };
            continue;
        }
        if quote.is_some() {
            continue;
        }
        match ch {
            '(' => depth += 1,
            ')' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(absolute);
                }
            }
            _ => {}
        }
    }
    None
}

fn split_parameter_segments(params: &str, base_offset: usize) -> Vec<(&str, usize)> {
    let mut segments = Vec::new();
    let mut start = 0usize;
    let mut depth = 0usize;
    let mut quote = None;
    for (index, ch) in params.char_indices() {
        if ch == '\'' || ch == '"' {
            quote = match quote {
                Some(current) if current == ch => None,
                None => Some(ch),
                current => current,
            };
            continue;
        }
        if quote.is_some() {
            continue;
        }
        match ch {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                segments.push((&params[start..index], base_offset + start));
                start = index + 1;
            }
            _ => {}
        }
    }
    segments.push((&params[start..], base_offset + start));
    segments
}

fn local_assignment_symbols_for_scope(
    source: &str,
    code_map: &CodeMap,
    position: QueryPosition,
    scope: Option<(u32, usize)>,
) -> Vec<SymbolRecord> {
    let mut symbols = Vec::new();
    for captures in semantic_assignment_re().captures_iter(source) {
        let Some(name) = captures.name("name") else {
            continue;
        };
        if !code_map.is_code_offset(name.start()) {
            continue;
        }
        let (line, character) = code_map.line_col(name.start());
        if line > position.line {
            continue;
        }
        let Some(source_line) = source.lines().nth(line as usize) else {
            continue;
        };
        let indent = line_indent(source_line);
        let in_scope = match scope {
            Some((scope_line, scope_indent)) => line > scope_line && indent > scope_indent,
            None => indent == 0,
        };
        if !in_scope {
            continue;
        }
        symbols.push(SymbolRecord {
            name: name.as_str().to_string(),
            kind: SymbolKind::Variable,
            module: "document".to_string(),
            path: PathBuf::from("document.py"),
            range: SourceRange {
                start_line: line,
                start_character: character,
                end_line: line,
                end_character: character + name.as_str().len() as u32,
            },
            detail: format!("Local variable {}", name.as_str()),
            docstring: None,
            import_from: None,
            signature: None,
        });
    }
    symbols
}

fn parameter_name(raw: &str) -> Option<&str> {
    let without_default = raw.split('=').next()?.trim();
    let without_annotation = without_default.split(':').next()?.trim();
    let name = without_annotation
        .trim_start_matches('*')
        .trim()
        .trim_start_matches('/');
    if name.is_empty() || !is_valid_identifier(name) {
        None
    } else {
        Some(name)
    }
}

fn line_indent(line: &str) -> usize {
    line.chars().take_while(|ch| ch.is_whitespace()).count()
}

#[derive(Clone, Debug)]
struct MemberCompletionContext {
    owner: String,
    prefix: String,
}

fn member_completion_context(
    source: &str,
    position: QueryPosition,
) -> Option<MemberCompletionContext> {
    let code_map = CodeMap::new(source);
    let offset = code_map.offset(position.line, position.character)?;
    if offset > 0 && !code_map.is_code_offset(offset - 1) {
        return None;
    }
    let source_line = source.lines().nth(position.line as usize)?;
    let character = position.character.min(source_line.len() as u32) as usize;
    let bytes = source_line.as_bytes();
    let mut prefix_start = character;
    while prefix_start > 0 && is_word_byte(bytes[prefix_start - 1]) {
        prefix_start -= 1;
    }
    if prefix_start == 0 || bytes[prefix_start - 1] != b'.' {
        return None;
    }
    let dot = prefix_start - 1;
    let owner_start = python_primary_start(source_line, dot)?;
    if owner_start >= dot {
        return None;
    }
    let owner = source_line[owner_start..dot].trim();
    if owner.is_empty() {
        return None;
    }
    Some(MemberCompletionContext {
        owner: owner.to_string(),
        prefix: source_line[prefix_start..character].to_string(),
    })
}

fn infer_completion_owner_type(source: &str, owner: &str, line: u32) -> Option<SageOwnerType> {
    infer_owner_type_before(source, owner, "", line)
        .or_else(|| infer_owner_type_from_owner_expression(owner, ""))
        .or_else(|| {
            owner_base_identifier(owner).and_then(|name| {
                infer_owner_type_from_completion_owner_name(name)
                    .or_else(|| infer_owner_type_from_name(name))
            })
        })
        .or_else(|| infer_owner_type_from_completion_owner_name(owner))
        .or_else(|| infer_owner_type_from_name(owner))
}

fn infer_owner_type_from_completion_owner_name(name: &str) -> Option<SageOwnerType> {
    let lower = name.to_ascii_lowercase();
    if name == "matrix" {
        return Some(SageOwnerType::MatrixConstructor);
    }
    if matches!(
        name,
        "A" | "G"
            | "M"
            | "P"
            | "Q"
            | "Q0"
            | "Q0inv"
            | "Qa"
            | "S1"
            | "T"
            | "base"
            | "base_inv"
            | "symbolic_obj"
            | "numeric_obj"
    ) || lower.contains("mat")
        || lower.ends_with("matrix")
    {
        return Some(SageOwnerType::Matrix);
    }
    if matches!(
        name,
        "u" | "v"
            | "target_u"
            | "u_candidate"
            | "vec"
            | "vec_obj"
            | "signature"
            | "normalized_signature"
    ) || lower.ends_with("vec")
        || lower.ends_with("vector")
    {
        return Some(SageOwnerType::Vector);
    }
    if matches!(name, "field" | "F" | "K") || lower.ends_with("_field") {
        return Some(SageOwnerType::Field);
    }
    if matches!(name, "curve" | "elliptic_curve") || lower.ends_with("_curve") {
        return Some(SageOwnerType::EllipticCurve);
    }
    if matches!(name, "graph" | "digraph") || lower.ends_with("_graph") {
        return Some(SageOwnerType::Graph);
    }
    if lower == "number_field" || lower.ends_with("_number_field") {
        return Some(SageOwnerType::NumberField);
    }
    if matches!(name, "value" | "element" | "entry" | "x" | "y" | "root") {
        return Some(SageOwnerType::FieldElement);
    }
    if matches!(
        name,
        "f" | "f1" | "f2" | "poly" | "polynomial" | "fac" | "factor"
    ) || lower.contains("poly")
        || lower.ends_with("_factor")
    {
        return Some(SageOwnerType::PolynomialElement);
    }
    None
}

fn method_completion_from_record(
    owner_type: SageOwnerType,
    label: &str,
    record: Option<&SymbolRecord>,
) -> QueryCompletion {
    let detail = record
        .map(|record| {
            if record.name != label {
                format!("{} (alias for {})", record.detail, record.name)
            } else {
                record.detail.clone()
            }
        })
        .filter(|detail| !detail.trim().is_empty())
        .unwrap_or_else(|| format!("Sage {} method", owner_type.as_str()));
    QueryCompletion {
        label: label.to_string(),
        kind: record
            .map(|record| format!("{:?}", record.kind))
            .unwrap_or_else(|| "Method".to_string()),
        detail,
        signature: record.and_then(|record| record.signature.clone()),
        documentation: record.and_then(|record| {
            record.docstring.as_ref().map(|docstring| {
                if let Some(signature) = &record.signature {
                    format!("```sage\n{signature}\n```\n\n{docstring}")
                } else {
                    docstring.clone()
                }
            })
        }),
        resolve_name: record
            .map(|record| record.name.clone())
            .or_else(|| Some(label.to_string())),
        module: record.map(|record| record.module.clone()),
    }
}

pub fn function_call_at_position(text: &str, line: u32, character: u32) -> Option<(String, u32)> {
    let code_map = CodeMap::new(text);
    function_call_at_position_with_code_map(text, line, character, &code_map)
}

fn function_call_at_position_with_code_map(
    text: &str,
    line: u32,
    character: u32,
    code_map: &CodeMap,
) -> Option<(String, u32)> {
    let offset = code_map.offset(line, character)?;
    let mut stack: Vec<CallFrame> = Vec::new();

    for (index, ch) in text.char_indices() {
        if index >= offset {
            break;
        }
        if !code_map.is_code_offset(index) {
            continue;
        }
        match ch {
            '(' => stack.push(CallFrame {
                close: ')',
                name: callable_name_before(text, index),
                active_parameter: 0,
            }),
            '[' => stack.push(CallFrame {
                close: ']',
                name: None,
                active_parameter: 0,
            }),
            '{' => stack.push(CallFrame {
                close: '}',
                name: None,
                active_parameter: 0,
            }),
            ')' | ']' | '}' => pop_call_frame(&mut stack, ch),
            ',' => {
                if let Some(frame) = stack.last_mut().filter(|frame| frame.name.is_some()) {
                    frame.active_parameter += 1;
                }
            }
            _ => {}
        }
    }

    stack.iter().rev().find_map(|frame| {
        frame
            .name
            .as_ref()
            .map(|name| (name.clone(), frame.active_parameter))
    })
}

#[derive(Clone, Debug)]
struct CallFrame {
    close: char,
    name: Option<String>,
    active_parameter: u32,
}

fn pop_call_frame(stack: &mut Vec<CallFrame>, close: char) {
    while let Some(frame) = stack.pop() {
        if frame.close == close {
            break;
        }
    }
}

fn callable_name_before(text: &str, open_index: usize) -> Option<String> {
    let prefix = &text[..open_index];
    let bytes = prefix.as_bytes();
    let mut end = bytes.len();
    while end > 0 && bytes[end - 1].is_ascii_whitespace() {
        end -= 1;
    }
    let mut start = end;
    while start > 0 && is_word_byte(bytes[start - 1]) {
        start -= 1;
    }
    (start < end).then(|| prefix[start..end].to_string())
}

fn is_word_byte(byte: u8) -> bool {
    byte == b'_' || byte.is_ascii_alphanumeric()
}

fn is_valid_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first == '_' || first.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

fn tune_cache_connection(connection: &Connection) -> Result<()> {
    connection.execute_batch(
        r#"
        pragma synchronous = off;
        pragma temp_store = memory;
        pragma cache_size = -200000;
        "#,
    )?;
    Ok(())
}

fn cached_counts_for_roots(
    connection: &Connection,
    roots: &[PathBuf],
) -> Result<(usize, usize, usize)> {
    if let Some(counts) = load_cached_counts_from_metadata_partial(connection, roots)? {
        return Ok(counts);
    }
    cached_counts_for_roots_by_path_scan(connection, roots)
}

fn cached_counts_for_roots_by_path_scan(
    connection: &Connection,
    roots: &[PathBuf],
) -> Result<(usize, usize, usize)> {
    let files = load_file_fingerprints_from_db(connection, roots)?;
    let paths: BTreeSet<String> = files
        .keys()
        .map(|path| path.display().to_string())
        .collect();
    if paths.is_empty() {
        return Ok((0, 0, 0));
    }
    let symbol_count = count_paths(connection, "symbols", &paths)?;
    let doc_count = count_docs_for_paths(connection, &paths)?;
    Ok((paths.len(), symbol_count, doc_count))
}

fn load_cached_counts_from_metadata(
    connection: &Connection,
    roots: &[PathBuf],
) -> Result<Option<(usize, usize, usize)>> {
    let roots = metadata_count_roots(roots);
    if roots.is_empty() {
        return Ok(None);
    }
    let Ok(mut statement) = connection.prepare(
        "select file_count, symbol_count, doc_count from index_root_metadata where root = ?1",
    ) else {
        return Ok(None);
    };
    let mut file_count = 0usize;
    let mut symbol_count = 0usize;
    let mut doc_count = 0usize;
    for root in &roots {
        let root_text = root.display().to_string();
        let Some((files, symbols, docs)) = statement
            .query_row(params![root_text], |row| {
                Ok((
                    row.get::<_, usize>(0)?,
                    row.get::<_, usize>(1)?,
                    row.get::<_, usize>(2)?,
                ))
            })
            .optional()?
        else {
            return Ok(None);
        };
        file_count = file_count.saturating_add(files);
        symbol_count = symbol_count.saturating_add(symbols);
        doc_count = doc_count.saturating_add(docs);
    }
    Ok(Some((file_count, symbol_count, doc_count)))
}

fn load_cached_counts_from_metadata_partial(
    connection: &Connection,
    roots: &[PathBuf],
) -> Result<Option<(usize, usize, usize)>> {
    let roots = metadata_count_roots(roots);
    if roots.is_empty() {
        return Ok(None);
    }
    let Ok(mut statement) = connection.prepare(
        "select file_count, symbol_count, doc_count from index_root_metadata where root = ?1",
    ) else {
        return Ok(None);
    };
    let mut file_count = 0usize;
    let mut symbol_count = 0usize;
    let mut doc_count = 0usize;
    let mut missing_roots = Vec::new();
    let mut found_metadata = false;
    for root in &roots {
        let root_text = root.display().to_string();
        let Some((files, symbols, docs)) = statement
            .query_row(params![root_text], |row| {
                Ok((
                    row.get::<_, usize>(0)?,
                    row.get::<_, usize>(1)?,
                    row.get::<_, usize>(2)?,
                ))
            })
            .optional()?
        else {
            missing_roots.push(root.clone());
            continue;
        };
        found_metadata = true;
        file_count = file_count.saturating_add(files);
        symbol_count = symbol_count.saturating_add(symbols);
        doc_count = doc_count.saturating_add(docs);
    }
    if !missing_roots.is_empty() {
        if !found_metadata {
            return Ok(None);
        }
        let (files, symbols, docs) =
            cached_counts_for_roots_by_path_scan(connection, &missing_roots)?;
        file_count = file_count.saturating_add(files);
        symbol_count = symbol_count.saturating_add(symbols);
        doc_count = doc_count.saturating_add(docs);
    }
    Ok(Some((file_count, symbol_count, doc_count)))
}

fn metadata_count_roots(roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut count_roots = Vec::<PathBuf>::new();
    for root in normalize_paths(roots.to_vec()) {
        if count_roots.iter().any(|kept| root.starts_with(kept)) {
            continue;
        }
        count_roots.retain(|kept| !kept.starts_with(&root));
        count_roots.push(root);
    }
    count_roots
}

fn peer_cache_paths(cache_dir: &Path, current_db_path: &Path) -> Result<Vec<PathBuf>> {
    let current_name = current_db_path.file_name().and_then(|name| name.to_str());
    let mut paths = Vec::new();
    for entry in fs::read_dir(cache_dir)
        .with_context(|| format!("read cache dir {}", cache_dir.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if current_name == Some(name) {
            continue;
        }
        if name.starts_with("sage-index-") && name.ends_with(".sqlite") {
            let modified = entry
                .metadata()
                .and_then(|metadata| metadata.modified())
                .ok();
            paths.push((path, modified));
        }
    }
    paths.sort_by(|(left_path, left_modified), (right_path, right_modified)| {
        right_modified
            .cmp(left_modified)
            .then_with(|| left_path.cmp(right_path))
    });
    Ok(paths.into_iter().map(|(path, _)| path).collect())
}

fn seed_shared_roots_from_peer_cache(
    connection: &mut Connection,
    peer_path: &Path,
    roots: &[PathBuf],
) -> Result<usize> {
    let peer_path_text = peer_path.display().to_string();
    connection.execute("attach database ?1 as peer_seed", params![peer_path_text])?;
    let result = seed_shared_roots_from_attached_peer(connection, roots);
    let detach_result = connection.execute("detach database peer_seed", []);
    result.and_then(|imported| {
        detach_result?;
        Ok(imported)
    })
}

fn seed_shared_roots_from_attached_peer(
    connection: &mut Connection,
    roots: &[PathBuf],
) -> Result<usize> {
    let tx = connection.transaction()?;
    let mut imported = 0usize;
    for root in roots {
        let current_fingerprint = source_root_fingerprint(root);
        if metadata_matches_current_root(
            root_metadata_for_schema(&tx, "", root)?,
            &current_fingerprint,
        ) {
            continue;
        }
        let peer_metadata = root_metadata_for_schema(&tx, "peer_seed.", root)?;
        if !metadata_matches_current_root(peer_metadata, &current_fingerprint) {
            continue;
        }
        imported = imported.saturating_add(copy_root_from_attached_peer(&tx, root)?);
    }
    tx.commit()?;
    Ok(imported)
}

fn root_metadata_for_schema(
    connection: &Connection,
    schema_prefix: &str,
    root: &Path,
) -> Result<Option<(usize, Option<String>)>> {
    let sql = format!(
        "select file_count, root_fingerprint from {schema_prefix}index_root_metadata where root = ?1"
    );
    let root_text = root.display().to_string();
    connection
        .query_row(&sql, params![root_text], |row| {
            Ok((row.get::<_, usize>(0)?, row.get::<_, Option<String>>(1)?))
        })
        .optional()
        .map_err(Into::into)
}

fn schema_table_has_column(
    connection: &Connection,
    schema_prefix: &str,
    table: &str,
    column: &str,
) -> Result<bool> {
    let sql = match schema_prefix.strip_suffix('.') {
        Some(schema) if !schema.is_empty() => format!("pragma {schema}.table_info({table})"),
        _ => format!("pragma table_info({table})"),
    };
    let mut statement = connection.prepare(&sql)?;
    let rows = statement.query_map([], |row| row.get::<_, String>(1))?;
    for row in rows {
        if row? == column {
            return Ok(true);
        }
    }
    Ok(false)
}

fn shared_roots_are_seeded(connection: &Connection, roots: &[PathBuf]) -> Result<bool> {
    for root in roots {
        let current_fingerprint = source_root_fingerprint(root);
        if !metadata_matches_current_root(
            root_metadata_for_schema(connection, "", root)?,
            &current_fingerprint,
        ) {
            return Ok(false);
        }
    }
    Ok(true)
}

fn metadata_matches_current_root(
    metadata: Option<(usize, Option<String>)>,
    current: &SourceRootFingerprint,
) -> bool {
    let Some((file_count, cached_digest)) = metadata else {
        return false;
    };
    if file_count == 0 {
        return false;
    }
    cached_digest
        .filter(|digest| !digest.is_empty())
        .is_none_or(|digest| digest == current.digest)
}

fn copy_root_from_attached_peer(connection: &Connection, root: &Path) -> Result<usize> {
    let root_text = root.display().to_string();
    let child_pattern = like_pattern_for_children(&root_text);
    for table in ["docs", "reference_spans", "symbols", "files"] {
        connection.execute(
            &format!("delete from {table} where path = ?1 or path like ?2 escape '~'"),
            params![root_text.as_str(), child_pattern.as_str()],
        )?;
    }
    connection.execute(
        "delete from sage_export_cache where path = ?1 or path like ?2 escape '~'",
        params![root_text.as_str(), child_pattern.as_str()],
    )?;
    connection.execute(
        "delete from sage_method_cache where path = ?1 or path like ?2 escape '~'",
        params![root_text.as_str(), child_pattern.as_str()],
    )?;

    connection.execute(
        "insert into files(path, module, fingerprint)
         select path, module, fingerprint from peer_seed.files
         where path = ?1 or path like ?2 escape '~'",
        params![root_text.as_str(), child_pattern.as_str()],
    )?;
    connection.execute(
        "insert into symbols(name, kind, module, path, start_line, start_character, end_line, end_character, detail, import_from, signature)
         select name, kind, module, path, start_line, start_character, end_line, end_character, detail, import_from, signature from peer_seed.symbols
         where path = ?1 or path like ?2 escape '~'",
        params![root_text.as_str(), child_pattern.as_str()],
    )?;
    connection.execute(
        "insert into docs(name, module, path, detail, docstring)
         select name, module, path, detail, docstring from peer_seed.docs
         where path = ?1 or path like ?2 escape '~'",
        params![root_text.as_str(), child_pattern.as_str()],
    )?;
    connection.execute(
        "insert into reference_spans(name, path, start_line, start_character, end_line, end_character)
         select name, path, start_line, start_character, end_line, end_character from peer_seed.reference_spans
         where path = ?1 or path like ?2 escape '~'",
        params![root_text.as_str(), child_pattern.as_str()],
    )?;
    connection.execute(
        "insert or replace into sage_export_cache(public_name, source_name, import_module, reason, name, kind, module, path, start_line, start_character, end_line, end_character, detail, import_from, signature, docstring)
         select public_name, source_name, import_module, reason, name, kind, module, path, start_line, start_character, end_line, end_character, detail, import_from, signature, docstring from peer_seed.sage_export_cache
         where path = ?1 or path like ?2 escape '~'",
        params![root_text.as_str(), child_pattern.as_str()],
    )?;
    if schema_table_has_column(connection, "peer_seed.", "sage_method_cache", "origin")? {
        connection.execute(
            "insert or replace into sage_method_cache(owner_type, member, origin, name, kind, module, path, start_line, start_character, end_line, end_character, detail, import_from, signature, docstring)
             select owner_type, member, origin, name, kind, module, path, start_line, start_character, end_line, end_character, detail, import_from, signature, docstring from peer_seed.sage_method_cache
             where path = ?1 or path like ?2 escape '~'",
            params![root_text.as_str(), child_pattern.as_str()],
        )?;
    } else {
        connection.execute(
            "insert or replace into sage_method_cache(owner_type, member, origin, name, kind, module, path, start_line, start_character, end_line, end_character, detail, import_from, signature, docstring)
             select owner_type, member, 'unknown', name, kind, module, path, start_line, start_character, end_line, end_character, detail, import_from, signature, docstring from peer_seed.sage_method_cache
             where path = ?1 or path like ?2 escape '~'",
            params![root_text.as_str(), child_pattern.as_str()],
        )?;
    }
    connection.execute(
        "insert or replace into index_root_metadata(root, file_count, symbol_count, doc_count, updated_at, root_fingerprint, root_marker)
         select root, file_count, symbol_count, doc_count, updated_at, root_fingerprint, root_marker from peer_seed.index_root_metadata
         where root = ?1",
        params![root_text.as_str()],
    )?;

    count_files_under_root(connection, root)
}

fn count_files_under_root(connection: &Connection, root: &Path) -> Result<usize> {
    let root_text = root.display().to_string();
    let child_pattern = like_pattern_for_children(&root_text);
    connection
        .query_row(
            "select count(*) from files where path = ?1 or path like ?2 escape '~'",
            params![root_text, child_pattern],
            |row| row.get::<_, usize>(0),
        )
        .map_err(Into::into)
}

fn load_root_fingerprint_mismatches_from_metadata(
    connection: &Connection,
    roots: &[PathBuf],
) -> Result<Vec<StaleSourceRootFingerprint>> {
    load_root_fingerprint_status_from_metadata(connection, roots).map(|(_, mismatches)| mismatches)
}

fn load_root_fingerprint_status_from_metadata(
    connection: &Connection,
    roots: &[PathBuf],
) -> Result<(Vec<SourceRootFingerprint>, Vec<StaleSourceRootFingerprint>)> {
    if roots.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }
    let Ok(mut statement) = connection
        .prepare("select root_fingerprint, root_marker from index_root_metadata where root = ?1")
    else {
        return Ok((source_root_fingerprints_for_roots(roots), Vec::new()));
    };
    let mut fingerprints = Vec::new();
    let mut mismatches = Vec::new();
    for root in roots {
        let root_text = root.display().to_string();
        let current = source_root_fingerprint(root);
        fingerprints.push(current.clone());
        let Some((cached_digest, cached_marker)) = statement
            .query_row(params![root_text], |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, Option<String>>(1)?,
                ))
            })
            .optional()?
        else {
            continue;
        };
        let Some(cached_digest) = cached_digest.filter(|digest| !digest.is_empty()) else {
            continue;
        };
        if cached_digest != current.digest {
            mismatches.push(StaleSourceRootFingerprint {
                root: root_text,
                cached_digest,
                current_digest: current.digest,
                cached_marker,
                current_marker: current.marker,
            });
        }
    }
    Ok((fingerprints, mismatches))
}

fn update_root_metadata(connection: &Connection, roots: &[PathBuf]) -> Result<()> {
    let now = UNIX_EPOCH.elapsed().unwrap_or_default().as_secs() as i64;
    let mut statement = connection.prepare(
        "insert or replace into index_root_metadata(root, file_count, symbol_count, doc_count, updated_at, root_fingerprint, root_marker) values(?1, ?2, ?3, ?4, ?5, ?6, ?7)",
    )?;
    for root in roots {
        let root_text = root.display().to_string();
        let (file_count, symbol_count, doc_count) = count_rows_under_root(connection, &root_text)?;
        let fingerprint = source_root_fingerprint(root);
        statement.execute(params![
            root_text,
            file_count as i64,
            symbol_count as i64,
            doc_count as i64,
            now,
            fingerprint.digest,
            fingerprint.marker,
        ])?;
    }
    Ok(())
}

fn metadata_deltas_for_path_refresh(
    connection: &Connection,
    changed: &[IndexedFile],
    deleted: &[PathBuf],
    roots: &[PathBuf],
) -> Result<BTreeMap<String, (i64, i64, i64)>> {
    let mut deltas = BTreeMap::<String, (i64, i64, i64)>::new();
    for path in deleted {
        if let Some(root) = root_for_path(path, roots) {
            let old = count_rows_for_path(connection, path)?;
            add_metadata_delta(&mut deltas, root, -old.0, -old.1, -old.2);
        }
    }
    for file in changed {
        if let Some(root) = root_for_path(&file.path, roots) {
            let old = count_rows_for_path(connection, &file.path)?;
            let new = counts_for_indexed_file(file);
            add_metadata_delta(
                &mut deltas,
                root,
                new.0 - old.0,
                new.1 - old.1,
                new.2 - old.2,
            );
        }
    }
    Ok(deltas)
}

fn update_root_metadata_with_deltas(
    connection: &Connection,
    roots: &[PathBuf],
    deltas: BTreeMap<String, (i64, i64, i64)>,
) -> Result<()> {
    if deltas.is_empty() {
        return Ok(());
    }
    let now = UNIX_EPOCH.elapsed().unwrap_or_default().as_secs() as i64;
    let mut update_statement = connection.prepare(
        "update index_root_metadata set file_count = max(file_count + ?2, 0), symbol_count = max(symbol_count + ?3, 0), doc_count = max(doc_count + ?4, 0), updated_at = ?5, root_fingerprint = ?6, root_marker = ?7 where root = ?1",
    )?;
    for (root_text, (file_delta, symbol_delta, doc_delta)) in deltas {
        let root = roots
            .iter()
            .find(|candidate| candidate.display().to_string() == root_text);
        let Some(root) = root else {
            continue;
        };
        let fingerprint = source_root_fingerprint(root);
        let changed = update_statement.execute(params![
            root_text,
            file_delta,
            symbol_delta,
            doc_delta,
            now,
            fingerprint.digest,
            fingerprint.marker,
        ])?;
        if changed == 0 {
            update_root_metadata(connection, std::slice::from_ref(root))?;
        }
    }
    Ok(())
}

fn add_metadata_delta(
    deltas: &mut BTreeMap<String, (i64, i64, i64)>,
    root: &Path,
    file_delta: i64,
    symbol_delta: i64,
    doc_delta: i64,
) {
    let entry = deltas
        .entry(root.display().to_string())
        .or_insert((0, 0, 0));
    entry.0 += file_delta;
    entry.1 += symbol_delta;
    entry.2 += doc_delta;
}

fn root_for_path<'a>(path: &Path, roots: &'a [PathBuf]) -> Option<&'a PathBuf> {
    roots
        .iter()
        .filter(|root| path.starts_with(root))
        .max_by_key(|root| root.components().count())
}

fn count_rows_for_path(connection: &Connection, path: &Path) -> Result<(i64, i64, i64)> {
    let path = path.display().to_string();
    let file_count = connection.query_row(
        "select count(*) from files where path = ?1",
        params![path.as_str()],
        |row| row.get::<_, i64>(0),
    )?;
    let symbol_count = connection.query_row(
        "select count(*) from symbols where path = ?1",
        params![path.as_str()],
        |row| row.get::<_, i64>(0),
    )?;
    let doc_count = connection.query_row(
        "select count(*) from docs where path = ?1 and detail != 'module'",
        params![path.as_str()],
        |row| row.get::<_, i64>(0),
    )?;
    Ok((file_count, symbol_count, doc_count))
}

fn counts_for_indexed_file(file: &IndexedFile) -> (i64, i64, i64) {
    (
        1,
        file.symbols.len() as i64,
        file.symbols
            .iter()
            .filter(|symbol| symbol.docstring.as_ref().is_some_and(|doc| !doc.is_empty()))
            .count() as i64,
    )
}

fn count_rows_under_root(connection: &Connection, root: &str) -> Result<(usize, usize, usize)> {
    let like_pattern = root_like_pattern(root);
    let file_count = connection.query_row(
        "select count(*) from files where path = ?1 or path like ?2",
        params![root, like_pattern],
        |row| row.get::<_, usize>(0),
    )?;
    let symbol_count = connection.query_row(
        "select count(*) from symbols where path = ?1 or path like ?2",
        params![root, like_pattern],
        |row| row.get::<_, usize>(0),
    )?;
    let doc_count = connection.query_row(
        "select count(*) from docs where detail != 'module' and (path = ?1 or path like ?2)",
        params![root, like_pattern],
        |row| row.get::<_, usize>(0),
    )?;
    Ok((file_count, symbol_count, doc_count))
}

fn root_like_pattern(root: &str) -> String {
    let separator = std::path::MAIN_SEPARATOR;
    if root.ends_with(separator) {
        format!("{root}%")
    } else {
        format!("{root}{separator}%")
    }
}

fn load_file_fingerprints_from_db(
    connection: &Connection,
    roots: &[PathBuf],
) -> Result<BTreeMap<PathBuf, String>> {
    let mut statement = connection.prepare("select path, fingerprint from files order by path")?;
    let rows = statement.query_map([], |row| {
        Ok((
            PathBuf::from(row.get::<_, String>(0)?),
            row.get::<_, String>(1)?,
        ))
    })?;
    let mut fingerprints = BTreeMap::new();
    for row in rows {
        let (path, fingerprint) = row?;
        if path_is_under_roots(&path, roots) {
            fingerprints.insert(path, fingerprint);
        }
    }
    Ok(fingerprints)
}

fn count_paths(connection: &Connection, table: &str, paths: &BTreeSet<String>) -> Result<usize> {
    let mut statement =
        connection.prepare(&format!("select count(*) from {table} where path = ?1"))?;
    let mut count = 0usize;
    for path in paths {
        count =
            count.saturating_add(statement.query_row(params![path], |row| row.get::<_, usize>(0))?);
    }
    Ok(count)
}

fn count_docs_for_paths(connection: &Connection, paths: &BTreeSet<String>) -> Result<usize> {
    let mut statement =
        connection.prepare("select count(*) from docs where path = ?1 and detail != 'module'")?;
    let mut count = 0usize;
    for path in paths {
        count =
            count.saturating_add(statement.query_row(params![path], |row| row.get::<_, usize>(0))?);
    }
    Ok(count)
}

fn load_file_paths_from_db(db_path: &Path, roots: &[PathBuf]) -> Result<Vec<PathBuf>> {
    let connection = Connection::open(db_path)?;
    let mut statement = connection.prepare("select path from files order by path")?;
    let rows = statement.query_map([], |row| Ok(PathBuf::from(row.get::<_, String>(0)?)))?;
    let paths = rows
        .collect::<std::result::Result<Vec<_>, _>>()?
        .into_iter()
        .filter(|path| path_is_under_roots(path, roots))
        .collect();
    Ok(paths)
}

fn load_reference_spans_from_db(
    db_path: &Path,
    name: &str,
    roots: &[PathBuf],
) -> Result<Vec<ReferenceRecord>> {
    if name.is_empty() || !db_path.exists() {
        return Ok(Vec::new());
    }
    let connection = Connection::open(db_path)?;
    create_schema(&connection)?;
    let mut statement = connection.prepare(
        "select path, start_line, start_character, end_line, end_character
         from reference_spans
         where name = ?1
         order by path, start_line, start_character",
    )?;
    let rows = statement.query_map(params![name], |row| {
        Ok(ReferenceRecord {
            path: PathBuf::from(row.get::<_, String>(0)?),
            range: SourceRange {
                start_line: row.get(1)?,
                start_character: row.get(2)?,
                end_line: row.get(3)?,
                end_character: row.get(4)?,
            },
        })
    })?;
    let mut references = Vec::new();
    for row in rows {
        let reference = row?;
        if path_is_under_roots(&reference.path, roots) {
            references.push(reference);
        }
    }
    Ok(references)
}

fn load_file_from_db(db_path: &Path, path: &Path) -> Result<IndexedFile> {
    let connection = Connection::open(db_path)?;
    let path_text = path.display().to_string();
    let module = connection
        .query_row(
            "select module from files where path = ?1",
            params![path_text],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .with_context(|| format!("indexed file not found {}", path.display()))?;
    Ok(IndexedFile {
        module,
        path: path.to_path_buf(),
        symbols: Vec::new(),
        module_docstring: None,
    })
}

fn load_symbols_by_name_from_db(
    db_path: &Path,
    name: &str,
    roots: &[PathBuf],
) -> Result<Vec<SymbolRecord>> {
    let connection = Connection::open(db_path)?;
    let mut statement = connection.prepare(
        "select name, kind, module, path, start_line, start_character, end_line, end_character, detail, import_from, signature from symbols where name = ?1 order by path, start_line, start_character",
    )?;
    let mut symbols = filter_symbols_to_roots(
        collect_symbol_rows(statement.query_map(params![name], symbol_from_row)?)?,
        roots,
    );
    attach_docstrings(&connection, &mut symbols)?;
    Ok(symbols)
}

fn load_symbols_by_name_from_db_without_docs(
    db_path: &Path,
    name: &str,
    roots: &[PathBuf],
) -> Result<Vec<SymbolRecord>> {
    let connection = Connection::open(db_path)?;
    let mut statement = connection.prepare(
        "select name, kind, module, path, start_line, start_character, end_line, end_character, detail, import_from, signature from symbols where name = ?1 order by path, start_line, start_character",
    )?;
    let symbols = collect_symbol_rows(statement.query_map(params![name], symbol_from_row)?)?;
    Ok(filter_symbols_to_roots(symbols, roots))
}

fn load_symbols_by_name_and_module_from_db(
    db_path: &Path,
    name: &str,
    module: &str,
    roots: &[PathBuf],
) -> Result<Vec<SymbolRecord>> {
    let connection = Connection::open(db_path)?;
    let mut statement = connection.prepare(
        "select s.name, s.kind, s.module, s.path, s.start_line, s.start_character, s.end_line, s.end_character, s.detail, s.import_from, s.signature, d.docstring from symbols s left join docs d on d.path = s.path and d.module = s.module and d.name = s.name and d.detail = s.detail where s.name = ?1 and s.module = ?2 order by s.path, s.start_line, s.start_character",
    )?;
    let symbols =
        collect_symbol_rows(statement.query_map(params![name, module], symbol_with_doc_from_row)?)?;
    Ok(filter_symbols_to_roots(symbols, roots))
}

fn load_symbols_by_name_and_module_from_db_without_docs(
    db_path: &Path,
    name: &str,
    module: &str,
    roots: &[PathBuf],
) -> Result<Vec<SymbolRecord>> {
    let connection = Connection::open(db_path)?;
    let mut statement = connection.prepare(
        "select name, kind, module, path, start_line, start_character, end_line, end_character, detail, import_from, signature from symbols where name = ?1 and module = ?2 order by path, start_line, start_character",
    )?;
    let symbols =
        collect_symbol_rows(statement.query_map(params![name, module], symbol_from_row)?)?;
    Ok(filter_symbols_to_roots(symbols, roots))
}

fn load_materialized_sage_export_groups_by_names_from_db(
    db_path: &Path,
    import_module: &str,
    names: &[String],
    roots: &[PathBuf],
) -> Result<HashMap<String, Vec<SymbolRecord>>> {
    if names.is_empty() {
        return Ok(HashMap::new());
    }
    let connection = Connection::open(db_path)?;
    let mut grouped = HashMap::new();
    for chunk in names.chunks(128) {
        let placeholders = std::iter::repeat_n("?", chunk.len())
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "select public_name, name, kind, module, path, start_line, start_character, end_line, end_character, detail, import_from, signature, docstring from sage_export_cache where import_module = ? and public_name in ({placeholders}) order by public_name"
        );
        let mut statement = connection.prepare(&sql)?;
        let params = std::iter::once(import_module).chain(chunk.iter().map(String::as_str));
        let rows = statement.query_map(params_from_iter(params), |row| {
            let public_name = row.get::<_, String>(0)?;
            let symbol = symbol_with_doc_from_row_offset(row, 1)?;
            Ok((public_name, symbol))
        })?;
        insert_export_rows_into_groups(rows, roots, &mut grouped)?;
    }
    Ok(grouped)
}

fn load_hot_sage_export_groups_from_db(
    db_path: &Path,
    import_module: &str,
    roots: &[PathBuf],
) -> Result<HashMap<String, Vec<SymbolRecord>>> {
    let connection = Connection::open(db_path)?;
    let mut statement = connection.prepare(
        "select public_name, name, kind, module, path, start_line, start_character, end_line, end_character, detail, import_from, signature, docstring from sage_export_cache where import_module = ?1 order by public_name limit ?2",
    )?;
    let rows = statement.query_map(
        params![import_module, MAX_DYNAMIC_HOT_EXPORT_NAMES as i64],
        |row| {
            let public_name = row.get::<_, String>(0)?;
            let symbol = symbol_with_doc_from_row_offset(row, 1)?;
            Ok((public_name, symbol))
        },
    )?;
    let mut grouped = HashMap::new();
    insert_export_rows_into_groups(rows, roots, &mut grouped)?;
    Ok(grouped)
}

fn insert_export_rows_into_groups<I>(
    rows: I,
    roots: &[PathBuf],
    grouped: &mut HashMap<String, Vec<SymbolRecord>>,
) -> Result<()>
where
    I: IntoIterator<Item = rusqlite::Result<(String, SymbolRecord)>>,
{
    for row in rows {
        let (public_name, symbol) = row?;
        if !path_is_under_roots(&symbol.path, roots) {
            continue;
        }
        grouped
            .entry(public_name.to_ascii_lowercase())
            .or_default()
            .push(symbol.clone());
        grouped
            .entry(symbol.name.to_ascii_lowercase())
            .or_default()
            .push(symbol);
    }
    Ok(())
}

fn filter_symbols_to_roots(symbols: Vec<SymbolRecord>, roots: &[PathBuf]) -> Vec<SymbolRecord> {
    symbols
        .into_iter()
        .filter(|symbol| path_is_under_roots(&symbol.path, roots))
        .collect()
}

fn load_symbols_for_path_from_db(db_path: &Path, path: &Path) -> Result<Vec<SymbolRecord>> {
    let connection = Connection::open(db_path)?;
    let path = path.display().to_string();
    let mut statement = connection.prepare(
        "select name, kind, module, path, start_line, start_character, end_line, end_character, detail, import_from, signature from symbols where path = ?1 order by start_line, start_character",
    )?;
    let mut symbols = collect_symbol_rows(statement.query_map(params![path], symbol_from_row)?)?;
    attach_docstrings(&connection, &mut symbols)?;
    Ok(symbols)
}

fn load_symbols_with_prefix_from_db(
    db_path: &Path,
    prefix: &str,
    limit: usize,
    roots: &[PathBuf],
) -> Result<Vec<SymbolRecord>> {
    if limit == 0 {
        return Ok(Vec::new());
    }
    let connection = Connection::open(db_path)?;
    let fetch_limit = limit.saturating_mul(8).max(limit).max(64) as i64;
    let mut symbols = Vec::new();
    if prefix.is_empty() {
        let mut statement = connection.prepare(
            "select name, kind, module, path, start_line, start_character, end_line, end_character, detail, import_from, signature from symbols order by name, module limit ?1",
        )?;
        symbols.extend(collect_symbol_rows(
            statement.query_map(params![fetch_limit], symbol_from_row)?,
        )?);
    } else {
        let mut range_prefixes = vec![prefix.to_string()];
        if let Some(title_prefix) = ascii_titlecase_first(prefix) {
            if title_prefix != prefix {
                range_prefixes.push(title_prefix);
            }
        }
        for range_prefix in range_prefixes {
            if let Some(upper_bound) = prefix_upper_bound(&range_prefix) {
                let mut statement = connection.prepare(
                    "select name, kind, module, path, start_line, start_character, end_line, end_character, detail, import_from, signature from symbols where name >= ?1 and name < ?2 order by name, module limit ?3",
                )?;
                symbols.extend(collect_symbol_rows(statement.query_map(
                    params![range_prefix, upper_bound, fetch_limit],
                    symbol_from_row,
                )?)?);
            } else {
                let mut statement = connection.prepare(
                    "select name, kind, module, path, start_line, start_character, end_line, end_character, detail, import_from, signature from symbols where name >= ?1 order by name, module limit ?2",
                )?;
                symbols.extend(collect_symbol_rows(
                    statement.query_map(params![range_prefix, fetch_limit], symbol_from_row)?,
                )?);
            }
        }
    }
    let prefix_lower = prefix.to_ascii_lowercase();
    let mut symbols = filter_symbols_to_roots(dedupe_symbol_records(symbols), roots)
        .into_iter()
        .filter(|symbol| {
            prefix_lower.is_empty() || symbol.name.to_ascii_lowercase().starts_with(&prefix_lower)
        })
        .collect::<Vec<_>>();
    attach_docstrings(&connection, &mut symbols)?;
    Ok(dedupe_best_symbols(symbols, limit))
}

fn prefix_upper_bound(prefix: &str) -> Option<String> {
    let mut bytes = prefix.as_bytes().to_vec();
    for index in (0..bytes.len()).rev() {
        if bytes[index] < u8::MAX {
            bytes[index] = bytes[index].saturating_add(1);
            bytes.truncate(index + 1);
            return String::from_utf8(bytes).ok();
        }
    }
    None
}

fn ascii_titlecase_first(prefix: &str) -> Option<String> {
    let mut chars = prefix.chars();
    let first = chars.next()?;
    if !first.is_ascii_lowercase() {
        return None;
    }
    let mut result = String::new();
    result.push(first.to_ascii_uppercase());
    result.push_str(chars.as_str());
    Some(result)
}

fn load_workspace_symbols_from_db(
    db_path: &Path,
    query: &str,
    limit: usize,
    roots: &[PathBuf],
) -> Result<Vec<SymbolRecord>> {
    let connection = Connection::open(db_path)?;
    let query_lower = query.to_ascii_lowercase();
    let prefix_pattern = format!("{query_lower}%");
    let contains_pattern = format!("%{query_lower}%");
    let fetch_limit = limit.max(1);
    let sql_limit = fetch_limit as i64;
    let mut statement = connection.prepare(
        "select name, kind, module, path, start_line, start_character, end_line, end_character, detail, import_from, signature
         from symbols
         where ?1 = ''
            or lower(name) = ?1
            or lower(name) like ?2
            or lower(name) like ?3
            or lower(module) like ?3
         order by
            case
              when ?1 = '' then 3
              when lower(name) = ?1 then 0
              when lower(name) like ?2 then 1
              when lower(name) like ?3 then 3
              when lower(module) like ?3 then 4
              else 5
            end,
            case kind
              when 'Class' then 0
              when 'Function' then 0
              when 'CythonDeclaration' then 0
              when 'PreparserGenerator' then 1
              when 'Variable' then 1
              when 'Module' then 2
              else 3
            end,
            length(name),
            name,
            module
         limit ?4",
    )?;
    let mut symbols = filter_symbols_to_roots(
        collect_symbol_rows(statement.query_map(
            params![query_lower, prefix_pattern, contains_pattern, sql_limit],
            symbol_from_row,
        )?)?,
        roots,
    );
    symbols.truncate(fetch_limit);
    attach_docstrings(&connection, &mut symbols)?;
    Ok(symbols)
}

fn collect_symbol_rows<I>(rows: I) -> Result<Vec<SymbolRecord>>
where
    I: IntoIterator<Item = rusqlite::Result<SymbolRecord>>,
{
    let mut symbols = Vec::new();
    for row in rows {
        symbols.push(row?);
    }
    Ok(symbols)
}

fn symbol_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SymbolRecord> {
    symbol_from_row_offset(row, 0)
}

fn symbol_from_row_offset(
    row: &rusqlite::Row<'_>,
    offset: usize,
) -> rusqlite::Result<SymbolRecord> {
    let path = PathBuf::from(row.get::<_, String>(offset + 3)?);
    Ok(SymbolRecord {
        name: row.get(offset)?,
        kind: parse_symbol_kind(&row.get::<_, String>(offset + 1)?),
        module: row.get(offset + 2)?,
        path,
        range: SourceRange {
            start_line: row.get(offset + 4)?,
            start_character: row.get(offset + 5)?,
            end_line: row.get(offset + 6)?,
            end_character: row.get(offset + 7)?,
        },
        detail: row.get(offset + 8)?,
        docstring: None,
        import_from: row.get(offset + 9)?,
        signature: row.get(offset + 10)?,
    })
}

fn symbol_with_doc_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SymbolRecord> {
    symbol_with_doc_from_row_offset(row, 0)
}

fn symbol_with_doc_from_row_offset(
    row: &rusqlite::Row<'_>,
    offset: usize,
) -> rusqlite::Result<SymbolRecord> {
    let mut symbol = symbol_from_row_offset(row, offset)?;
    symbol.docstring = row.get(offset + 11)?;
    Ok(symbol)
}

fn attach_docstrings(connection: &Connection, symbols: &mut [SymbolRecord]) -> Result<()> {
    let mut statement = connection.prepare(
        "select docstring from docs where path = ?1 and module = ?2 and name = ?3 and detail = ?4 limit 1",
    )?;
    for symbol in symbols {
        symbol.docstring = statement
            .query_row(
                params![
                    symbol.path.display().to_string(),
                    symbol.module,
                    symbol.name,
                    symbol.detail
                ],
                |row| row.get(0),
            )
            .optional()?;
    }
    Ok(())
}

fn load_runtime_documentation_from_db(
    db_path: &Path,
    symbol: &str,
) -> Result<Option<DocumentationRecord>> {
    if !db_path.exists() {
        return Ok(None);
    }
    let connection = Connection::open(db_path)?;
    create_schema(&connection)?;
    let mut statement = connection.prepare(
        "select name, module_name, kind, detail, summary, docstring, uri from runtime_docs where symbol = ?1",
    )?;
    let record = statement
        .query_row(params![symbol], |row| {
            Ok(DocumentationRecord {
                name: row.get(0)?,
                module_name: row.get(1)?,
                kind: row.get(2)?,
                detail: row.get(3)?,
                summary: row.get(4)?,
                docstring: row.get(5)?,
                uri: row.get(6)?,
                markers: vec!["runtime-writeback".to_string()],
                sections: Vec::new(),
            })
        })
        .optional()?;
    Ok(record)
}

fn upsert_runtime_documentation(
    connection: &Connection,
    symbol: &str,
    record: &DocumentationRecord,
) -> Result<()> {
    let now = UNIX_EPOCH.elapsed().unwrap_or_default().as_secs() as i64;
    connection.execute(
        "insert or replace into runtime_docs(symbol, name, module_name, kind, detail, summary, docstring, uri, updated_at) values(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            symbol,
            record.name,
            record.module_name,
            record.kind,
            record.detail,
            record.summary,
            record.docstring,
            record.uri,
            now,
        ],
    )?;
    Ok(())
}

fn load_materialized_sage_export_from_db(
    db_path: &Path,
    import_module: &str,
    name: &str,
    roots: &[PathBuf],
) -> Result<Option<SageExportResolution>> {
    let connection = Connection::open(db_path)?;
    let mut statement = connection.prepare(
        "select name, kind, module, path, start_line, start_character, end_line, end_character, detail, import_from, signature, docstring from sage_export_cache where import_module = ?1 and public_name = ?2",
    )?;
    let symbol = statement
        .query_row(params![import_module, name], symbol_with_doc_from_row)
        .optional()?;
    Ok(symbol
        .filter(|symbol| path_is_under_roots(&symbol.path, roots))
        .map(|record| SageExportResolution {
            record,
            reason: "materialized sage.all export cache",
        }))
}

fn load_materialized_sage_method_from_db(
    db_path: &Path,
    owner_type: SageOwnerType,
    member: &str,
    roots: &[PathBuf],
) -> Result<Option<SymbolRecord>> {
    let connection = Connection::open(db_path)?;
    let mut statement = connection.prepare(
        "select name, kind, module, path, start_line, start_character, end_line, end_character, detail, import_from, signature, docstring from sage_method_cache where owner_type = ?1 and member = ?2",
    )?;
    let symbol = statement
        .query_row(
            params![owner_type.as_str(), member],
            symbol_with_doc_from_row,
        )
        .optional()?;
    Ok(symbol.filter(|symbol| path_is_under_roots(&symbol.path, roots)))
}

fn load_materialized_sage_methods_from_db(
    db_path: &Path,
    keys: &[(SageOwnerType, &'static str)],
    roots: &[PathBuf],
) -> Result<Vec<(SageOwnerType, String, SymbolRecord)>> {
    let connection = Connection::open(db_path)?;
    let mut statement = connection.prepare(
        "select name, kind, module, path, start_line, start_character, end_line, end_character, detail, import_from, signature, docstring from sage_method_cache where owner_type = ?1 and member = ?2",
    )?;
    let mut records = Vec::new();
    for (owner_type, member) in keys {
        let symbol = statement
            .query_row(
                params![owner_type.as_str(), member],
                symbol_with_doc_from_row,
            )
            .optional()?;
        if let Some(symbol) = symbol.filter(|symbol| path_is_under_roots(&symbol.path, roots)) {
            records.push((*owner_type, (*member).to_string(), symbol));
        }
    }
    Ok(records)
}

fn load_materialized_sage_method_completions_from_db(
    db_path: &Path,
    owner_type: SageOwnerType,
    prefix: &str,
    roots: &[PathBuf],
    limit: usize,
) -> Result<Vec<(String, SymbolRecord)>> {
    let connection = Connection::open(db_path)?;
    let like_pattern = format!("{prefix}%");
    let mut statement = connection.prepare(
        "select member, name, kind, module, path, start_line, start_character, end_line, end_character, detail, import_from, signature, docstring from sage_method_cache where owner_type = ?1 and member like ?2 order by member limit ?3",
    )?;
    let rows = statement.query_map(
        params![owner_type.as_str(), like_pattern, limit.saturating_mul(2)],
        |row| {
            let member: String = row.get(0)?;
            let symbol = symbol_with_doc_from_row_offset(row, 1)?;
            Ok((member, symbol))
        },
    )?;
    let mut completions = Vec::new();
    for row in rows {
        let (member, symbol) = row?;
        if path_is_under_roots(&symbol.path, roots) {
            completions.push((member, symbol));
        }
        if completions.len() >= limit {
            break;
        }
    }
    Ok(completions)
}

fn load_sage_method_cache_stats_from_db(db_path: &Path) -> Result<SageMethodCacheStats> {
    let connection = Connection::open(db_path)?;
    let mut statement =
        connection.prepare("select coalesce(origin, 'unknown'), count(*) from sage_method_cache group by coalesce(origin, 'unknown')")?;
    let rows = statement.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, usize>(1)?))
    })?;
    let mut stats = SageMethodCacheStats::default();
    for row in rows {
        let (origin, count) = row?;
        stats.total += count;
        match origin.as_str() {
            METHOD_CACHE_ORIGIN_SOURCE_DERIVED => stats.source_derived += count,
            _ => stats.static_fallback += count,
        }
    }
    Ok(stats)
}

fn refresh_materialized_caches(connection: &Connection, roots: &[PathBuf]) -> Result<()> {
    connection.execute("delete from sage_export_cache", [])?;
    connection.execute("delete from sage_method_cache", [])?;
    refresh_materialized_export_cache(connection, roots)?;
    refresh_materialized_method_cache(connection, roots)?;
    Ok(())
}

fn refresh_materialized_caches_from_symbols(
    connection: &Connection,
    symbols_by_name: &HashMap<String, Vec<SymbolRecord>>,
) -> Result<()> {
    connection.execute("delete from sage_export_cache", [])?;
    connection.execute("delete from sage_method_cache", [])?;
    refresh_materialized_export_cache_from_symbols(connection, symbols_by_name)?;
    refresh_materialized_method_cache_from_symbols(connection, symbols_by_name)?;
    Ok(())
}

fn refresh_materialized_export_cache_from_symbols(
    connection: &Connection,
    symbols_by_name: &HashMap<String, Vec<SymbolRecord>>,
) -> Result<()> {
    let mut statement = connection.prepare(
        "insert or replace into sage_export_cache(public_name, source_name, import_module, reason, name, kind, module, path, start_line, start_character, end_line, end_character, detail, import_from, signature, docstring) values(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
    )?;
    let mut exports_by_module = BTreeMap::<String, BTreeMap<String, SymbolRecord>>::new();
    for import_symbol in symbols_by_name
        .values()
        .flat_map(|symbols| symbols.iter())
        .filter(|symbol| {
            symbol.kind == SymbolKind::Import && module_is_sage_all_export_module(&symbol.module)
        })
    {
        if is_star_import_symbol(import_symbol) || is_all_export_symbol(import_symbol) {
            continue;
        }
        if let Some(record) = resolve_import_symbol_from_symbol_map(
            symbols_by_name,
            import_symbol,
            0,
            &mut BTreeSet::new(),
        ) {
            insert_export_cache_row(
                &mut statement,
                &import_symbol.name,
                &import_symbol.module,
                "indexed sage.all re-export chain",
                &record,
            )?;
            exports_by_module
                .entry(import_symbol.module.clone())
                .or_default()
                .entry(import_symbol.name.clone())
                .or_insert(record);
        }
    }
    let star_edges = sage_all_star_import_edges_from_symbol_map(symbols_by_name);
    populate_star_source_exports_from_symbol_map(
        &mut exports_by_module,
        symbols_by_name,
        &star_edges,
    );
    insert_star_re_exports_from_modules(&mut statement, &mut exports_by_module, &star_edges)?;
    insert_static_sage_export_fallbacks_from_symbol_map(
        &mut statement,
        &mut exports_by_module,
        symbols_by_name,
    )?;
    Ok(())
}

fn refresh_materialized_method_cache_from_symbols(
    connection: &Connection,
    symbols_by_name: &HashMap<String, Vec<SymbolRecord>>,
) -> Result<()> {
    let mut statement = connection.prepare(
        "insert or replace into sage_method_cache(owner_type, member, origin, name, kind, module, path, start_line, start_character, end_line, end_character, detail, import_from, signature, docstring) values(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
    )?;
    let mut source_derived_keys = BTreeSet::new();
    for (owner_type, member, record) in source_derived_method_records_from_symbols(symbols_by_name)
    {
        source_derived_keys.insert((owner_type, member.clone()));
        insert_method_cache_row(
            &mut statement,
            owner_type,
            &member,
            METHOD_CACHE_ORIGIN_SOURCE_DERIVED,
            &record,
        )?;
    }
    for spec in SAGE_METHOD_SPECS {
        if source_derived_keys.contains(&(spec.owner_type, spec.member.to_string())) {
            continue;
        }
        if let Some(record) = best_symbol_by_name_and_module_from_symbol_map(
            symbols_by_name,
            spec.member,
            spec.module,
        ) {
            insert_method_cache_row(
                &mut statement,
                spec.owner_type,
                spec.member,
                METHOD_CACHE_ORIGIN_STATIC_SPEC,
                &record,
            )?;
        }
    }
    for spec in SAGE_METHOD_ALIAS_SPECS {
        if source_derived_keys.contains(&(spec.owner_type, spec.member.to_string())) {
            continue;
        }
        if let Some(record) = best_symbol_by_name_and_module_from_symbol_map(
            symbols_by_name,
            spec.source_name,
            spec.module,
        ) {
            insert_method_cache_row(
                &mut statement,
                spec.owner_type,
                spec.member,
                METHOD_CACHE_ORIGIN_STATIC_ALIAS,
                &record,
            )?;
        }
    }
    Ok(())
}

fn best_symbol_by_name_and_module_from_symbol_map(
    symbols_by_name: &HashMap<String, Vec<SymbolRecord>>,
    name: &str,
    module: &str,
) -> Option<SymbolRecord> {
    symbols_by_name
        .get(&name.to_ascii_lowercase())?
        .iter()
        .filter(|symbol| import_target_definition_matches(symbol, module, name))
        .min_by_key(|symbol| symbol_choice_key(symbol))
        .cloned()
}

fn source_derived_method_records_from_symbols(
    symbols_by_name: &HashMap<String, Vec<SymbolRecord>>,
) -> Vec<(SageOwnerType, String, SymbolRecord)> {
    let mut best = BTreeMap::<(SageOwnerType, String), (SageMethodChoiceKey, SymbolRecord)>::new();
    for symbol in symbols_by_name.values().flat_map(|symbols| symbols.iter()) {
        let Some(owner) = source_derived_method_owner_for_symbol(symbol) else {
            continue;
        };
        let key = (owner.owner_type, symbol.name.clone());
        let choice_key = sage_method_choice_key(owner.priority, symbol);
        match best.get(&key) {
            Some((existing_key, _)) if *existing_key <= choice_key => {}
            _ => {
                best.insert(key, (choice_key, symbol.clone()));
            }
        }
    }
    for (owner_type, member, record) in
        source_derived_method_alias_records_from_symbols(symbols_by_name)
    {
        let key = (owner_type, member);
        let choice_key = sage_method_choice_key(0, &record);
        match best.get(&key) {
            Some((existing_key, _)) if *existing_key <= choice_key => {}
            _ => {
                best.insert(key, (choice_key, record));
            }
        }
    }
    best.into_iter()
        .map(|((owner_type, member), (_, record))| (owner_type, member, record))
        .collect()
}

fn source_derived_method_alias_records_from_symbols(
    symbols_by_name: &HashMap<String, Vec<SymbolRecord>>,
) -> Vec<(SageOwnerType, String, SymbolRecord)> {
    let mut records: Vec<_> = symbols_by_name
        .values()
        .flat_map(|symbols| symbols.iter())
        .filter(|symbol| symbol.kind == SymbolKind::Import)
        .filter_map(|alias_symbol| {
            let (class_name, alias, target) =
                class_method_alias_detail_parts(&alias_symbol.detail)?;
            let owner_type = sage_owner_type_from_class_name(class_name, &alias_symbol.module)?;
            let target_detail = format!("Method {class_name}.{target}");
            let target_record = symbols_by_name
                .get(&target.to_ascii_lowercase())?
                .iter()
                .filter(|symbol| {
                    symbol.module == alias_symbol.module
                        && symbol.detail == target_detail
                        && is_source_derived_sage_method(symbol)
                })
                .min_by_key(|symbol| symbol_choice_key(symbol))
                .cloned()?;
            Some((owner_type, alias.to_string(), target_record))
        })
        .collect();
    records.extend(
        symbols_by_name
            .values()
            .flat_map(|symbols| symbols.iter())
            .filter(|symbol| symbol.kind == SymbolKind::Import)
            .filter_map(|alias_symbol| {
                let (alias, target) =
                    matrix_constructor_method_alias_detail_parts(&alias_symbol.detail)?;
                let target_record = symbols_by_name
                    .get(&target.to_ascii_lowercase())?
                    .iter()
                    .filter(|symbol| {
                        symbol.module == alias_symbol.module
                            && symbol.name == target
                            && symbol.kind != SymbolKind::Import
                    })
                    .min_by_key(|symbol| symbol_choice_key(symbol))
                    .cloned()?;
                Some((
                    SageOwnerType::MatrixConstructor,
                    alias.to_string(),
                    target_record,
                ))
            }),
    );
    records
}

fn sage_all_star_import_edges_from_symbol_map(
    symbols_by_name: &HashMap<String, Vec<SymbolRecord>>,
) -> Vec<(String, String)> {
    symbols_by_name
        .values()
        .flat_map(|symbols| symbols.iter())
        .filter(|symbol| module_is_sage_all_export_module(&symbol.module))
        .filter_map(|symbol| {
            star_import_source_module(symbol)
                .map(|source_module| (symbol.module.clone(), source_module))
        })
        .collect()
}

fn sage_all_star_import_edges_from_symbols(symbols: &[SymbolRecord]) -> Vec<(String, String)> {
    symbols
        .iter()
        .filter(|symbol| module_is_sage_all_export_module(&symbol.module))
        .filter_map(|symbol| {
            star_import_source_module(symbol)
                .map(|source_module| (symbol.module.clone(), source_module))
        })
        .collect()
}

fn insert_star_re_exports_from_modules(
    statement: &mut rusqlite::Statement<'_>,
    exports_by_module: &mut BTreeMap<String, BTreeMap<String, SymbolRecord>>,
    star_edges: &[(String, String)],
) -> Result<()> {
    for _ in 0..MAX_IMPORT_RESOLUTION_DEPTH {
        let mut pending = Vec::new();
        for (import_module, source_module) in star_edges {
            let Some(source_exports) = exports_by_module.get(source_module) else {
                continue;
            };
            for (public_name, record) in source_exports {
                if exports_by_module
                    .get(import_module)
                    .is_some_and(|exports| exports.contains_key(public_name))
                {
                    continue;
                }
                pending.push((import_module.clone(), public_name.clone(), record.clone()));
            }
        }
        if pending.is_empty() {
            break;
        }
        for (import_module, public_name, record) in pending {
            let inserted = exports_by_module
                .entry(import_module.clone())
                .or_default()
                .insert(public_name.clone(), record.clone())
                .is_none();
            if inserted {
                insert_export_cache_row(
                    statement,
                    &public_name,
                    &import_module,
                    "indexed sage.all star re-export",
                    &record,
                )?;
            }
        }
    }
    Ok(())
}

fn populate_star_source_exports_from_connection(
    connection: &Connection,
    roots: &[PathBuf],
    exports_by_module: &mut BTreeMap<String, BTreeMap<String, SymbolRecord>>,
    star_edges: &[(String, String)],
) -> Result<()> {
    for source_module in star_edges.iter().map(|(_, source)| source) {
        if exports_by_module.contains_key(source_module) {
            continue;
        }
        let exports = public_module_exports_from_connection(connection, roots, source_module)?;
        if !exports.is_empty() {
            exports_by_module.insert(source_module.clone(), exports);
        }
    }
    Ok(())
}

fn public_module_exports_from_connection(
    connection: &Connection,
    roots: &[PathBuf],
    module: &str,
) -> Result<BTreeMap<String, SymbolRecord>> {
    let symbols = load_symbols_by_module_from_connection(connection, module, roots)?;
    let explicit_names = explicit_all_names_from_symbols(symbols.iter());
    let mut exports = BTreeMap::<String, (SageMethodChoiceKey, SymbolRecord)>::new();
    for symbol in symbols {
        if !is_star_namespace_export_candidate(&symbol, explicit_names.as_ref()) {
            continue;
        }
        let record = if symbol.kind == SymbolKind::Import {
            resolve_import_symbol_from_connection(
                connection,
                &symbol,
                roots,
                0,
                &mut BTreeSet::new(),
            )?
        } else {
            Some(symbol.clone())
        };
        let Some(record) = record else {
            continue;
        };
        let key = sage_method_choice_key(0, &record);
        match exports.get(&symbol.name) {
            Some((existing_key, _)) if *existing_key <= key => {}
            _ => {
                exports.insert(symbol.name.clone(), (key, record));
            }
        }
    }
    Ok(exports
        .into_iter()
        .map(|(public_name, (_, record))| (public_name, record))
        .collect())
}

fn populate_star_source_exports_from_symbol_map(
    exports_by_module: &mut BTreeMap<String, BTreeMap<String, SymbolRecord>>,
    symbols_by_name: &HashMap<String, Vec<SymbolRecord>>,
    star_edges: &[(String, String)],
) {
    for source_module in star_edges.iter().map(|(_, source)| source) {
        if exports_by_module.contains_key(source_module) {
            continue;
        }
        let exports = public_module_exports_from_symbol_map(symbols_by_name, source_module);
        if !exports.is_empty() {
            exports_by_module.insert(source_module.clone(), exports);
        }
    }
}

fn public_module_exports_from_symbol_map(
    symbols_by_name: &HashMap<String, Vec<SymbolRecord>>,
    module: &str,
) -> BTreeMap<String, SymbolRecord> {
    let module_symbols: Vec<&SymbolRecord> = symbols_by_name
        .values()
        .flat_map(|symbols| symbols.iter())
        .filter(|symbol| symbol.module == module)
        .collect();
    let explicit_names = explicit_all_names_from_symbols(module_symbols.iter().copied());
    let mut exports = BTreeMap::<String, (SageMethodChoiceKey, SymbolRecord)>::new();
    for symbol in module_symbols {
        if !is_star_namespace_export_candidate(symbol, explicit_names.as_ref()) {
            continue;
        }
        let record = if symbol.kind == SymbolKind::Import {
            resolve_import_symbol_from_symbol_map(symbols_by_name, symbol, 0, &mut BTreeSet::new())
        } else {
            Some(symbol.clone())
        };
        let Some(record) = record else {
            continue;
        };
        let key = sage_method_choice_key(0, &record);
        match exports.get(&symbol.name) {
            Some((existing_key, _)) if *existing_key <= key => {}
            _ => {
                exports.insert(symbol.name.clone(), (key, record));
            }
        }
    }
    exports
        .into_iter()
        .map(|(public_name, (_, record))| (public_name, record))
        .collect()
}

fn insert_static_sage_export_fallbacks_from_symbol_map(
    statement: &mut rusqlite::Statement<'_>,
    exports_by_module: &mut BTreeMap<String, BTreeMap<String, SymbolRecord>>,
    symbols_by_name: &HashMap<String, Vec<SymbolRecord>>,
) -> Result<()> {
    for target in SAGE_EXPORT_MAP {
        if exports_by_module
            .get(target.import_module)
            .is_some_and(|exports| exports.contains_key(target.name))
        {
            continue;
        }
        let Some(record) = best_symbol_by_name_and_module_from_symbol_map(
            symbols_by_name,
            target.source_name,
            target.source_module,
        ) else {
            continue;
        };
        insert_export_cache_row(
            statement,
            target.name,
            target.import_module,
            "built-in sage.all export fallback",
            &record,
        )?;
        exports_by_module
            .entry(target.import_module.to_string())
            .or_default()
            .insert(target.name.to_string(), record);
    }
    Ok(())
}

fn resolve_import_symbol_from_symbol_map(
    symbols_by_name: &HashMap<String, Vec<SymbolRecord>>,
    symbol: &SymbolRecord,
    depth: usize,
    seen: &mut BTreeSet<String>,
) -> Option<SymbolRecord> {
    if symbol.kind != SymbolKind::Import || depth >= MAX_IMPORT_RESOLUTION_DEPTH {
        return None;
    }
    let import_from = symbol.import_from.as_ref()?;
    let (source_module, source_name) =
        import_target_in_context(import_from, &symbol.name, &symbol.module);
    if !seen.insert(format!("{source_module}::{source_name}")) {
        return None;
    }
    let candidates = symbols_by_name.get(&source_name.to_ascii_lowercase())?;
    if let Some(definition) = candidates
        .iter()
        .filter(|candidate| {
            import_target_definition_matches(candidate, &source_module, &source_name)
        })
        .min_by_key(|candidate| symbol_choice_key(candidate))
        .cloned()
    {
        return Some(definition);
    }
    let next_import = candidates
        .iter()
        .filter(|candidate| {
            candidate.kind == SymbolKind::Import
                && candidate.name == source_name
                && module_matches_import(&candidate.module, &source_module)
        })
        .min_by_key(|candidate| symbol_choice_key(candidate))?;
    resolve_import_symbol_from_symbol_map(symbols_by_name, next_import, depth + 1, seen)
        .or_else(|| Some(next_import.clone()))
}

fn insert_method_cache_row(
    statement: &mut rusqlite::Statement<'_>,
    owner_type: SageOwnerType,
    member: &str,
    origin: &str,
    record: &SymbolRecord,
) -> Result<()> {
    statement.execute(params![
        owner_type.as_str(),
        member,
        origin,
        record.name.as_str(),
        symbol_kind_as_str(&record.kind),
        record.module.as_str(),
        record.path.display().to_string(),
        record.range.start_line,
        record.range.start_character,
        record.range.end_line,
        record.range.end_character,
        record.detail.as_str(),
        record.import_from.as_deref(),
        record.signature.as_deref(),
        record.docstring.as_deref(),
    ])?;
    Ok(())
}

fn insert_export_cache_row(
    statement: &mut rusqlite::Statement<'_>,
    public_name: &str,
    import_module: &str,
    reason: &str,
    record: &SymbolRecord,
) -> Result<()> {
    statement.execute(params![
        public_name,
        record.name.as_str(),
        import_module,
        reason,
        record.name.as_str(),
        symbol_kind_as_str(&record.kind),
        record.module.as_str(),
        record.path.display().to_string(),
        record.range.start_line,
        record.range.start_character,
        record.range.end_line,
        record.range.end_character,
        record.detail.as_str(),
        record.import_from.as_deref(),
        record.signature.as_deref(),
        record.docstring.as_deref(),
    ])?;
    Ok(())
}

fn refresh_materialized_export_cache(connection: &Connection, roots: &[PathBuf]) -> Result<()> {
    let dynamic_imports = load_sage_export_imports_from_connection(connection, roots)?;
    let mut statement = connection.prepare(
        "insert or replace into sage_export_cache(public_name, source_name, import_module, reason, name, kind, module, path, start_line, start_character, end_line, end_character, detail, import_from, signature, docstring) values(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
    )?;
    let mut exports_by_module = BTreeMap::<String, BTreeMap<String, SymbolRecord>>::new();
    for import_symbol in &dynamic_imports {
        if is_star_import_symbol(import_symbol) || is_all_export_symbol(import_symbol) {
            continue;
        }
        if let Some(record) = resolve_import_symbol_from_connection(
            connection,
            import_symbol,
            roots,
            0,
            &mut BTreeSet::new(),
        )? {
            insert_export_cache_row(
                &mut statement,
                &import_symbol.name,
                &import_symbol.module,
                "indexed sage.all re-export chain",
                &record,
            )?;
            exports_by_module
                .entry(import_symbol.module.clone())
                .or_default()
                .entry(import_symbol.name.clone())
                .or_insert(record);
        }
    }
    let star_edges = sage_all_star_import_edges_from_symbols(&dynamic_imports);
    populate_star_source_exports_from_connection(
        connection,
        roots,
        &mut exports_by_module,
        &star_edges,
    )?;
    insert_star_re_exports_from_modules(&mut statement, &mut exports_by_module, &star_edges)?;
    insert_static_sage_export_fallbacks_from_connection(
        connection,
        roots,
        &mut statement,
        &mut exports_by_module,
    )?;
    Ok(())
}

fn insert_static_sage_export_fallbacks_from_connection(
    connection: &Connection,
    roots: &[PathBuf],
    statement: &mut rusqlite::Statement<'_>,
    exports_by_module: &mut BTreeMap<String, BTreeMap<String, SymbolRecord>>,
) -> Result<()> {
    for target in SAGE_EXPORT_MAP {
        if exports_by_module
            .get(target.import_module)
            .is_some_and(|exports| exports.contains_key(target.name))
        {
            continue;
        }
        let Some(record) = load_best_symbol_by_name_and_module_from_connection(
            connection,
            target.source_name,
            target.source_module,
            roots,
        )?
        else {
            continue;
        };
        insert_export_cache_row(
            statement,
            target.name,
            target.import_module,
            "built-in sage.all export fallback",
            &record,
        )?;
        exports_by_module
            .entry(target.import_module.to_string())
            .or_default()
            .insert(target.name.to_string(), record);
    }
    Ok(())
}

fn refresh_materialized_method_cache(connection: &Connection, roots: &[PathBuf]) -> Result<()> {
    let mut statement = connection.prepare(
        "insert or replace into sage_method_cache(owner_type, member, origin, name, kind, module, path, start_line, start_character, end_line, end_character, detail, import_from, signature, docstring) values(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
    )?;
    let mut source_derived_keys = BTreeSet::new();
    for (owner_type, member, record) in
        source_derived_method_records_from_connection(connection, roots)?
    {
        source_derived_keys.insert((owner_type, member.clone()));
        insert_method_cache_row(
            &mut statement,
            owner_type,
            &member,
            METHOD_CACHE_ORIGIN_SOURCE_DERIVED,
            &record,
        )?;
    }
    for spec in SAGE_METHOD_SPECS {
        if source_derived_keys.contains(&(spec.owner_type, spec.member.to_string())) {
            continue;
        }
        if let Some(record) = load_best_symbol_by_name_and_module_from_connection(
            connection,
            spec.member,
            spec.module,
            roots,
        )? {
            insert_method_cache_row(
                &mut statement,
                spec.owner_type,
                spec.member,
                METHOD_CACHE_ORIGIN_STATIC_SPEC,
                &record,
            )?;
        }
    }
    for spec in SAGE_METHOD_ALIAS_SPECS {
        if source_derived_keys.contains(&(spec.owner_type, spec.member.to_string())) {
            continue;
        }
        if let Some(record) = load_best_symbol_by_name_and_module_from_connection(
            connection,
            spec.source_name,
            spec.module,
            roots,
        )? {
            insert_method_cache_row(
                &mut statement,
                spec.owner_type,
                spec.member,
                METHOD_CACHE_ORIGIN_STATIC_ALIAS,
                &record,
            )?;
        }
    }
    Ok(())
}

fn source_derived_method_records_from_connection(
    connection: &Connection,
    roots: &[PathBuf],
) -> Result<Vec<(SageOwnerType, String, SymbolRecord)>> {
    let mut best = BTreeMap::<(SageOwnerType, String), (SageMethodChoiceKey, SymbolRecord)>::new();
    for symbol in load_class_context_method_symbols_from_connection(connection, roots)? {
        let Some(owner) = source_derived_method_owner_for_symbol(&symbol) else {
            continue;
        };
        let key = (owner.owner_type, symbol.name.clone());
        let choice_key = sage_method_choice_key(owner.priority, &symbol);
        match best.get(&key) {
            Some((existing_key, _)) if *existing_key <= choice_key => {}
            _ => {
                best.insert(key, (choice_key, symbol));
            }
        }
    }
    for alias_symbol in load_class_method_alias_symbols_from_connection(connection, roots)? {
        let Some((class_name, alias, target)) =
            class_method_alias_detail_parts(&alias_symbol.detail)
        else {
            continue;
        };
        let Some(owner_type) = sage_owner_type_from_class_name(class_name, &alias_symbol.module)
        else {
            continue;
        };
        let Some(record) = load_class_method_alias_target_from_connection(
            connection,
            roots,
            &alias_symbol.module,
            class_name,
            target,
        )?
        else {
            continue;
        };
        let key = (owner_type, alias.to_string());
        let choice_key = sage_method_choice_key(0, &record);
        match best.get(&key) {
            Some((existing_key, _)) if *existing_key <= choice_key => {}
            _ => {
                best.insert(key, (choice_key, record));
            }
        }
    }
    for alias_symbol in
        load_matrix_constructor_method_alias_symbols_from_connection(connection, roots)?
    {
        let Some((alias, target)) =
            matrix_constructor_method_alias_detail_parts(&alias_symbol.detail)
        else {
            continue;
        };
        let Some(record) = load_best_symbol_by_name_and_module_from_connection(
            connection,
            target,
            &alias_symbol.module,
            roots,
        )?
        else {
            continue;
        };
        let key = (SageOwnerType::MatrixConstructor, alias.to_string());
        let choice_key = sage_method_choice_key(0, &record);
        match best.get(&key) {
            Some((existing_key, _)) if *existing_key <= choice_key => {}
            _ => {
                best.insert(key, (choice_key, record));
            }
        }
    }
    for module_spec in SAGE_OWNER_METHOD_MODULES {
        let mut symbols =
            load_method_like_symbols_for_owner_module(connection, module_spec, roots)?;
        for symbol in symbols.drain(..) {
            let key = (module_spec.owner_type, symbol.name.clone());
            let choice_key = sage_method_choice_key(module_spec.priority, &symbol);
            match best.get(&key) {
                Some((existing_key, _)) if *existing_key <= choice_key => {}
                _ => {
                    best.insert(key, (choice_key, symbol));
                }
            }
        }
    }
    Ok(best
        .into_iter()
        .map(|((owner_type, member), (_, record))| (owner_type, member, record))
        .collect())
}

fn load_class_context_method_symbols_from_connection(
    connection: &Connection,
    roots: &[PathBuf],
) -> Result<Vec<SymbolRecord>> {
    let mut statement = connection.prepare(
        "select s.name, s.kind, s.module, s.path, s.start_line, s.start_character, s.end_line, s.end_character, s.detail, s.import_from, s.signature, d.docstring from symbols s left join docs d on d.path = s.path and d.module = s.module and d.name = s.name and d.detail = s.detail where s.kind != 'Import' and s.signature is not null and s.detail like 'Method %' order by s.module, s.name",
    )?;
    let symbols = collect_symbol_rows(statement.query_map([], symbol_with_doc_from_row)?)?
        .into_iter()
        .filter(|symbol| path_is_under_roots(&symbol.path, roots))
        .filter(|symbol| source_derived_method_owner_for_symbol(symbol).is_some())
        .collect();
    Ok(symbols)
}

fn load_class_method_alias_symbols_from_connection(
    connection: &Connection,
    roots: &[PathBuf],
) -> Result<Vec<SymbolRecord>> {
    let mut statement = connection.prepare(
        "select name, kind, module, path, start_line, start_character, end_line, end_character, detail, import_from, signature from symbols where kind = 'Import' and detail like 'MethodAlias %' order by module, name",
    )?;
    let symbols = collect_symbol_rows(statement.query_map([], symbol_from_row)?)?
        .into_iter()
        .filter(|symbol| path_is_under_roots(&symbol.path, roots))
        .collect();
    Ok(symbols)
}

fn load_matrix_constructor_method_alias_symbols_from_connection(
    connection: &Connection,
    roots: &[PathBuf],
) -> Result<Vec<SymbolRecord>> {
    let mut statement = connection.prepare(
        "select name, kind, module, path, start_line, start_character, end_line, end_character, detail, import_from, signature from symbols where kind = 'Import' and detail like 'MatrixConstructorMethodAlias %' order by module, name",
    )?;
    let symbols = collect_symbol_rows(statement.query_map([], symbol_from_row)?)?
        .into_iter()
        .filter(|symbol| path_is_under_roots(&symbol.path, roots))
        .collect();
    Ok(symbols)
}

fn load_class_method_alias_target_from_connection(
    connection: &Connection,
    roots: &[PathBuf],
    module: &str,
    class_name: &str,
    target: &str,
) -> Result<Option<SymbolRecord>> {
    let target_detail = format!("Method {class_name}.{target}");
    let mut statement = connection.prepare(
        "select s.name, s.kind, s.module, s.path, s.start_line, s.start_character, s.end_line, s.end_character, s.detail, s.import_from, s.signature, d.docstring from symbols s left join docs d on d.path = s.path and d.module = s.module and d.name = s.name and d.detail = s.detail where s.module = ?1 and s.name = ?2 and s.detail = ?3 and s.signature is not null order by s.path, s.start_line, s.start_character",
    )?;
    let symbols = collect_symbol_rows(statement.query_map(
        params![module, target, target_detail],
        symbol_with_doc_from_row,
    )?)?;
    Ok(symbols
        .into_iter()
        .filter(|symbol| {
            path_is_under_roots(&symbol.path, roots) && is_source_derived_sage_method(symbol)
        })
        .min_by_key(symbol_choice_key))
}

fn load_method_like_symbols_for_owner_module(
    connection: &Connection,
    module_spec: &SageOwnerModuleSpec,
    roots: &[PathBuf],
) -> Result<Vec<SymbolRecord>> {
    let mut statement = if module_spec.recursive {
        connection.prepare(
            "select s.name, s.kind, s.module, s.path, s.start_line, s.start_character, s.end_line, s.end_character, s.detail, s.import_from, s.signature, d.docstring from symbols s left join docs d on d.path = s.path and d.module = s.module and d.name = s.name and d.detail = s.detail where s.kind != 'Import' and s.signature is not null and (s.module = ?1 or s.module like ?2) order by s.module, s.name",
        )?
    } else {
        connection.prepare(
            "select s.name, s.kind, s.module, s.path, s.start_line, s.start_character, s.end_line, s.end_character, s.detail, s.import_from, s.signature, d.docstring from symbols s left join docs d on d.path = s.path and d.module = s.module and d.name = s.name and d.detail = s.detail where s.kind != 'Import' and s.signature is not null and s.module = ?1 order by s.module, s.name",
        )?
    };
    let module_pattern = format!("{}.%", module_spec.module);
    let rows = if module_spec.recursive {
        statement.query_map(
            params![module_spec.module, module_pattern],
            symbol_with_doc_from_row,
        )?
    } else {
        statement.query_map(params![module_spec.module], symbol_with_doc_from_row)?
    };
    let symbols = collect_symbol_rows(rows)?
        .into_iter()
        .filter(|symbol| {
            path_is_under_roots(&symbol.path, roots) && is_source_derived_sage_method(symbol)
        })
        .collect();
    Ok(symbols)
}

fn load_sage_export_imports_from_connection(
    connection: &Connection,
    roots: &[PathBuf],
) -> Result<Vec<SymbolRecord>> {
    let mut statement = connection.prepare(
        "select name, kind, module, path, start_line, start_character, end_line, end_character, detail, import_from, signature from symbols where kind = 'Import' and (module = 'sage.all' or module like 'sage.%.all') order by module, name",
    )?;
    let symbols = collect_symbol_rows(statement.query_map([], symbol_from_row)?)?;
    Ok(filter_symbols_to_roots(symbols, roots))
}

fn load_best_symbol_by_name_and_module_from_connection(
    connection: &Connection,
    name: &str,
    module: &str,
    roots: &[PathBuf],
) -> Result<Option<SymbolRecord>> {
    let symbols = load_symbols_by_name_from_connection(connection, name, roots)?;
    Ok(symbols
        .into_iter()
        .filter(|symbol| import_target_definition_matches(symbol, module, name))
        .min_by_key(symbol_choice_key))
}

fn load_symbols_by_module_from_connection(
    connection: &Connection,
    module: &str,
    roots: &[PathBuf],
) -> Result<Vec<SymbolRecord>> {
    let mut statement = connection.prepare(
        "select s.name, s.kind, s.module, s.path, s.start_line, s.start_character, s.end_line, s.end_character, s.detail, s.import_from, s.signature, d.docstring from symbols s left join docs d on d.path = s.path and d.module = s.module and d.name = s.name and d.detail = s.detail where s.module = ?1 order by s.name, s.path, s.start_line, s.start_character",
    )?;
    let symbols =
        collect_symbol_rows(statement.query_map(params![module], symbol_with_doc_from_row)?)?;
    Ok(filter_symbols_to_roots(symbols, roots))
}

fn load_symbols_by_name_from_connection(
    connection: &Connection,
    name: &str,
    roots: &[PathBuf],
) -> Result<Vec<SymbolRecord>> {
    let mut statement = connection.prepare(
        "select s.name, s.kind, s.module, s.path, s.start_line, s.start_character, s.end_line, s.end_character, s.detail, s.import_from, s.signature, d.docstring from symbols s left join docs d on d.path = s.path and d.module = s.module and d.name = s.name and d.detail = s.detail where s.name = ?1 order by s.path, s.start_line, s.start_character",
    )?;
    let symbols =
        collect_symbol_rows(statement.query_map(params![name], symbol_with_doc_from_row)?)?;
    Ok(filter_symbols_to_roots(symbols, roots))
}

fn resolve_import_symbol_from_connection(
    connection: &Connection,
    symbol: &SymbolRecord,
    roots: &[PathBuf],
    depth: usize,
    seen: &mut BTreeSet<String>,
) -> Result<Option<SymbolRecord>> {
    if symbol.kind != SymbolKind::Import || depth >= MAX_IMPORT_RESOLUTION_DEPTH {
        return Ok(None);
    }
    let Some(import_from) = symbol.import_from.as_ref() else {
        return Ok(None);
    };
    let (source_module, source_name) =
        import_target_in_context(import_from, &symbol.name, &symbol.module);
    if !seen.insert(format!("{source_module}::{source_name}")) {
        return Ok(None);
    }
    let candidates = load_symbols_by_name_from_connection(connection, &source_name, roots)?;
    if let Some(definition) = candidates
        .iter()
        .filter(|candidate| {
            import_target_definition_matches(candidate, &source_module, &source_name)
        })
        .min_by_key(|candidate| symbol_choice_key(candidate))
        .cloned()
    {
        return Ok(Some(definition));
    }
    let Some(next_import) = candidates
        .iter()
        .filter(|candidate| {
            candidate.kind == SymbolKind::Import
                && candidate.name == source_name
                && module_matches_import(&candidate.module, &source_module)
        })
        .min_by_key(|candidate| symbol_choice_key(candidate))
        .cloned()
    else {
        return Ok(None);
    };
    Ok(
        resolve_import_symbol_from_connection(connection, &next_import, roots, depth + 1, seen)?
            .or(Some(next_import)),
    )
}

fn persist_file(
    connection: &Connection,
    file: &IndexedFile,
    persist_reference_spans: bool,
) -> Result<()> {
    let path = file.path.display().to_string();
    delete_path_from_db(connection, &path)?;
    let mut file_statement =
        connection.prepare("insert into files(path, module, fingerprint) values(?1, ?2, ?3)")?;
    let mut symbol_statement = connection.prepare(
        "insert into symbols(name, kind, module, path, start_line, start_character, end_line, end_character, detail, import_from, signature) values(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
    )?;
    let mut doc_statement = connection.prepare(
        "insert into docs(name, module, path, detail, docstring) values(?1, ?2, ?3, ?4, ?5)",
    )?;
    let mut reference_statement = connection.prepare(
        "insert into reference_spans(name, path, start_line, start_character, end_line, end_character) values(?1, ?2, ?3, ?4, ?5, ?6)",
    )?;
    let references = persist_reference_spans.then_some(&mut reference_statement);
    insert_file_rows(
        file,
        &mut file_statement,
        &mut symbol_statement,
        &mut doc_statement,
        references,
    )
}

fn insert_file_rows(
    file: &IndexedFile,
    file_statement: &mut rusqlite::Statement<'_>,
    symbol_statement: &mut rusqlite::Statement<'_>,
    doc_statement: &mut rusqlite::Statement<'_>,
    reference_statement: Option<&mut rusqlite::Statement<'_>>,
) -> Result<()> {
    let path = file.path.display().to_string();
    let fingerprint = file_fingerprint(&file.path)?;
    file_statement.execute(params![path.as_str(), file.module.as_str(), fingerprint])?;
    if let Some(docstring) = &file.module_docstring {
        doc_statement.execute(params![
            file.module.as_str(),
            file.module.as_str(),
            path.as_str(),
            "module",
            docstring.as_str()
        ])?;
    }
    for symbol in &file.symbols {
        symbol_statement.execute(params![
            symbol.name.as_str(),
            symbol_kind_as_str(&symbol.kind),
            symbol.module.as_str(),
            path.as_str(),
            symbol.range.start_line,
            symbol.range.start_character,
            symbol.range.end_line,
            symbol.range.end_character,
            symbol.detail.as_str(),
            symbol.import_from.as_deref(),
            symbol.signature.as_deref(),
        ])?;
        if let Some(docstring) = &symbol.docstring {
            doc_statement.execute(params![
                symbol.name.as_str(),
                symbol.module.as_str(),
                path.as_str(),
                symbol.detail.as_str(),
                docstring.as_str()
            ])?;
        }
    }
    if let Some(statement) = reference_statement {
        insert_reference_rows(file, statement)?;
    }
    Ok(())
}

fn insert_reference_rows(
    file: &IndexedFile,
    statement: &mut rusqlite::Statement<'_>,
) -> Result<()> {
    let source = fs::read_to_string(&file.path)
        .with_context(|| format!("read references from {}", file.path.display()))?;
    for (name, reference) in reference_spans_in_source(&file.path, &source) {
        statement.execute(params![
            name.as_str(),
            reference.path.display().to_string(),
            reference.range.start_line,
            reference.range.start_character,
            reference.range.end_line,
            reference.range.end_character,
        ])?;
    }
    Ok(())
}

fn delete_path_from_db(connection: &Connection, path: &str) -> Result<()> {
    connection.execute("delete from docs where path = ?1", params![path])?;
    connection.execute("delete from reference_spans where path = ?1", params![path])?;
    connection.execute("delete from symbols where path = ?1", params![path])?;
    connection.execute("delete from files where path = ?1", params![path])?;
    Ok(())
}

fn delete_roots_from_db(connection: &Connection, roots: &[PathBuf]) -> Result<()> {
    for root in roots {
        let root_path = root.display().to_string();
        let child_path_pattern = like_pattern_for_children(&root_path);
        for table in ["docs", "reference_spans", "symbols", "files"] {
            connection.execute(
                &format!("delete from {table} where path = ?1 or path like ?2 escape '~'"),
                params![root_path, child_path_pattern],
            )?;
        }
    }
    clear_doc_fts(connection)?;
    Ok(())
}

fn clear_doc_fts(connection: &Connection) -> Result<()> {
    connection.execute("delete from docs_fts", [])?;
    Ok(())
}

fn create_lookup_indexes(connection: &Connection) -> Result<()> {
    connection.execute_batch(
        r#"
        create index if not exists idx_symbols_name on symbols(name);
        create index if not exists idx_symbols_module on symbols(module);
        create index if not exists idx_symbols_path on symbols(path);
        create index if not exists idx_docs_path on docs(path);
        create index if not exists idx_docs_symbol on docs(path, module, name, detail);
        create index if not exists idx_reference_spans_name on reference_spans(name);
        create index if not exists idx_reference_spans_path on reference_spans(path);
        create index if not exists idx_sage_export_cache_path on sage_export_cache(path);
        create index if not exists idx_sage_method_cache_path on sage_method_cache(path);
        "#,
    )?;
    Ok(())
}

fn like_pattern_for_children(root_path: &str) -> String {
    let mut value = String::new();
    for character in root_path
        .chars()
        .chain(std::iter::once(std::path::MAIN_SEPARATOR))
    {
        match character {
            '~' | '%' | '_' => {
                value.push('~');
                value.push(character);
            }
            _ => value.push(character),
        }
    }
    value.push('%');
    value
}

fn parse_symbol_kind(value: &str) -> SymbolKind {
    match value {
        "Module" => SymbolKind::Module,
        "Class" => SymbolKind::Class,
        "Function" => SymbolKind::Function,
        "Variable" => SymbolKind::Variable,
        "Import" => SymbolKind::Import,
        "CythonDeclaration" => SymbolKind::CythonDeclaration,
        "PreparserGenerator" => SymbolKind::PreparserGenerator,
        _ => SymbolKind::Variable,
    }
}

fn symbol_kind_as_str(kind: &SymbolKind) -> &'static str {
    match kind {
        SymbolKind::Module => "Module",
        SymbolKind::Class => "Class",
        SymbolKind::Function => "Function",
        SymbolKind::Variable => "Variable",
        SymbolKind::Import => "Import",
        SymbolKind::CythonDeclaration => "CythonDeclaration",
        SymbolKind::PreparserGenerator => "PreparserGenerator",
    }
}

fn module_matches_import(module: &str, import_from: &str) -> bool {
    module == import_from || module.ends_with(&format!(".{import_from}"))
}

fn import_target_definition_matches(
    candidate: &SymbolRecord,
    source_module: &str,
    source_name: &str,
) -> bool {
    if candidate.kind == SymbolKind::Import {
        return false;
    }
    if candidate.kind == SymbolKind::Module {
        return candidate.module == source_module
            || (candidate.name == source_name
                && candidate.module == format!("{source_module}.{source_name}"));
    }
    candidate.name == source_name && module_matches_import(&candidate.module, source_module)
}

fn module_basename(module: &str) -> &str {
    module.rsplit('.').next().unwrap_or(module)
}

fn is_namespace_owner_record(record: &SymbolRecord) -> bool {
    matches!(record.kind, SymbolKind::Module | SymbolKind::Variable)
}

fn namespace_member_matches_owner(
    candidate: &SymbolRecord,
    owner_record: &SymbolRecord,
    member: &str,
) -> bool {
    if candidate.name != member {
        return false;
    }
    if candidate.module == owner_record.module {
        return true;
    }
    candidate.kind == SymbolKind::Module
        && owner_record.kind == SymbolKind::Module
        && candidate.module == format!("{}.{}", owner_record.module, member)
}

pub fn collect_indexable_paths(options: &IndexOptions) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    for root in &options.roots {
        if !root.exists() {
            continue;
        }
        for scan_root in index_scan_roots(root) {
            let walker = WalkBuilder::new(scan_root)
                .hidden(false)
                .ignore(false)
                .git_ignore(true)
                .build();
            for entry in walker.flatten() {
                let path = entry.path();
                if !path.is_file() || is_excluded(path, &options.exclude_globs) {
                    continue;
                }
                if is_indexable(path, options.enable_pyx) {
                    paths.push(path.to_path_buf());
                }
            }
        }
    }
    paths.sort();
    paths.dedup();
    paths
}

fn index_scan_roots(root: &Path) -> Vec<PathBuf> {
    if is_python_package_root(root) {
        let sage_package = root.join("sage");
        if sage_package.is_dir() {
            return vec![sage_package];
        }
    }
    vec![root.to_path_buf()]
}

pub fn parse_file_for_roots(path: &Path, roots: &[PathBuf]) -> Result<IndexedFile> {
    let path = normalize_path(path.to_path_buf());
    let source = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    let root = roots
        .iter()
        .find(|root| path.starts_with(root))
        .cloned()
        .unwrap_or_else(|| path.parent().unwrap_or(Path::new("")).to_path_buf());
    let module = module_name_from_path(&root, &path);
    Ok(parse_source(&module, &path, &source))
}

pub fn parse_source(module: &str, path: &Path, source: &str) -> IndexedFile {
    let mut symbols = Vec::new();
    let module_docstring = first_docstring(source);
    let code_map = CodeMap::new(source);

    push_module_symbol(module, path, module_docstring.as_deref(), &mut symbols);
    capture_declarations(module, path, source, &code_map, &mut symbols);
    capture_class_method_aliases(module, path, source, &code_map, &mut symbols);
    capture_matrix_constructor_method_aliases(module, path, source, &code_map, &mut symbols);
    capture_preparser_generators(module, path, source, &code_map, &mut symbols);
    capture_assignments(module, path, source, &code_map, &mut symbols);
    capture_imports(module, path, source, &code_map, &mut symbols);
    capture_lazy_imports(module, path, source, &code_map, &mut symbols);
    capture_import_alias_assignments(module, path, source, &code_map, &mut symbols);
    capture_local_definition_alias_assignments(module, path, source, &code_map, &mut symbols);
    capture_import_member_alias_assignments(module, path, source, &code_map, &mut symbols);
    capture_static_member_aliases(module, path, source, &code_map, &mut symbols);
    capture_deprecated_function_aliases(module, path, source, &code_map, &mut symbols);
    capture_all_exports(module, path, source, &code_map, &mut symbols);

    IndexedFile {
        module: module.to_string(),
        path: path.to_path_buf(),
        symbols,
        module_docstring,
    }
}

fn push_module_symbol(
    module: &str,
    path: &Path,
    module_docstring: Option<&str>,
    symbols: &mut Vec<SymbolRecord>,
) {
    let name = module_basename(module);
    if name.is_empty() {
        return;
    }
    symbols.push(SymbolRecord {
        name: name.to_string(),
        kind: SymbolKind::Module,
        module: module.to_string(),
        path: path.to_path_buf(),
        range: SourceRange::default(),
        detail: format!("Module {module}"),
        docstring: module_docstring.map(str::to_string),
        import_from: None,
        signature: None,
    });
}

pub fn preprocess_sage_source(source: &str) -> PreprocessResult {
    let mut generated = String::with_capacity(source.len());
    let mut edits = Vec::new();
    let mut quote: Option<char> = None;
    let lines: Vec<&str> = source.lines().collect();
    for (line_index, line) in lines.iter().enumerate() {
        if quote.is_none() {
            if let Some(rewrite) = rewrite_preparser_assignment(line, line_index as u32) {
                generated.push_str(&rewrite.generated);
                edits.extend(rewrite.edits);
                if line_index + 1 < lines.len() || source.ends_with('\n') {
                    generated.push('\n');
                }
                continue;
            }
        }
        let mut chars = line.char_indices().peekable();
        while let Some((character, ch)) = chars.next() {
            if quote.is_none() && ch == '#' {
                generated.push_str(&line[character..]);
                break;
            }
            if ch == '\'' || ch == '"' {
                quote = match quote {
                    Some(current) if current == ch => None,
                    None => Some(ch),
                    current => current,
                };
                generated.push(ch);
                continue;
            }
            if quote.is_none() && ch == '^' {
                generated.push_str("**");
                edits.push(PreprocessEdit {
                    line: line_index as u32,
                    source_character: character as u32,
                    generated_character: character as u32,
                    source_text: "^".to_string(),
                    generated_text: "**".to_string(),
                });
                continue;
            }
            if quote.is_none() && ch == '[' {
                let next_is_close = chars.peek().is_some_and(|(_, next)| *next == ']');
                if next_is_close && should_rewrite_empty_sage_index(line, character) {
                    generated.push_str("[0]");
                    edits.push(PreprocessEdit {
                        line: line_index as u32,
                        source_character: character as u32,
                        generated_character: character as u32,
                        source_text: "[]".to_string(),
                        generated_text: "[0]".to_string(),
                    });
                    chars.next();
                    continue;
                }
            }
            if quote.is_none() && ch == '.' {
                let next_is_dot = chars.peek().is_some_and(|(_, next)| *next == '.');
                if next_is_dot {
                    let next_index = chars
                        .peek()
                        .map(|(index, _)| *index)
                        .unwrap_or(character + 1);
                    let previous = line[..character].chars().next_back();
                    let after_next = line[next_index + 1..].chars().next();
                    if previous != Some('.') && after_next != Some('.') {
                        generated.push(',');
                        edits.push(PreprocessEdit {
                            line: line_index as u32,
                            source_character: character as u32,
                            generated_character: character as u32,
                            source_text: "..".to_string(),
                            generated_text: ",".to_string(),
                        });
                        chars.next();
                        continue;
                    }
                }
            }
            generated.push(ch);
        }
        if line_index + 1 < lines.len() || source.ends_with('\n') {
            generated.push('\n');
        }
    }
    PreprocessResult { generated, edits }
}

#[derive(Clone, Debug)]
struct LineRewrite {
    generated: String,
    edits: Vec<PreprocessEdit>,
}

fn rewrite_preparser_assignment(line: &str, line_index: u32) -> Option<LineRewrite> {
    let captures = preparser_assignment_re().captures(line)?;
    let parent = captures.name("parent")?.as_str();
    let symbols = captures.name("symbols")?.as_str();
    let rhs = captures.name("rhs")?;
    let rhs_source = rhs.as_str();
    let generated_prefix = format!("{parent} = ");
    let (rewritten_rhs, mut op_edits) = rewrite_sage_operators_in_segment(
        rhs_source,
        line_index,
        rhs.start(),
        generated_prefix.len(),
    );
    let mut generated = format!("{generated_prefix}{rewritten_rhs}");
    for (index, name) in symbols
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .enumerate()
    {
        generated.push_str(&format!("; {name} = {parent}.gen({index})"));
    }
    let mut edits = vec![PreprocessEdit {
        line: line_index,
        source_character: captures.name("parent")?.start() as u32,
        generated_character: 0,
        source_text: line.to_string(),
        generated_text: "preparser-assignment".to_string(),
    }];
    edits.append(&mut op_edits);
    Some(LineRewrite { generated, edits })
}

fn rewrite_sage_operators_in_segment(
    segment: &str,
    line_index: u32,
    source_base: usize,
    generated_base: usize,
) -> (String, Vec<PreprocessEdit>) {
    let mut generated = String::with_capacity(segment.len());
    let mut edits = Vec::new();
    let mut quote: Option<char> = None;
    let mut chars = segment.char_indices().peekable();

    while let Some((character, ch)) = chars.next() {
        if quote.is_none() && ch == '#' {
            generated.push_str(&segment[character..]);
            break;
        }
        if ch == '\'' || ch == '"' {
            quote = match quote {
                Some(current) if current == ch => None,
                None => Some(ch),
                current => current,
            };
            generated.push(ch);
            continue;
        }
        if quote.is_none() && ch == '^' {
            let generated_character = generated_base + generated.len();
            generated.push_str("**");
            edits.push(PreprocessEdit {
                line: line_index,
                source_character: (source_base + character) as u32,
                generated_character: generated_character as u32,
                source_text: "^".to_string(),
                generated_text: "**".to_string(),
            });
            continue;
        }
        if quote.is_none() && ch == '[' {
            let next_is_close = chars.peek().is_some_and(|(_, next)| *next == ']');
            if next_is_close && should_rewrite_empty_sage_index(segment, character) {
                let generated_character = generated_base + generated.len();
                generated.push_str("[0]");
                edits.push(PreprocessEdit {
                    line: line_index,
                    source_character: (source_base + character) as u32,
                    generated_character: generated_character as u32,
                    source_text: "[]".to_string(),
                    generated_text: "[0]".to_string(),
                });
                chars.next();
                continue;
            }
        }
        if quote.is_none() && ch == '.' {
            let next_is_dot = chars.peek().is_some_and(|(_, next)| *next == '.');
            if next_is_dot {
                let next_index = chars
                    .peek()
                    .map(|(index, _)| *index)
                    .unwrap_or(character + 1);
                let previous = segment[..character].chars().next_back();
                let after_next = segment[next_index + 1..].chars().next();
                if previous != Some('.') && after_next != Some('.') {
                    let generated_character = generated_base + generated.len();
                    generated.push(',');
                    edits.push(PreprocessEdit {
                        line: line_index,
                        source_character: (source_base + character) as u32,
                        generated_character: generated_character as u32,
                        source_text: "..".to_string(),
                        generated_text: ",".to_string(),
                    });
                    chars.next();
                    continue;
                }
            }
        }
        generated.push(ch);
    }

    (generated, edits)
}

fn should_rewrite_empty_sage_index(text: &str, open_bracket: usize) -> bool {
    text[..open_bracket]
        .chars()
        .next_back()
        .is_some_and(is_empty_sage_index_owner)
}

fn is_empty_sage_index_owner(ch: char) -> bool {
    ch == ')' || ch == ']' || ch == '}' || ch == '_' || ch.is_ascii_alphanumeric()
}

pub fn semantic_spans(source: &str) -> Vec<SemanticSpan> {
    let mut spans = Vec::new();
    let code_map = CodeMap::new(source);
    for re in [class_re(), function_re()] {
        for captures in re.captures_iter(source) {
            if let Some(name) = captures.name("name") {
                if !code_map.is_code_offset(name.start()) {
                    continue;
                }
                let (line, character) = code_map.line_col(name.start());
                spans.push(SemanticSpan {
                    line,
                    start: character,
                    length: name.as_str().len() as u32,
                    token_type: if re.as_str().contains("class") {
                        "class".to_string()
                    } else {
                        "function".to_string()
                    },
                    modifiers: vec!["declaration".to_string()],
                });
            }
        }
    }
    for captures in preparser_re().captures_iter(source) {
        if let Some(parent) = captures.name("parent") {
            if !code_map.is_code_offset(parent.start()) {
                continue;
            }
            let (line, character) = code_map.line_col(parent.start());
            spans.push(SemanticSpan {
                line,
                start: character,
                length: parent.as_str().len() as u32,
                token_type: "variable".to_string(),
                modifiers: vec!["declaration".to_string()],
            });
        }
        if let Some(symbols) = captures.name("symbols") {
            if !code_map.is_code_offset(symbols.start()) {
                continue;
            }
            for name in symbols
                .as_str()
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                if let Some(offset) = source[symbols.start()..symbols.end()].find(name) {
                    let absolute = symbols.start() + offset;
                    let (line, character) = code_map.line_col(absolute);
                    spans.push(SemanticSpan {
                        line,
                        start: character,
                        length: name.len() as u32,
                        token_type: "parameter".to_string(),
                        modifiers: vec!["declaration".to_string()],
                    });
                }
            }
        }
    }
    for captures in semantic_assignment_re().captures_iter(source) {
        let Some(name) = captures.name("name") else {
            continue;
        };
        if !code_map.is_code_offset(name.start()) {
            continue;
        }
        let (line, character) = code_map.line_col(name.start());
        spans.push(SemanticSpan {
            line,
            start: character,
            length: name.as_str().len() as u32,
            token_type: "variable".to_string(),
            modifiers: vec!["declaration".to_string()],
        });
    }
    for captures in decorator_re().captures_iter(source) {
        if let Some(name) = captures.name("name") {
            if !code_map.is_code_offset(name.start()) {
                continue;
            }
            let (line, character) = code_map.line_col(name.start());
            spans.push(SemanticSpan {
                line,
                start: character,
                length: name.as_str().len() as u32,
                token_type: "decorator".to_string(),
                modifiers: vec!["defaultLibrary".to_string()],
            });
        }
    }
    for captures in word_re().captures_iter(source) {
        let Some(name) = captures.name("name") else {
            continue;
        };
        if !code_map.is_code_offset(name.start()) {
            continue;
        }
        if name.start() > 0 && source.as_bytes()[name.start() - 1] == b'@' {
            continue;
        }
        let token = name.as_str();
        let token_type = if SAGE_NAMESPACES.contains(&token) {
            "namespace"
        } else if SAGE_TYPES.contains(&token) {
            "type"
        } else if SAGE_FUNCTIONS.contains(&token) {
            "function"
        } else if SAGE_READONLY.contains(&token) {
            "variable"
        } else {
            continue;
        };
        let (line, character) = code_map.line_col(name.start());
        let modifiers = if SAGE_READONLY.contains(&token) {
            vec!["readonly".to_string(), "defaultLibrary".to_string()]
        } else {
            vec!["defaultLibrary".to_string()]
        };
        spans.push(SemanticSpan {
            line,
            start: character,
            length: token.len() as u32,
            token_type: token_type.to_string(),
            modifiers,
        });
    }
    spans.sort_by(|left, right| {
        left.line
            .cmp(&right.line)
            .then(left.start.cmp(&right.start))
            .then(right.length.cmp(&left.length))
    });
    let mut filtered = Vec::with_capacity(spans.len());
    let mut last_line = None;
    let mut last_end = 0u32;
    for span in spans {
        if last_line != Some(span.line) {
            last_line = Some(span.line);
            last_end = 0;
        }
        if span.start < last_end {
            continue;
        }
        last_end = span.start.saturating_add(span.length);
        filtered.push(span);
    }
    filtered
}

fn parse_with_tree_sitter(source: &str) -> Option<tree_sitter::Tree> {
    let mut parser = Parser::new();
    parser.set_language(tree_sitter_python::language()).ok()?;
    parser.parse(source, None)
}

fn capture_declarations(
    module: &str,
    path: &Path,
    source: &str,
    code_map: &CodeMap,
    symbols: &mut Vec<SymbolRecord>,
) {
    let context = DeclarationCaptureContext {
        module,
        path,
        source,
        code_map,
    };
    let mut class_stack: Vec<(usize, String)> = Vec::new();
    for (line_start, line) in line_offsets(source) {
        let trimmed = line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let indent = line.len() - trimmed.len();
        let code_offset = line_start + indent;
        if !code_map.is_code_offset(code_offset) {
            continue;
        }
        while class_stack
            .last()
            .is_some_and(|(class_indent, _)| indent <= *class_indent)
        {
            class_stack.pop();
        }
        if let Some(captures) = class_re().captures(trimmed) {
            let Some(name) = captures.name("name") else {
                continue;
            };
            let offset = line_start + indent + name.start();
            push_declaration_symbol(
                &context,
                symbols,
                name.as_str(),
                offset,
                SymbolKind::Class,
                None,
            );
            class_stack.push((indent, name.as_str().to_string()));
            continue;
        }
        if let Some(captures) = function_re().captures(trimmed) {
            let Some(name) = captures.name("name") else {
                continue;
            };
            let offset = line_start + indent + name.start();
            let actual_kind = if is_cython_path(path) {
                SymbolKind::CythonDeclaration
            } else {
                SymbolKind::Function
            };
            let enclosing_class = class_stack
                .last()
                .filter(|(class_indent, _)| indent > *class_indent)
                .map(|(_, class_name)| class_name.as_str());
            push_declaration_symbol(
                &context,
                symbols,
                name.as_str(),
                offset,
                actual_kind,
                enclosing_class,
            );
        }
    }
}

fn capture_class_method_aliases(
    module: &str,
    path: &Path,
    source: &str,
    code_map: &CodeMap,
    symbols: &mut Vec<SymbolRecord>,
) {
    let method_targets: BTreeSet<(String, String)> = symbols
        .iter()
        .filter(|symbol| is_source_derived_sage_method(symbol))
        .filter_map(|symbol| {
            method_detail_parts(&symbol.detail)
                .map(|(class_name, method_name)| (class_name.to_string(), method_name.to_string()))
        })
        .collect();
    if method_targets.is_empty() {
        return;
    }

    let mut class_stack: Vec<(usize, String)> = Vec::new();
    for (line_start, line) in line_offsets(source) {
        let trimmed = line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let indent = line.len() - trimmed.len();
        let code_offset = line_start + indent;
        if !code_map.is_code_offset(code_offset) {
            continue;
        }
        while class_stack
            .last()
            .is_some_and(|(class_indent, _)| indent <= *class_indent)
        {
            class_stack.pop();
        }
        if let Some(captures) = class_re().captures(trimmed) {
            if let Some(name) = captures.name("name") {
                class_stack.push((indent, name.as_str().to_string()));
            }
            continue;
        }
        let Some((class_indent, class_name)) = class_stack.last() else {
            continue;
        };
        if indent != class_indent.saturating_add(4) {
            continue;
        }
        let trimmed_assignment = trimmed
            .split('#')
            .next()
            .unwrap_or(trimmed)
            .trim()
            .trim_end_matches(';')
            .trim();
        let Some(captures) = simple_assignment_re().captures(trimmed_assignment) else {
            continue;
        };
        let Some(alias) = captures.name("name") else {
            continue;
        };
        let Some(target) = captures.name("rhs") else {
            continue;
        };
        let target = target.as_str().trim();
        if alias.as_str() == target
            || !is_valid_identifier(target)
            || (alias.as_str().starts_with("__") && alias.as_str().ends_with("__"))
            || !method_targets.contains(&(class_name.clone(), target.to_string()))
        {
            continue;
        }
        let Some(relative) = line.find(alias.as_str()) else {
            continue;
        };
        let offset = line_start + relative;
        if !code_map.is_code_offset(offset) {
            continue;
        }
        push_import_symbol(
            symbols,
            module,
            path,
            alias.as_str(),
            code_map,
            offset,
            &format!("{module}::{target}"),
        );
        if let Some(symbol) = symbols.last_mut() {
            symbol.detail = format!(
                "MethodAlias {}.{} for {}",
                class_name,
                alias.as_str(),
                target
            );
        }
    }
}

fn capture_matrix_constructor_method_aliases(
    module: &str,
    path: &Path,
    source: &str,
    code_map: &CodeMap,
    symbols: &mut Vec<SymbolRecord>,
) {
    if !source.contains("@matrix_method") {
        return;
    }

    let mut pending_matrix_method: Option<Option<String>> = None;
    for (line_start, line) in line_offsets(source) {
        let trimmed = line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let indent = line.len() - trimmed.len();
        if indent != 0 {
            continue;
        }
        let code_offset = line_start + indent;
        if !code_map.is_code_offset(code_offset) {
            continue;
        }
        if let Some(alias) = parse_matrix_method_decorator(trimmed) {
            pending_matrix_method = Some(alias);
            continue;
        }
        if trimmed.starts_with('@') {
            continue;
        }
        let Some(alias_override) = pending_matrix_method.take() else {
            continue;
        };
        let declaration = function_re()
            .captures(trimmed)
            .and_then(|captures| captures.name("name"))
            .or_else(|| {
                class_re()
                    .captures(trimmed)
                    .and_then(|captures| captures.name("name"))
            });
        let Some(name) = declaration else {
            continue;
        };
        let target_name = name.as_str();
        let alias = alias_override.unwrap_or_else(|| matrix_method_alias_name(target_name));
        if !is_valid_identifier(&alias) {
            continue;
        }
        let offset = line_start + indent + name.start();
        push_import_symbol(
            symbols,
            module,
            path,
            &alias,
            code_map,
            offset,
            &format!("{module}::{target_name}"),
        );
        if let Some(symbol) = symbols.last_mut() {
            symbol.detail =
                format!("MatrixConstructorMethodAlias matrix.{alias} for {target_name}");
        }
    }
}

fn parse_matrix_method_decorator(trimmed: &str) -> Option<Option<String>> {
    let rest = trimmed.strip_prefix("@matrix_method")?;
    if rest.trim().is_empty() {
        return Some(None);
    }
    if !rest.trim_start().starts_with('(') {
        return None;
    }
    let explicit = matrix_method_name_override_re()
        .captures(rest)
        .and_then(|captures| {
            captures
                .name("double")
                .or_else(|| captures.name("single"))
                .map(|value| value.as_str().to_string())
        });
    Some(explicit)
}

fn matrix_method_alias_name(target_name: &str) -> String {
    let alias = target_name.replace("matrix", "");
    let alias = alias.trim_matches('_');
    if alias.is_empty() {
        target_name.to_string()
    } else {
        alias.to_string()
    }
}

struct DeclarationCaptureContext<'a> {
    module: &'a str,
    path: &'a Path,
    source: &'a str,
    code_map: &'a CodeMap,
}

fn push_declaration_symbol(
    context: &DeclarationCaptureContext<'_>,
    symbols: &mut Vec<SymbolRecord>,
    name: &str,
    offset: usize,
    kind: SymbolKind,
    enclosing_class: Option<&str>,
) {
    if !context.code_map.is_code_offset(offset) {
        return;
    }
    let detail = if let Some(class_name) = enclosing_class {
        format!("Method {class_name}.{name}")
    } else {
        format!("{kind:?} {name}")
    };
    let (line, character) = context.code_map.line_col(offset);
    symbols.push(SymbolRecord {
        name: name.to_string(),
        kind: kind.clone(),
        module: context.module.to_string(),
        path: context.path.to_path_buf(),
        range: SourceRange {
            start_line: line,
            start_character: character,
            end_line: line,
            end_character: character + name.len() as u32,
        },
        detail,
        docstring: doc_after_offset(context.source, offset + name.len()),
        import_from: None,
        signature: function_signature(context.source, offset, name),
    });
}

fn capture_preparser_generators(
    module: &str,
    path: &Path,
    source: &str,
    code_map: &CodeMap,
    symbols: &mut Vec<SymbolRecord>,
) {
    for captures in preparser_re().captures_iter(source) {
        if let Some(parent) = captures.name("parent") {
            if !code_map.is_code_offset(parent.start()) {
                continue;
            }
            push_simple_symbol(
                symbols,
                module,
                path,
                parent.as_str(),
                SymbolKind::Variable,
                code_map,
                parent.start(),
            );
        }
        if let Some(generators) = captures.name("symbols") {
            if !code_map.is_code_offset(generators.start()) {
                continue;
            }
            for name in generators
                .as_str()
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                if let Some(relative) = source[generators.start()..generators.end()].find(name) {
                    push_simple_symbol(
                        symbols,
                        module,
                        path,
                        name,
                        SymbolKind::PreparserGenerator,
                        code_map,
                        generators.start() + relative,
                    );
                }
            }
        }
    }
}

fn capture_assignments(
    module: &str,
    path: &Path,
    source: &str,
    code_map: &CodeMap,
    symbols: &mut Vec<SymbolRecord>,
) {
    let context = SymbolPushContext {
        module,
        path,
        code_map,
    };
    let declared_names = top_level_declared_symbol_names(symbols);
    for captures in assignment_re().captures_iter(source) {
        let Some(name) = captures.name("name") else {
            continue;
        };
        if !code_map.is_code_offset(name.start()) {
            continue;
        }
        let (line, _) = code_map.line_col(name.start());
        if symbols.iter().any(|symbol| {
            symbol.name == name.as_str() && symbol.path == path && symbol.range.start_line == line
        }) {
            continue;
        }
        let rhs = captures
            .name("rhs")
            .map(|value| value.as_str())
            .unwrap_or_default();
        if rhs.trim_start().starts_with("deprecated_function_alias(") {
            continue;
        }
        if member_reference_re().is_match(rhs.trim()) {
            continue;
        }
        if declared_names.contains(rhs.trim()) {
            continue;
        }
        let detail = assignment_detail(
            name.as_str(),
            captures.name("annotation").map(|value| value.as_str()),
            rhs,
        );
        push_symbol_with_detail(
            symbols,
            &context,
            name.as_str(),
            SymbolKind::Variable,
            name.start(),
            detail,
        );
    }
}

fn top_level_declared_symbol_names(symbols: &[SymbolRecord]) -> BTreeSet<String> {
    symbols
        .iter()
        .filter(|symbol| {
            matches!(
                symbol.kind,
                SymbolKind::Class | SymbolKind::Function | SymbolKind::CythonDeclaration
            )
        })
        .map(|symbol| symbol.name.clone())
        .collect()
}

fn capture_imports(
    module: &str,
    path: &Path,
    source: &str,
    code_map: &CodeMap,
    symbols: &mut Vec<SymbolRecord>,
) {
    let mut multiline_from_import: Option<String> = None;
    for (line_start, line) in line_offsets(source) {
        if !code_map.is_code_offset(line_start) {
            continue;
        }
        let trimmed = line.trim_start();
        let indent = line.len() - trimmed.len();
        if let Some(import_module) = multiline_from_import.clone() {
            capture_multiline_import_names(
                module,
                path,
                MultilineImportCapture {
                    text: trimmed,
                    original_line: line,
                    line_start,
                    import_module: &import_module,
                },
                code_map,
                symbols,
            );
            if trimmed.contains(')') {
                multiline_from_import = None;
            }
            continue;
        }
        if let Some((import_module, rest)) = parse_multiline_from_import_start(trimmed) {
            capture_multiline_import_names(
                module,
                path,
                MultilineImportCapture {
                    text: rest,
                    original_line: line,
                    line_start,
                    import_module: &import_module,
                },
                code_map,
                symbols,
            );
            if !rest.contains(')') {
                multiline_from_import = Some(import_module);
            }
            continue;
        }
        if module_is_sage_all_export_module(module) {
            if let Some(import_module) = parse_star_import(trimmed) {
                push_import_symbol(
                    symbols,
                    module,
                    path,
                    SAGE_STAR_IMPORT_SENTINEL,
                    code_map,
                    line_start + indent,
                    &format!("{import_module}::*"),
                );
                continue;
            }
        }
        if let Some(import) =
            parse_from_import(trimmed, false).or_else(|| parse_from_import(trimmed, true))
        {
            for binding in import.bindings {
                if let Some(relative) = line.find(&binding.binding) {
                    push_import_symbol(
                        symbols,
                        module,
                        path,
                        &binding.binding,
                        code_map,
                        line_start + relative,
                        &format!("{}::{}", import.module, binding.source_name),
                    );
                }
            }
            continue;
        }
        if let Some(include_name) = parse_cython_include(trimmed) {
            push_import_symbol(
                symbols,
                module,
                path,
                &include_name,
                code_map,
                line_start + indent,
                &include_name,
            );
            continue;
        }
        if let Some(names) = parse_plain_import(trimmed) {
            for (name, import_from) in names {
                if let Some(relative) = line.find(&name) {
                    push_import_symbol(
                        symbols,
                        module,
                        path,
                        &name,
                        code_map,
                        line_start + relative,
                        &import_from,
                    );
                }
            }
        }
    }
}

fn capture_lazy_imports(
    module: &str,
    path: &Path,
    source: &str,
    code_map: &CodeMap,
    symbols: &mut Vec<SymbolRecord>,
) {
    for (call_start, call) in lazy_import_calls(source, code_map) {
        for import in parse_lazy_imports(call) {
            let Some(relative) = string_literal_position(call, &import.binding)
                .or_else(|| string_literal_position(call, &import.target))
            else {
                continue;
            };
            push_import_symbol(
                symbols,
                module,
                path,
                &import.binding,
                code_map,
                call_start + relative,
                &format!("{}::{}", import.module, import.target),
            );
        }
    }
    for (binding_offset, binding, import) in lazy_import_object_assignments(source, code_map) {
        push_import_symbol(
            symbols,
            module,
            path,
            &binding,
            code_map,
            binding_offset,
            &format!("{}::{}", import.module, import.target),
        );
    }
}

fn capture_import_alias_assignments(
    module: &str,
    path: &Path,
    source: &str,
    code_map: &CodeMap,
    symbols: &mut Vec<SymbolRecord>,
) {
    let imports_by_name: BTreeMap<String, String> = symbols
        .iter()
        .filter(|symbol| symbol.kind == SymbolKind::Import)
        .filter_map(|symbol| {
            symbol
                .import_from
                .as_ref()
                .map(|import_from| (symbol.name.clone(), import_from.clone()))
        })
        .collect();

    for (line_start, line) in line_offsets(source) {
        let trimmed = line
            .split('#')
            .next()
            .unwrap_or(line)
            .trim()
            .trim_end_matches(';')
            .trim();
        let Some(captures) = simple_assignment_re().captures(trimmed) else {
            continue;
        };
        let Some(alias) = captures.name("name") else {
            continue;
        };
        let Some(target) = captures.name("rhs") else {
            continue;
        };
        let target = target.as_str().trim();
        if !is_valid_identifier(target) {
            continue;
        }
        let Some(import_from) = imports_by_name.get(target) else {
            continue;
        };
        let Some(relative) = line.find(alias.as_str()) else {
            continue;
        };
        let offset = line_start + relative;
        if !code_map.is_code_offset(offset) {
            continue;
        }
        push_import_symbol(
            symbols,
            module,
            path,
            alias.as_str(),
            code_map,
            offset,
            import_from,
        );
    }
}

fn capture_local_definition_alias_assignments(
    module: &str,
    path: &Path,
    source: &str,
    code_map: &CodeMap,
    symbols: &mut Vec<SymbolRecord>,
) {
    let declarations = top_level_declared_symbol_names(symbols);
    if declarations.is_empty() {
        return;
    }

    for (line_start, line) in line_offsets(source) {
        if line.trim_start() != line {
            continue;
        }
        let trimmed = line
            .split('#')
            .next()
            .unwrap_or(line)
            .trim()
            .trim_end_matches(';')
            .trim();
        let Some(captures) = simple_assignment_re().captures(trimmed) else {
            continue;
        };
        let Some(alias) = captures.name("name") else {
            continue;
        };
        let Some(target) = captures.name("rhs") else {
            continue;
        };
        let target = target.as_str().trim();
        if alias.as_str() == target || !declarations.contains(target) {
            continue;
        }
        let Some(relative) = line.find(alias.as_str()) else {
            continue;
        };
        let offset = line_start + relative;
        if !code_map.is_code_offset(offset) {
            continue;
        }
        push_import_symbol(
            symbols,
            module,
            path,
            alias.as_str(),
            code_map,
            offset,
            &format!("{module}::{target}"),
        );
        if let Some(symbol) = symbols.last_mut() {
            symbol.detail = format!("Import alias {} for {}", alias.as_str(), target);
        }
    }
}

fn capture_import_member_alias_assignments(
    module: &str,
    path: &Path,
    source: &str,
    code_map: &CodeMap,
    symbols: &mut Vec<SymbolRecord>,
) {
    let imports_by_name: BTreeMap<String, String> = symbols
        .iter()
        .filter(|symbol| symbol.kind == SymbolKind::Import)
        .filter_map(|symbol| {
            symbol
                .import_from
                .as_ref()
                .map(|import_from| (symbol.name.clone(), import_from.clone()))
        })
        .collect();

    for (line_start, line) in line_offsets(source) {
        if line.trim_start() != line {
            continue;
        }
        let trimmed = line
            .split('#')
            .next()
            .unwrap_or(line)
            .trim()
            .trim_end_matches(';')
            .trim();
        let Some(captures) = member_alias_assignment_re().captures(trimmed) else {
            continue;
        };
        let Some(alias) = captures.name("name") else {
            continue;
        };
        let Some(owner) = captures.name("owner") else {
            continue;
        };
        let Some(member) = captures.name("member") else {
            continue;
        };
        let Some(owner_import) = imports_by_name.get(owner.as_str()) else {
            continue;
        };
        let Some(relative) = line.find(alias.as_str()) else {
            continue;
        };
        let offset = line_start + relative;
        if !code_map.is_code_offset(offset) {
            continue;
        }
        let target_module = imported_module_path(owner_import, owner.as_str(), module);
        push_import_symbol(
            symbols,
            module,
            path,
            alias.as_str(),
            code_map,
            offset,
            &format!("{}::{}", target_module, member.as_str()),
        );
        if let Some(symbol) = symbols.last_mut() {
            symbol.detail = format!(
                "Import alias {} for {}.{}",
                alias.as_str(),
                owner.as_str(),
                member.as_str()
            );
        }
    }
}

fn capture_static_member_aliases(
    module: &str,
    path: &Path,
    source: &str,
    code_map: &CodeMap,
    symbols: &mut Vec<SymbolRecord>,
) {
    let imports_by_name: BTreeMap<String, String> = symbols
        .iter()
        .filter(|symbol| symbol.kind == SymbolKind::Import)
        .filter_map(|symbol| {
            symbol
                .import_from
                .as_ref()
                .map(|import_from| (symbol.name.clone(), import_from.clone()))
        })
        .collect();

    for (line_start, line) in line_offsets(source) {
        let Some(captures) = static_member_alias_re().captures(line) else {
            continue;
        };
        let Some(alias) = captures.name("name") else {
            continue;
        };
        let Some(module_alias) = captures.name("module") else {
            continue;
        };
        let Some(member) = captures.name("member") else {
            continue;
        };
        let Some(relative) = line.find(alias.as_str()) else {
            continue;
        };
        let offset = line_start + relative;
        if !code_map.is_code_offset(offset) {
            continue;
        }
        let Some(import_from) = imports_by_name.get(module_alias.as_str()) else {
            continue;
        };
        let target_module = imported_module_path(import_from, module_alias.as_str(), module);
        push_import_symbol(
            symbols,
            module,
            path,
            alias.as_str(),
            code_map,
            offset,
            &format!("{}::{}", target_module, member.as_str()),
        );
    }
}

fn capture_deprecated_function_aliases(
    module: &str,
    path: &Path,
    source: &str,
    code_map: &CodeMap,
    symbols: &mut Vec<SymbolRecord>,
) {
    let imports_by_name: BTreeMap<String, String> = symbols
        .iter()
        .filter(|symbol| symbol.kind == SymbolKind::Import)
        .filter_map(|symbol| {
            symbol
                .import_from
                .as_ref()
                .map(|import_from| (symbol.name.clone(), import_from.clone()))
        })
        .collect();

    for (line_start, line) in line_offsets(source) {
        let Some(captures) = deprecated_function_alias_re().captures(line) else {
            continue;
        };
        let Some(alias) = captures.name("name") else {
            continue;
        };
        let Some(issue) = captures.name("issue") else {
            continue;
        };
        let Some(target) = captures.name("target") else {
            continue;
        };
        let Some(relative) = line.find(alias.as_str()) else {
            continue;
        };
        let offset = line_start + relative;
        if !code_map.is_code_offset(offset) {
            continue;
        }
        let Some(import_from) =
            deprecated_alias_import_target(module, target.as_str(), &imports_by_name)
        else {
            continue;
        };
        push_import_symbol(
            symbols,
            module,
            path,
            alias.as_str(),
            code_map,
            offset,
            &import_from,
        );
        if let Some(symbol) = symbols.last_mut() {
            symbol.detail = format!(
                "Deprecated alias {} for {} (Sage issue #{})",
                alias.as_str(),
                target.as_str(),
                issue.as_str()
            );
        }
    }
}

fn deprecated_alias_import_target(
    module: &str,
    target: &str,
    imports_by_name: &BTreeMap<String, String>,
) -> Option<String> {
    if let Some((owner, member)) = target.rsplit_once('.') {
        if !is_valid_identifier(member) {
            return None;
        }
        let owner_import = imports_by_name.get(owner)?;
        let target_module = imported_module_path(owner_import, owner, module);
        return Some(format!("{target_module}::{member}"));
    }
    if !is_valid_identifier(target) {
        return None;
    }
    imports_by_name
        .get(target)
        .cloned()
        .or_else(|| Some(format!("{module}::{target}")))
}

fn imported_module_path(import_from: &str, fallback_name: &str, importer_module: &str) -> String {
    if import_from.contains("::") {
        let (source_module, source_name) =
            import_target_in_context(import_from, fallback_name, importer_module);
        format!("{source_module}.{source_name}")
    } else {
        resolve_relative_module(import_from, importer_module)
    }
}

fn capture_all_exports(
    module: &str,
    path: &Path,
    source: &str,
    code_map: &CodeMap,
    symbols: &mut Vec<SymbolRecord>,
) {
    let mut explicit_offsets = Vec::new();
    let mut entries = Vec::<(usize, String)>::new();

    for (line_start, line) in line_offsets(source) {
        let trimmed = line.trim_start();
        if !trimmed.starts_with("__all__") {
            continue;
        }
        let indent = line.len() - trimmed.len();
        let name_offset = line_start + indent;
        if !code_map.is_code_offset(name_offset) {
            continue;
        }
        let Some(eq_relative) = line.find('=') else {
            continue;
        };
        explicit_offsets.push(name_offset);
        let rhs_start = line_start + eq_relative + 1;
        let Some((value_offset, value)) = python_container_literal_after(source, rhs_start) else {
            continue;
        };
        entries.extend(string_literal_entries(value, value_offset));
    }

    for (call_start, call) in all_export_calls(source, code_map, "__all__.append(") {
        explicit_offsets.push(call_start);
        if let Some(argument_text) = lazy_import_argument_text(call) {
            if let Some(entry) = string_literal_entries(argument_text, call_start)
                .into_iter()
                .next()
            {
                entries.push(entry);
            }
        }
    }

    for (call_start, call) in all_export_calls(source, code_map, "__all__.extend(") {
        explicit_offsets.push(call_start);
        if let Some(argument_text) = lazy_import_argument_text(call) {
            entries.extend(string_literal_entries(argument_text, call_start));
        }
    }

    if explicit_offsets.is_empty() {
        return;
    }
    let marker_offset = explicit_offsets[0];
    push_all_export_symbol(symbols, module, path, code_map, marker_offset, None);
    let mut seen = BTreeSet::new();
    for (offset, name) in entries {
        if is_valid_identifier(&name) && seen.insert(name.clone()) {
            push_all_export_symbol(symbols, module, path, code_map, offset, Some(&name));
        }
    }
}

fn all_export_calls<'a>(
    source: &'a str,
    code_map: &CodeMap,
    pattern: &str,
) -> Vec<(usize, &'a str)> {
    let mut calls = Vec::new();
    for (start, _) in source.match_indices(pattern) {
        if !code_map.is_code_offset(start) {
            continue;
        }
        let Some(end) = matching_python_call_end(&source[start..]) else {
            continue;
        };
        calls.push((start, &source[start..start + end]));
    }
    calls
}

fn python_container_literal_after(source: &str, start: usize) -> Option<(usize, &str)> {
    let offset = start
        + source[start..]
            .char_indices()
            .find_map(|(index, ch)| (!ch.is_whitespace()).then_some(index))?;
    let opener = source[offset..].chars().next()?;
    let closer = match opener {
        '[' => ']',
        '(' => ')',
        _ => {
            let line_end = source[offset..]
                .find('\n')
                .map(|index| offset + index)
                .unwrap_or(source.len());
            return Some((offset, &source[offset..line_end]));
        }
    };
    let mut depth = 0usize;
    let mut chars = source[offset..].char_indices().peekable();
    while let Some((relative, ch)) = chars.next() {
        if ch == '\'' || ch == '"' {
            skip_python_string(ch, &mut chars);
            continue;
        }
        if ch == opener {
            depth = depth.saturating_add(1);
        } else if ch == closer {
            depth = depth.checked_sub(1)?;
            if depth == 0 {
                let end = offset + relative + ch.len_utf8();
                return Some((offset, &source[offset..end]));
            }
        }
    }
    None
}

fn string_literal_entries(text: &str, base_offset: usize) -> Vec<(usize, String)> {
    let values = string_literal_args(text);
    let mut entries = Vec::new();
    let mut search_start = 0usize;
    for value in values {
        let relative = string_literal_position(&text[search_start..], &value)
            .map(|index| search_start + index)
            .or_else(|| string_literal_position(text, &value))
            .unwrap_or(0);
        search_start = relative.saturating_add(value.len());
        entries.push((base_offset + relative, value));
    }
    entries
}

fn push_all_export_symbol(
    symbols: &mut Vec<SymbolRecord>,
    module: &str,
    path: &Path,
    code_map: &CodeMap,
    offset: usize,
    name: Option<&str>,
) {
    let (line, character) = code_map.line_col(offset);
    let import_from = name
        .map(|name| format!("__all__::{name}"))
        .unwrap_or_else(|| SAGE_ALL_EXPORT_MARKER.to_string());
    symbols.push(SymbolRecord {
        name: SAGE_ALL_EXPORT_SENTINEL.to_string(),
        kind: SymbolKind::Import,
        module: module.to_string(),
        path: path.to_path_buf(),
        range: SourceRange {
            start_line: line,
            start_character: character,
            end_line: line,
            end_character: character + SAGE_ALL_EXPORT_SENTINEL.len() as u32,
        },
        detail: "Sage __all__ export".to_string(),
        docstring: None,
        import_from: Some(import_from),
        signature: None,
    });
}

#[derive(Clone, Debug)]
struct ParsedImport {
    module: String,
    bindings: Vec<ImportedBinding>,
}

#[derive(Clone, Debug)]
struct ImportedBinding {
    binding: String,
    source_name: String,
}

#[derive(Clone, Debug)]
struct ParsedLazyImport {
    module: String,
    target: String,
    binding: String,
}

fn lazy_import_calls<'a>(source: &'a str, code_map: &CodeMap) -> Vec<(usize, &'a str)> {
    let mut calls = Vec::new();
    for (start, _) in source.match_indices("lazy_import(") {
        if !code_map.is_code_offset(start) {
            continue;
        }
        let Some(end) = matching_python_call_end(&source[start..]) else {
            continue;
        };
        calls.push((start, &source[start..start + end]));
    }
    calls
}

fn lazy_import_object_assignments(
    source: &str,
    code_map: &CodeMap,
) -> Vec<(usize, String, ParsedLazyImport)> {
    let mut assignments = Vec::new();
    for (start, _) in source.match_indices("LazyImport(") {
        if !code_map.is_code_offset(start) {
            continue;
        }
        let Some((binding_offset, binding)) = assignment_binding_before_call(source, start) else {
            continue;
        };
        let Some(end) = matching_python_call_end(&source[start..]) else {
            continue;
        };
        let call = &source[start..start + end];
        let Some(import) = parse_lazy_import_object_call(call, &binding) else {
            continue;
        };
        assignments.push((binding_offset, binding, import));
    }
    assignments
}

fn assignment_binding_before_call(source: &str, call_start: usize) -> Option<(usize, String)> {
    let line_start = source[..call_start]
        .rfind('\n')
        .map(|index| index + 1)
        .unwrap_or(0);
    let prefix = &source[line_start..call_start];
    let eq = prefix.rfind('=')?;
    let lhs = prefix[..eq].trim();
    if !is_valid_identifier(lhs) {
        return None;
    }
    let lhs_offset = prefix[..eq].rfind(lhs)?;
    Some((line_start + lhs_offset, lhs.to_string()))
}

fn parse_lazy_import_object_call(call: &str, binding: &str) -> Option<ParsedLazyImport> {
    let args = lazy_import_argument_text(call)?;
    let args = split_top_level_args(args);
    let module = args.first().and_then(|arg| first_string_literal(arg))?;
    let target = args
        .get(1)
        .and_then(|arg| string_literal_args(arg).into_iter().next())?;
    if !is_valid_identifier(binding) || !is_valid_identifier(&target) {
        return None;
    }
    Some(ParsedLazyImport {
        module,
        target,
        binding: binding.to_string(),
    })
}

fn matching_python_call_end(text: &str) -> Option<usize> {
    let mut depth = 0usize;
    let mut chars = text.char_indices().peekable();
    while let Some((index, ch)) = chars.next() {
        if ch == '\'' || ch == '"' {
            skip_python_string(ch, &mut chars);
            continue;
        }
        match ch {
            '(' | '[' | '{' => depth = depth.saturating_add(1),
            ')' | ']' | '}' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(index + ch.len_utf8());
                }
            }
            _ => {}
        }
    }
    None
}

fn skip_python_string(quote: char, chars: &mut std::iter::Peekable<std::str::CharIndices<'_>>) {
    while let Some((_, inner)) = chars.next() {
        if inner == '\\' {
            let _ = chars.next();
            continue;
        }
        if inner == quote {
            break;
        }
    }
}

fn parse_lazy_imports(call: &str) -> Vec<ParsedLazyImport> {
    let Some(args) = lazy_import_argument_text(call) else {
        return Vec::new();
    };
    let args = split_top_level_args(args);
    let Some(module) = args.first().and_then(|arg| first_string_literal(arg)) else {
        return Vec::new();
    };
    let Some(targets) = args.get(1).map(|arg| string_literal_args(arg)) else {
        return Vec::new();
    };
    let alias_arg = args
        .iter()
        .skip(2)
        .find_map(|arg| {
            let trimmed = arg.trim();
            if let Some((key, value)) = trimmed.split_once('=') {
                (key.trim() == "as_").then_some(value.trim())
            } else {
                Some(trimmed)
            }
        })
        .filter(|arg| !arg.is_empty() && *arg != "None");
    let aliases = alias_arg.map(string_literal_args).unwrap_or_default();
    targets
        .into_iter()
        .enumerate()
        .filter_map(|(index, target)| {
            if !is_valid_identifier(&target) {
                return None;
            }
            let binding = aliases
                .get(index)
                .or_else(|| (aliases.len() == 1).then(|| aliases.first()).flatten())
                .cloned()
                .unwrap_or_else(|| target.clone());
            is_valid_identifier(&binding).then_some(ParsedLazyImport {
                module: module.clone(),
                target,
                binding,
            })
        })
        .collect()
}

fn lazy_import_argument_text(call: &str) -> Option<&str> {
    let open = call.find('(')?;
    let close = call.rfind(')')?;
    (open < close).then(|| &call[open + 1..close])
}

fn split_top_level_args(args: &str) -> Vec<&str> {
    let mut result = Vec::new();
    let mut depth = 0usize;
    let mut start = 0usize;
    let mut chars = args.char_indices().peekable();
    while let Some((index, ch)) = chars.next() {
        if ch == '\'' || ch == '"' {
            skip_python_string(ch, &mut chars);
            continue;
        }
        match ch {
            '(' | '[' | '{' => depth = depth.saturating_add(1),
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                let value = args[start..index].trim();
                if !value.is_empty() {
                    result.push(value);
                }
                start = index + ch.len_utf8();
            }
            _ => {}
        }
    }
    let value = args[start..].trim();
    if !value.is_empty() {
        result.push(value);
    }
    result
}

fn first_string_literal(text: &str) -> Option<String> {
    string_literal_args(text).into_iter().next()
}

fn string_literal_position(text: &str, value: &str) -> Option<usize> {
    for quote in ['\'', '"'] {
        let needle = format!("{quote}{value}{quote}");
        if let Some(index) = text.find(&needle) {
            return Some(index + quote.len_utf8());
        }
    }
    text.find(value)
}

fn string_literal_args(text: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut chars = text.char_indices().peekable();
    while let Some((_, ch)) = chars.next() {
        if ch != '\'' && ch != '"' {
            continue;
        }
        let quote = ch;
        let mut value = String::new();
        while let Some((_, inner)) = chars.next() {
            if inner == '\\' {
                if let Some((_, escaped)) = chars.next() {
                    value.push(escaped);
                }
                continue;
            }
            if inner == quote {
                break;
            }
            value.push(inner);
        }
        result.push(value);
    }
    result
}

fn parse_from_import(line: &str, cython: bool) -> Option<ParsedImport> {
    let keyword = if cython { " cimport " } else { " import " };
    let prefix = "from ";
    let line = line.strip_prefix(prefix)?;
    let (module, names) = line.split_once(keyword)?;
    Some(ParsedImport {
        module: module.trim().to_string(),
        bindings: names
            .split(',')
            .filter_map(parse_imported_binding)
            .collect(),
    })
}

fn parse_star_import(line: &str) -> Option<String> {
    let line = line
        .split('#')
        .next()
        .unwrap_or(line)
        .trim()
        .trim_end_matches(';')
        .trim();
    let rest = line.strip_prefix("from ")?;
    let (module, names) = rest
        .split_once(" import ")
        .or_else(|| rest.split_once(" cimport "))?;
    (names.trim() == "*").then(|| module.trim().to_string())
}

struct SourceImportLookup {
    import_module: String,
    source_name: String,
}

fn source_explicit_import_lookup(source: &str, binding_name: &str) -> Option<SourceImportLookup> {
    let mut multiline_module: Option<String> = None;
    for line in source.lines() {
        let trimmed = line
            .split('#')
            .next()
            .unwrap_or(line)
            .trim()
            .trim_end_matches(';')
            .trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(module) = multiline_module.as_deref() {
            let entries = trimmed.trim_end_matches(')').trim();
            if let Some(source_name) = imported_source_name(entries, binding_name) {
                return Some(SourceImportLookup {
                    import_module: module.to_string(),
                    source_name,
                });
            }
            if trimmed.contains(')') {
                multiline_module = None;
            } else {
                multiline_module = Some(module.to_string());
            }
            continue;
        }
        let Some(rest) = trimmed.strip_prefix("from ") else {
            continue;
        };
        let Some((module, names)) = rest
            .split_once(" import ")
            .or_else(|| rest.split_once(" cimport "))
        else {
            continue;
        };
        let module = module.trim();
        let names = names.trim_start();
        if names == "*" {
            continue;
        }
        if let Some(after_open) = names.strip_prefix('(') {
            let entries = after_open.trim_end_matches(')').trim();
            if let Some(source_name) = imported_source_name(entries, binding_name) {
                return Some(SourceImportLookup {
                    import_module: module.to_string(),
                    source_name,
                });
            }
            if !names.contains(')') {
                multiline_module = Some(module.to_string());
            }
            continue;
        }
        if let Some(source_name) = imported_source_name(names, binding_name) {
            return Some(SourceImportLookup {
                import_module: module.to_string(),
                source_name,
            });
        }
    }
    None
}

fn source_imported_sage_all_lookup(source: &str, binding_name: &str) -> Option<SourceImportLookup> {
    let mut star_imports = Vec::new();
    let mut multiline_module: Option<String> = None;
    for line in source.lines() {
        let trimmed = line
            .split('#')
            .next()
            .unwrap_or(line)
            .trim()
            .trim_end_matches(';')
            .trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(module) = multiline_module.as_deref() {
            let entries = trimmed.trim_end_matches(')').trim();
            if let Some(source_name) = imported_source_name(entries, binding_name) {
                return Some(SourceImportLookup {
                    import_module: module.to_string(),
                    source_name,
                });
            }
            if trimmed.contains(')') {
                multiline_module = None;
            } else {
                multiline_module = Some(module.to_string());
            }
            continue;
        }
        let Some(rest) = trimmed.strip_prefix("from ") else {
            continue;
        };
        let Some((module, names)) = rest
            .split_once(" import ")
            .or_else(|| rest.split_once(" cimport "))
        else {
            continue;
        };
        let module = module.trim();
        if !module_is_sage_all_export_module(module) {
            continue;
        }
        let names = names.trim_start();
        if names == "*" {
            star_imports.push(module.to_string());
            continue;
        }
        if let Some(after_open) = names.strip_prefix('(') {
            let entries = after_open.trim_end_matches(')').trim();
            if let Some(source_name) = imported_source_name(entries, binding_name) {
                return Some(SourceImportLookup {
                    import_module: module.to_string(),
                    source_name,
                });
            }
            if !names.contains(')') {
                multiline_module = Some(module.to_string());
            }
            continue;
        }
        if let Some(source_name) = imported_source_name(names, binding_name) {
            return Some(SourceImportLookup {
                import_module: module.to_string(),
                source_name,
            });
        }
    }
    star_imports
        .into_iter()
        .next()
        .map(|import_module| SourceImportLookup {
            import_module,
            source_name: binding_name.to_string(),
        })
}

fn is_sage_source_path(path: &Path) -> bool {
    path.extension().is_some_and(|ext| ext == "sage")
}

fn imported_source_name(entries: &str, binding_name: &str) -> Option<String> {
    entries.split(',').find_map(|entry| {
        let binding = parse_imported_binding(entry)?;
        (binding.binding == binding_name).then_some(binding.source_name)
    })
}

fn parse_multiline_from_import_start(line: &str) -> Option<(String, &str)> {
    let line = line.strip_prefix("from ")?;
    let (module, rest) = line.split_once(" import ")?;
    let rest = rest.trim_start();
    let rest = rest.strip_prefix('(')?;
    Some((module.trim().to_string(), rest))
}

struct MultilineImportCapture<'a> {
    text: &'a str,
    original_line: &'a str,
    line_start: usize,
    import_module: &'a str,
}

fn capture_multiline_import_names(
    module: &str,
    path: &Path,
    capture: MultilineImportCapture<'_>,
    code_map: &CodeMap,
    symbols: &mut Vec<SymbolRecord>,
) {
    let text = capture
        .text
        .split('#')
        .next()
        .unwrap_or(capture.text)
        .replace(')', "");
    for entry in text.split(',') {
        let Some(binding) = parse_imported_binding(entry) else {
            continue;
        };
        if let Some(relative) = capture.original_line.find(&binding.binding) {
            push_import_symbol(
                symbols,
                module,
                path,
                &binding.binding,
                code_map,
                capture.line_start + relative,
                &format!("{}::{}", capture.import_module, binding.source_name),
            );
        }
    }
}

fn parse_plain_import(line: &str) -> Option<Vec<(String, String)>> {
    let rest = line
        .strip_prefix("import ")
        .or_else(|| line.strip_prefix("cimport "))?;
    Some(
        rest.split(',')
            .filter_map(|entry| {
                let module = entry.split_whitespace().next()?.to_string();
                let binding = entry
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .windows(2)
                    .find_map(|window| (window[0] == "as").then(|| window[1].to_string()))
                    .unwrap_or_else(|| module.split('.').next().unwrap_or(&module).to_string());
                is_valid_identifier(&binding).then_some((binding, module))
            })
            .collect(),
    )
}

fn parse_cython_include(line: &str) -> Option<String> {
    let rest = line.strip_prefix("include ")?;
    Some(rest.trim().trim_matches('"').trim_matches('\'').to_string())
}

fn sage_load_attach_paths_before_line(
    query_path: &Path,
    source: &str,
    max_line: u32,
) -> Vec<PathBuf> {
    let base_dir = query_path.parent().unwrap_or_else(|| Path::new("."));
    let code_map = CodeMap::new(source);
    let mut paths = Vec::new();
    let mut seen = BTreeSet::new();
    for captures in load_attach_call_re().captures_iter(source) {
        let Some(call) = captures.get(0) else {
            continue;
        };
        if !code_map.is_code_offset(call.start()) {
            continue;
        }
        let (line, _) = code_map.line_col(call.start());
        if line > max_line {
            continue;
        }
        let Some(target) = captures
            .name("double")
            .or_else(|| captures.name("single"))
            .map(|value| value.as_str())
        else {
            continue;
        };
        if target.trim().is_empty() {
            continue;
        }
        let target_path = PathBuf::from(target);
        let resolved = if target_path.is_absolute() {
            target_path
        } else {
            base_dir.join(target_path)
        };
        let resolved = normalize_path(resolved);
        if seen.insert(resolved.clone()) {
            paths.push(resolved);
        }
    }
    paths
}

fn load_attach_call_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r#"\b(?:load|attach)\s*\(\s*(?:"(?P<double>[^"\n]+)"|'(?P<single>[^'\n]+)')"#)
            .unwrap()
    })
}

fn parse_imported_binding(entry: &str) -> Option<ImportedBinding> {
    let entry = entry.trim();
    if entry.is_empty() || entry == "*" {
        return None;
    }
    let parts: Vec<_> = entry.split_whitespace().collect();
    let source_name = parts
        .first()
        .copied()
        .filter(|value| is_valid_identifier(value))?;
    let binding = parts
        .iter()
        .position(|part| *part == "as")
        .and_then(|alias_index| parts.get(alias_index + 1).copied())
        .unwrap_or(source_name);
    is_valid_identifier(binding).then(|| ImportedBinding {
        binding: binding.to_string(),
        source_name: source_name.to_string(),
    })
}

fn sage_export_import_from(import_from: &str, name: &str) -> Option<String> {
    let module = import_from
        .split_once("::")
        .map_or(import_from, |(module, _)| module);
    let target = SAGE_EXPORT_MAP
        .iter()
        .find(|target| target.import_module == module && target.name == name)
        .or_else(|| {
            (module == "sage.all")
                .then(|| {
                    SAGE_EXPORT_MAP
                        .iter()
                        .find(|target| target.name == name && target.import_module != "sage.all")
                })
                .flatten()
        })?;
    Some(format!("{}::{}", target.source_module, target.source_name))
}

#[derive(Clone, Copy, Debug)]
struct SageExportTarget {
    import_module: &'static str,
    name: &'static str,
    source_module: &'static str,
    source_name: &'static str,
}

fn push_import_symbol(
    symbols: &mut Vec<SymbolRecord>,
    module: &str,
    path: &Path,
    name: &str,
    code_map: &CodeMap,
    offset: usize,
    import_from: &str,
) {
    let import_from = normalize_import_from(import_from, module, name);
    let import_from =
        sage_export_import_from(&import_from, name).unwrap_or_else(|| import_from.to_string());
    let (line, character) = code_map.line_col(offset);
    symbols.push(SymbolRecord {
        name: name.to_string(),
        kind: SymbolKind::Import,
        module: module.to_string(),
        path: path.to_path_buf(),
        range: SourceRange {
            start_line: line,
            start_character: character,
            end_line: line,
            end_character: character + name.len() as u32,
        },
        detail: format!("Import {name} from {import_from}"),
        docstring: None,
        import_from: Some(import_from),
        signature: None,
    });
}

fn line_offsets(source: &str) -> Vec<(usize, &str)> {
    let mut result = Vec::new();
    let mut offset = 0usize;
    for line in source.lines() {
        result.push((offset, line));
        offset += line.len() + 1;
    }
    result
}

fn push_simple_symbol(
    symbols: &mut Vec<SymbolRecord>,
    module: &str,
    path: &Path,
    name: &str,
    kind: SymbolKind,
    code_map: &CodeMap,
    offset: usize,
) {
    let detail = format!("{:?} {}", kind, name);
    let context = SymbolPushContext {
        module,
        path,
        code_map,
    };
    push_symbol_with_detail(symbols, &context, name, kind, offset, detail);
}

struct SymbolPushContext<'a> {
    module: &'a str,
    path: &'a Path,
    code_map: &'a CodeMap,
}

fn push_symbol_with_detail(
    symbols: &mut Vec<SymbolRecord>,
    context: &SymbolPushContext<'_>,
    name: &str,
    kind: SymbolKind,
    offset: usize,
    detail: String,
) {
    let (line, character) = context.code_map.line_col(offset);
    symbols.push(SymbolRecord {
        name: name.to_string(),
        kind,
        module: context.module.to_string(),
        path: context.path.to_path_buf(),
        range: SourceRange {
            start_line: line,
            start_character: character,
            end_line: line,
            end_character: character + name.len() as u32,
        },
        detail,
        docstring: None,
        import_from: None,
        signature: None,
    });
}

fn function_signature(source: &str, offset: usize, name: &str) -> Option<String> {
    let line_start = source[..offset].rfind('\n').map_or(0, |index| index + 1);
    let line_end = source[offset..]
        .find('\n')
        .map_or(source.len(), |index| offset + index);
    let header_end = definition_header_end(source, offset).unwrap_or(line_end);
    let header = source[line_start..header_end]
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let name_offset = header.find(name)?;
    let rest = &header[name_offset + name.len()..];
    let open = rest.find('(')?;
    let close = rest.rfind(')')?;
    if close < open {
        return None;
    }
    Some(format!("{}{}", name, &rest[open..=close]))
}

fn diagnostics_for_source(path: &Path, source: &str) -> Vec<DiagnosticRecord> {
    if let Some(caret) = sage_trailing_caret_error(source) {
        return vec![DiagnosticRecord {
            message: "Syntax error: incomplete Sage exponentiation".to_string(),
            range: caret,
            code: "syntax-error".to_string(),
            severity: "error".to_string(),
        }];
    }
    let mut diagnostics = if path.extension().is_some_and(|ext| ext == "py")
        && source_looks_sage_heavy_python(source)
    {
        sage_python_caret_exponent_diagnostics(source)
    } else {
        Vec::new()
    };
    if path
        .extension()
        .is_some_and(|ext| ext == "pyx" || ext == "pxd" || ext == "pxi")
    {
        return diagnostics;
    }
    let generated = if path.extension().is_some_and(|ext| ext == "sage") {
        preprocess_sage_source(source).generated
    } else {
        source.to_string()
    };
    let Some(tree) = parse_with_tree_sitter(&generated) else {
        return diagnostics;
    };
    if tree.root_node().has_error() {
        diagnostics.push(DiagnosticRecord {
            message: "Syntax error: source could not be parsed".to_string(),
            range: SourceRange {
                start_line: 0,
                start_character: 0,
                end_line: 0,
                end_character: 1,
            },
            code: "syntax-error".to_string(),
            severity: "error".to_string(),
        });
    }
    diagnostics
}

fn sage_trailing_caret_error(source: &str) -> Option<SourceRange> {
    for (line_index, line) in source.lines().enumerate() {
        let trimmed = line.trim_end();
        if trimmed.ends_with('^') {
            let character = line.rfind('^')? as u32;
            return Some(SourceRange {
                start_line: line_index as u32,
                start_character: character,
                end_line: line_index as u32,
                end_character: character + 1,
            });
        }
    }
    None
}

fn source_looks_sage_heavy_python(source: &str) -> bool {
    source.contains("from sage.all import")
        || source.contains("import sage.all")
        || source.contains("from sage.")
        || source.contains("from sage_")
}

fn sage_python_caret_exponent_diagnostics(source: &str) -> Vec<DiagnosticRecord> {
    let code_map = CodeMap::new(source);
    let bytes = source.as_bytes();
    let mut diagnostics = Vec::new();
    for (offset, byte) in bytes.iter().enumerate() {
        if *byte != b'^'
            || !code_map.is_code_offset(offset)
            || !looks_like_binary_caret_expression(bytes, &code_map, offset)
        {
            continue;
        }
        let (line, character) = code_map.line_col(offset);
        diagnostics.push(DiagnosticRecord {
            message:
                "Sage-style exponent operator `^` has Python XOR semantics in `.py`; use `**`."
                    .to_string(),
            range: SourceRange {
                start_line: line,
                start_character: character,
                end_line: line,
                end_character: character + 1,
            },
            code: "sage-python-caret-exponent".to_string(),
            severity: "warning".to_string(),
        });
    }
    diagnostics
}

fn looks_like_binary_caret_expression(bytes: &[u8], code_map: &CodeMap, offset: usize) -> bool {
    let Some(left) = nearest_code_byte_before(bytes, code_map, offset) else {
        return false;
    };
    let Some(right) = nearest_code_byte_after(bytes, code_map, offset + 1) else {
        return false;
    };
    is_caret_operand_end(left) && is_caret_operand_start(right)
}

fn nearest_code_byte_before(bytes: &[u8], code_map: &CodeMap, offset: usize) -> Option<u8> {
    let mut index = offset;
    while index > 0 {
        index -= 1;
        if bytes[index].is_ascii_whitespace() || !code_map.is_code_offset(index) {
            continue;
        }
        return Some(bytes[index]);
    }
    None
}

fn nearest_code_byte_after(bytes: &[u8], code_map: &CodeMap, offset: usize) -> Option<u8> {
    let mut index = offset;
    while index < bytes.len() {
        if bytes[index].is_ascii_whitespace() || !code_map.is_code_offset(index) {
            index += 1;
            continue;
        }
        return Some(bytes[index]);
    }
    None
}

fn is_caret_operand_end(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b')' | b']')
}

fn is_caret_operand_start(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'(' | b'[' | b'+' | b'-')
}

pub fn references_in_source(path: &Path, source: &str, name: &str) -> Vec<ReferenceRecord> {
    if name.is_empty() {
        return Vec::new();
    }
    reference_spans_in_source(path, source)
        .into_iter()
        .filter_map(|(candidate, reference)| (candidate == name).then_some(reference))
        .collect()
}

fn reference_spans_in_source(path: &Path, source: &str) -> Vec<(String, ReferenceRecord)> {
    let mut records = Vec::new();
    let code_map = CodeMap::new(source);
    for captures in word_re().captures_iter(source) {
        let Some(candidate) = captures.name("name") else {
            continue;
        };
        if !code_map.is_code_offset(candidate.start()) {
            continue;
        }
        let name = candidate.as_str();
        let (line, character) = code_map.line_col(candidate.start());
        records.push((
            name.to_string(),
            ReferenceRecord {
                path: path.to_path_buf(),
                range: SourceRange {
                    start_line: line,
                    start_character: character,
                    end_line: line,
                    end_character: character + name.len() as u32,
                },
            },
        ));
    }
    records
}

pub fn is_code_reference_at_range(source: &str, name: &str, range: &SourceRange) -> bool {
    if name.is_empty() {
        return false;
    }
    let code_map = CodeMap::new(source);
    let Some(start) = code_map.offset(range.start_line, range.start_character) else {
        return false;
    };
    let Some(end) = code_map.offset(range.end_line, range.end_character) else {
        return false;
    };
    if start >= end || !code_map.is_code_offset(start) {
        return false;
    }
    let bytes = source.as_bytes();
    if bytes.get(start..end) != Some(name.as_bytes()) {
        return false;
    }
    if start > 0 && is_word_byte(bytes[start - 1]) {
        return false;
    }
    if bytes.get(end).is_some_and(|byte| is_word_byte(*byte)) {
        return false;
    }
    true
}

fn dedupe_reference_records(references: Vec<ReferenceRecord>) -> Vec<ReferenceRecord> {
    let mut seen = BTreeSet::new();
    let mut deduped = Vec::new();
    for reference in references {
        let key = (
            reference.path.clone(),
            reference.range.start_line,
            reference.range.start_character,
            reference.range.end_line,
            reference.range.end_character,
        );
        if seen.insert(key) {
            deduped.push(reference);
        }
    }
    deduped
}

fn scope_references_for_resolved_symbol(
    references: Vec<ReferenceRecord>,
    resolved: Option<&SymbolRecord>,
    query_path: &Path,
) -> Vec<ReferenceRecord> {
    let Some(resolved) = resolved else {
        return references;
    };
    if !matches!(
        resolved.kind,
        SymbolKind::Variable | SymbolKind::PreparserGenerator
    ) || resolved.path != query_path
    {
        return references;
    }
    references
        .into_iter()
        .filter(|reference| reference.path == resolved.path)
        .collect()
}

fn create_schema(connection: &Connection) -> Result<()> {
    connection.execute_batch(
        r#"
        pragma journal_mode = wal;
        create table if not exists files(
          path text primary key,
          module text not null,
          fingerprint text not null
        );
        create table if not exists symbols(
          name text not null,
          kind text not null,
          module text not null,
          path text not null,
          start_line integer not null,
          start_character integer not null,
          end_line integer not null,
          end_character integer not null,
          detail text not null,
          import_from text,
          signature text
        );
        create table if not exists docs(
          name text not null,
          module text not null,
          path text not null,
          detail text not null,
          docstring text not null
        );
        create table if not exists reference_spans(
          name text not null,
          path text not null,
          start_line integer not null,
          start_character integer not null,
          end_line integer not null,
          end_character integer not null
        );
        create virtual table if not exists docs_fts using fts5(name, module, docstring);
        create table if not exists runtime_docs(
          symbol text primary key,
          name text not null,
          module_name text not null,
          kind text not null,
          detail text not null,
          summary text not null,
          docstring text,
          uri text,
          updated_at integer not null
        );
        create table if not exists index_root_metadata(
          root text primary key,
          file_count integer not null,
          symbol_count integer not null,
          doc_count integer not null,
          updated_at integer not null,
          root_fingerprint text,
          root_marker text
        );
        create table if not exists sage_export_cache(
          public_name text not null,
          source_name text not null,
          import_module text not null,
          reason text not null,
          name text not null,
          kind text not null,
          module text not null,
          path text not null,
          start_line integer not null,
          start_character integer not null,
          end_line integer not null,
          end_character integer not null,
          detail text not null,
          import_from text,
          signature text,
          docstring text,
          primary key(import_module, public_name)
        );
        create table if not exists sage_method_cache(
          owner_type text not null,
          member text not null,
          origin text not null default 'unknown',
          name text not null,
          kind text not null,
          module text not null,
          path text not null,
          start_line integer not null,
          start_character integer not null,
          end_line integer not null,
          end_character integer not null,
          detail text not null,
          import_from text,
          signature text,
          docstring text,
          primary key(owner_type, member)
        );
        "#,
    )?;
    create_lookup_indexes(connection)?;
    ensure_column(connection, "symbols", "import_from", "text")?;
    ensure_column(connection, "symbols", "signature", "text")?;
    ensure_column(
        connection,
        "sage_method_cache",
        "origin",
        "text not null default 'unknown'",
    )?;
    ensure_column(
        connection,
        "index_root_metadata",
        "root_fingerprint",
        "text",
    )?;
    ensure_column(connection, "index_root_metadata", "root_marker", "text")?;
    Ok(())
}

fn ensure_column(
    connection: &Connection,
    table: &str,
    column: &str,
    column_type: &str,
) -> Result<()> {
    let mut statement = connection.prepare(&format!("pragma table_info({table})"))?;
    let rows = statement.query_map([], |row| row.get::<_, String>(1))?;
    for row in rows {
        if row? == column {
            return Ok(());
        }
    }
    connection.execute(
        &format!("alter table {table} add column {column} {column_type}"),
        [],
    )?;
    Ok(())
}

fn symbol_resolution_rank(kind: &SymbolKind) -> u8 {
    match kind {
        SymbolKind::Class | SymbolKind::Function | SymbolKind::CythonDeclaration => 0,
        SymbolKind::PreparserGenerator | SymbolKind::Variable => 1,
        SymbolKind::Module => 2,
        SymbolKind::Import => 3,
    }
}

fn symbol_path_rank(path: &Path) -> u8 {
    match path.extension().and_then(|ext| ext.to_str()) {
        Some("py" | "sage") => 0,
        Some("pyx") => 1,
        Some("pxd") => 2,
        Some("pxi") => 3,
        _ => 4,
    }
}

fn symbol_choice_key(symbol: &SymbolRecord) -> (u8, u8, u8) {
    (
        symbol_resolution_rank(&symbol.kind),
        symbol_doc_rank(symbol),
        symbol_path_rank(&symbol.path),
    )
}

type SageMethodChoiceKey = (u8, u8, u8, u8);

fn sage_method_choice_key(priority: u8, symbol: &SymbolRecord) -> SageMethodChoiceKey {
    let (resolution_rank, doc_rank, path_rank) = symbol_choice_key(symbol);
    (priority, resolution_rank, doc_rank, path_rank)
}

fn symbol_doc_rank(symbol: &SymbolRecord) -> u8 {
    match symbol.docstring.as_deref() {
        Some(docstring) if !docstring.trim().is_empty() => 0,
        _ => 1,
    }
}

fn source_derived_method_owner_for_symbol(
    symbol: &SymbolRecord,
) -> Option<SourceDerivedMethodOwner> {
    if !is_source_derived_sage_method(symbol) {
        return None;
    }
    if let Some(owner) = source_derived_method_owner_from_method_detail(symbol) {
        return Some(owner);
    }
    let module_spec = SAGE_OWNER_METHOD_MODULES
        .iter()
        .filter(|spec| module_matches_owner_module_spec(&symbol.module, spec))
        .min_by_key(|spec| spec.priority)?;
    Some(SourceDerivedMethodOwner {
        owner_type: module_spec.owner_type,
        priority: module_spec.priority,
    })
}

fn source_derived_method_owner_from_method_detail(
    symbol: &SymbolRecord,
) -> Option<SourceDerivedMethodOwner> {
    let (class_name, _) = method_detail_parts(&symbol.detail)?;
    let owner_type = sage_owner_type_from_class_name(class_name, &symbol.module)?;
    Some(SourceDerivedMethodOwner {
        owner_type,
        priority: source_derived_method_detail_priority(owner_type, class_name, &symbol.module),
    })
}

fn source_derived_method_detail_priority(
    owner_type: SageOwnerType,
    class_name: &str,
    module: &str,
) -> u8 {
    let lower = class_name.to_ascii_lowercase();
    match owner_type {
        SageOwnerType::Graph => {
            if module == "sage.graphs.generic_graph" || lower == "genericgraph" {
                0
            } else if (module == "sage.graphs.graph" && lower == "graph")
                || (module == "sage.graphs.digraph" && lower == "digraph")
            {
                5
            } else if module.starts_with("sage.graphs.") {
                30
            } else {
                60
            }
        }
        SageOwnerType::PolynomialRing => polynomial_ring_source_priority(module),
        SageOwnerType::PolynomialElement => polynomial_element_source_priority(module),
        SageOwnerType::EllipticCurve => elliptic_curve_source_priority(module, &lower),
        SageOwnerType::Matrix => matrix_source_priority(module, &lower),
        SageOwnerType::Ideal if !module.starts_with("sage.rings.polynomial") => 60,
        SageOwnerType::Field | SageOwnerType::FieldElement
            if !module.starts_with("sage.rings.finite_rings") =>
        {
            60
        }
        SageOwnerType::Vector if !module.starts_with("sage.modules") => 60,
        SageOwnerType::NumberField if !module.starts_with("sage.rings.number_field") => 60,
        _ => 0,
    }
}

fn matrix_source_priority(module: &str, lower_class_name: &str) -> u8 {
    if lower_class_name == "matrix"
        || matches!(
            module,
            "sage.matrix.matrix0" | "sage.matrix.matrix1" | "sage.matrix.matrix2"
        )
    {
        return 0;
    }
    match module {
        "sage.matrix.matrix_dense" | "sage.matrix.matrix_sparse" => 5,
        module if module.starts_with("sage.matrix.") => 30,
        _ => 60,
    }
}

fn polynomial_ring_source_priority(module: &str) -> u8 {
    match module {
        "sage.rings.polynomial.multi_polynomial_libsingular" => 0,
        "sage.rings.polynomial.polynomial_ring" | "sage.rings.polynomial.multi_polynomial_ring" => {
            10
        }
        "sage.structure.parent_gens"
        | "sage.structure.parent"
        | "sage.structure.category_object" => 20,
        module if module.starts_with("sage.rings.polynomial.") => 40,
        _ => 60,
    }
}

fn polynomial_element_source_priority(module: &str) -> u8 {
    match module {
        "sage.rings.polynomial.multi_polynomial"
        | "sage.rings.polynomial.multi_polynomial_element" => 0,
        "sage.rings.polynomial.polynomial_element" => 10,
        "sage.rings.polynomial.polynomial_element_generic" => 20,
        "sage.structure.element" => 30,
        module if module.starts_with("sage.rings.polynomial.") => 40,
        _ => 60,
    }
}

fn elliptic_curve_source_priority(module: &str, lower_class_name: &str) -> u8 {
    if lower_class_name == "ellipticcurves" {
        return 80;
    }
    match module {
        "sage.schemes.elliptic_curves.ell_generic" => 0,
        "sage.schemes.elliptic_curves.ell_rational_field" => 5,
        "sage.schemes.elliptic_curves.ell_finite_field" => 8,
        "sage.schemes.elliptic_curves.ell_field" => 10,
        "sage.schemes.elliptic_curves.ell_number_field" => 15,
        module if module.starts_with("sage.schemes.elliptic_curves.") => 60,
        _ => 80,
    }
}

fn method_detail_parts(detail: &str) -> Option<(&str, &str)> {
    detail.strip_prefix("Method ")?.split_once('.')
}

fn class_method_alias_detail_parts(detail: &str) -> Option<(&str, &str, &str)> {
    let (class_and_alias, target) = detail.strip_prefix("MethodAlias ")?.split_once(" for ")?;
    let (class_name, alias) = class_and_alias.rsplit_once('.')?;
    Some((class_name, alias, target))
}

fn matrix_constructor_method_alias_detail_parts(detail: &str) -> Option<(&str, &str)> {
    let (alias, target) = detail
        .strip_prefix("MatrixConstructorMethodAlias matrix.")?
        .split_once(" for ")?;
    Some((alias, target))
}

fn sage_owner_type_from_class_name(class_name: &str, module: &str) -> Option<SageOwnerType> {
    let lower = class_name.to_ascii_lowercase();
    if module.starts_with("sage.matrix") && lower.contains("matrix") {
        return Some(SageOwnerType::Matrix);
    }
    if module.starts_with("sage.modules.free_module_element")
        && (lower.contains("vector") || lower.contains("free_module_element"))
    {
        return Some(SageOwnerType::Vector);
    }
    if module.starts_with("sage.modules.free_module") && lower.contains("free_module") {
        return Some(SageOwnerType::FreeModule);
    }
    if module.starts_with("sage.rings.polynomial") {
        if lower.contains("ideal") {
            return Some(SageOwnerType::Ideal);
        }
        if lower.contains("polynomialring")
            || lower.contains("mpolynomialring")
            || lower.contains("booleanpolynomialring")
            || lower.ends_with("ring")
        {
            return Some(SageOwnerType::PolynomialRing);
        }
        if lower.contains("polynomial") || lower.contains("polydict") {
            return Some(SageOwnerType::PolynomialElement);
        }
    }
    if module.starts_with("sage.rings.finite_rings") {
        if lower.contains("element") {
            return Some(SageOwnerType::FieldElement);
        }
        if lower.contains("field") {
            return Some(SageOwnerType::Field);
        }
    }
    if module.starts_with("sage.") && lower.contains("ellipticcurve") {
        return Some(SageOwnerType::EllipticCurve);
    }
    if module.starts_with("sage.") && lower.contains("numberfield") {
        return Some(SageOwnerType::NumberField);
    }
    if module.starts_with("sage.") && lower.contains("graph") {
        return Some(SageOwnerType::Graph);
    }
    None
}

fn module_matches_owner_module_spec(module: &str, spec: &SageOwnerModuleSpec) -> bool {
    module == spec.module || (spec.recursive && module.starts_with(&format!("{}.", spec.module)))
}

fn is_source_derived_sage_method(symbol: &SymbolRecord) -> bool {
    if matches!(
        symbol.kind,
        SymbolKind::Import | SymbolKind::Module | SymbolKind::Class
    ) {
        return false;
    }
    if symbol.name.starts_with("__") && symbol.name.ends_with("__") {
        return false;
    }
    symbol
        .signature
        .as_deref()
        .is_some_and(signature_has_self_receiver)
}

fn signature_has_self_receiver(signature: &str) -> bool {
    let Some(open) = signature.find('(') else {
        return false;
    };
    let Some(close) = signature[open + 1..].find([',', ')']) else {
        return false;
    };
    let first_parameter = signature[open + 1..open + 1 + close].trim();
    first_parameter
        .split_whitespace()
        .next_back()
        .is_some_and(|name| name == "self")
}

#[derive(Clone, Debug)]
struct CodeMap {
    code: Vec<bool>,
    line_starts: Vec<usize>,
}

impl CodeMap {
    fn new(source: &str) -> Self {
        let bytes = source.as_bytes();
        let mut code = vec![true; bytes.len()];
        let mut line_starts = vec![0usize];
        for (index, byte) in bytes.iter().enumerate() {
            if *byte == b'\n' && index + 1 < bytes.len() {
                line_starts.push(index + 1);
            }
        }
        mark_non_code_ranges(bytes, &mut code);
        Self { code, line_starts }
    }

    fn is_code_offset(&self, offset: usize) -> bool {
        self.code.get(offset).copied().unwrap_or(false)
    }

    fn line_col(&self, offset: usize) -> (u32, u32) {
        let line_index = self
            .line_starts
            .partition_point(|line_start| *line_start <= offset)
            .saturating_sub(1);
        let line_start = self.line_starts.get(line_index).copied().unwrap_or(0);
        (line_index as u32, offset.saturating_sub(line_start) as u32)
    }

    fn offset(&self, line: u32, character: u32) -> Option<usize> {
        let line_start = *self.line_starts.get(line as usize)?;
        let next_line_start = self
            .line_starts
            .get(line as usize + 1)
            .copied()
            .unwrap_or(self.code.len());
        Some((line_start + character as usize).min(next_line_start))
    }
}

fn mark_non_code_ranges(bytes: &[u8], code: &mut [bool]) {
    let mut index = 0usize;
    let mut quote: Option<u8> = None;
    let mut triple: Option<&'static [u8]> = None;
    while index < bytes.len() {
        if let Some(marker) = triple {
            if bytes[index..].starts_with(marker) {
                mark_range(code, index, index + marker.len());
                index += marker.len();
                triple = None;
            } else {
                code[index] = false;
                index += 1;
            }
            continue;
        }
        if let Some(current_quote) = quote {
            if bytes[index] == b'\\' {
                let end = (index + 2).min(bytes.len());
                mark_range(code, index, end);
                index = end;
                continue;
            }
            code[index] = false;
            if bytes[index] == current_quote {
                quote = None;
            }
            index += 1;
            continue;
        }
        if bytes[index] == b'#' {
            while index < bytes.len() && bytes[index] != b'\n' {
                code[index] = false;
                index += 1;
            }
            continue;
        }
        if bytes[index..].starts_with(b"'''") {
            mark_range(code, index, index + 3);
            triple = Some(b"'''");
            index += 3;
            continue;
        }
        if bytes[index..].starts_with(b"\"\"\"") {
            mark_range(code, index, index + 3);
            triple = Some(b"\"\"\"");
            index += 3;
            continue;
        }
        if bytes[index] == b'\'' || bytes[index] == b'"' {
            code[index] = false;
            quote = Some(bytes[index]);
        }
        index += 1;
    }
}

fn mark_range(code: &mut [bool], start: usize, end: usize) {
    let end = end.min(code.len());
    for slot in &mut code[start..end] {
        *slot = false;
    }
}

fn file_fingerprint(path: &Path) -> Result<String> {
    let metadata = fs::metadata(path)?;
    let modified_ns = metadata
        .modified()?
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_nanos();
    Ok(format!("{}:{}", metadata.len(), modified_ns))
}

fn source_root_fingerprint(root: &Path) -> SourceRootFingerprint {
    let mut hasher = Sha256::new();
    let root_text = root.display().to_string();
    hasher.update(root_text.as_bytes());
    hasher.update([0]);

    let mut first_marker = None;
    if root.exists() {
        if let Ok(fingerprint) = file_fingerprint(root) {
            hasher.update(fingerprint.as_bytes());
            hasher.update([1]);
        }
    } else {
        hasher.update(b"missing");
        hasher.update([1]);
    }

    for marker in source_root_marker_candidates(root) {
        if !marker.exists() {
            continue;
        }
        first_marker.get_or_insert_with(|| marker.display().to_string());
        hasher.update(marker.display().to_string().as_bytes());
        hasher.update([2]);
        if let Ok(fingerprint) = file_fingerprint(&marker) {
            hasher.update(fingerprint.as_bytes());
        }
        if let Ok(content) = fs::read(&marker) {
            let limit = content.len().min(64 * 1024);
            hasher.update(&content[..limit]);
            if marker.file_name().and_then(|name| name.to_str()) == Some("HEAD") {
                if let Some(reference) = git_head_reference(&marker, &content) {
                    if let Ok(reference_content) = fs::read(&reference) {
                        hasher.update(reference.display().to_string().as_bytes());
                        hasher.update([3]);
                        hasher.update(reference_content);
                    }
                }
            }
        }
        hasher.update([4]);
    }

    SourceRootFingerprint {
        root: root_text,
        exists: root.exists(),
        digest: format!("{:x}", hasher.finalize())[..16].to_string(),
        marker: first_marker,
    }
}

fn source_root_fingerprints_for_roots(roots: &[PathBuf]) -> Vec<SourceRootFingerprint> {
    roots
        .iter()
        .map(|root| source_root_fingerprint(root))
        .collect()
}

fn source_root_marker_candidates(root: &Path) -> Vec<PathBuf> {
    let mut candidates = vec![
        root.join("sage").join("version.py"),
        root.join("sage").join("all.py"),
        root.join("sage").join("env.py"),
        root.join(".git").join("HEAD"),
    ];
    if let Some(parent) = root.parent() {
        candidates.push(parent.join(".git").join("HEAD"));
    }
    candidates
}

fn git_head_reference(head_path: &Path, content: &[u8]) -> Option<PathBuf> {
    let text = std::str::from_utf8(content).ok()?.trim();
    let reference = text.strip_prefix("ref: ")?;
    Some(head_path.parent()?.join(reference))
}

fn path_is_under_roots(path: &Path, roots: &[PathBuf]) -> bool {
    roots.iter().any(|root| path.starts_with(root))
}

fn is_python_package_root(path: &Path) -> bool {
    matches!(
        path.file_name().and_then(|name| name.to_str()),
        Some("site-packages" | "dist-packages")
    )
}

fn normalize_existing_paths(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    normalize_paths(paths)
        .into_iter()
        .filter(|path| path.exists())
        .collect()
}

fn normalize_paths(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut paths: Vec<_> = paths.into_iter().map(normalize_path).collect();
    paths.sort();
    paths.dedup();
    paths
}

fn normalize_path(path: PathBuf) -> PathBuf {
    if let Ok(canonical) = path.canonicalize() {
        return canonical;
    }
    if let (Some(parent), Some(file_name)) = (path.parent(), path.file_name()) {
        if let Ok(canonical_parent) = parent.canonicalize() {
            return canonical_parent.join(file_name);
        }
    }
    path
}

fn cache_namespace_digest(roots: &[PathBuf], exclude_globs: &[String], enable_pyx: bool) -> String {
    let mut hasher = Sha256::new();
    hasher.update(CACHE_FORMAT_VERSION);
    hasher.update([0]);
    for root in roots {
        hasher.update(root.display().to_string());
        hasher.update([0]);
    }
    hasher.update([1]);
    for glob in exclude_globs {
        hasher.update(glob);
        hasher.update([0]);
    }
    hasher.update([2]);
    hasher.update([enable_pyx as u8]);
    format!("{:x}", hasher.finalize())[..16].to_string()
}

fn is_indexable(path: &Path, enable_pyx: bool) -> bool {
    match path.extension().and_then(|ext| ext.to_str()) {
        Some("py" | "sage") => true,
        Some("pyx" | "pxd" | "pxi" | "spyx") => enable_pyx,
        _ => false,
    }
}

fn is_cython_path(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|ext| ext.to_str()),
        Some("pyx" | "pxd" | "pxi" | "spyx")
    )
}

fn is_excluded(path: &Path, exclude_globs: &[String]) -> bool {
    let text = path.display().to_string();
    exclude_globs.iter().any(|glob| {
        let needle = glob.trim_matches('*').trim_matches('/');
        !needle.is_empty() && text.contains(needle)
    })
}

fn module_name_from_path(root: &Path, path: &Path) -> String {
    let relative = path.strip_prefix(root).unwrap_or(path);
    let mut parts: Vec<String> = relative
        .components()
        .filter_map(|component| component.as_os_str().to_str())
        .map(|part| part.to_string())
        .collect();
    if let Some(last) = parts.last_mut() {
        if let Some((stem, _)) = last.rsplit_once('.') {
            *last = stem.to_string();
        }
    }
    if parts.last().is_some_and(|part| part == "__init__") {
        parts.pop();
    }
    if parts.is_empty() {
        "document".to_string()
    } else {
        parts.join(".")
    }
}

fn module_source_path_from_roots(
    module: &str,
    roots: &[PathBuf],
    enable_pyx: bool,
) -> Option<PathBuf> {
    let relative = module.replace('.', "/");
    let mut suffixes = vec!["py", "sage"];
    if enable_pyx {
        suffixes.extend(["pyx", "pxd", "pxi", "spyx"]);
    }
    for root in roots {
        for suffix in &suffixes {
            let candidate = root.join(format!("{relative}.{suffix}"));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
        for suffix in &suffixes {
            let candidate = root.join(&relative).join(format!("__init__.{suffix}"));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

fn first_docstring(source: &str) -> Option<String> {
    triple_quoted_literal(source.trim_start())
}

fn doc_after_offset(source: &str, offset: usize) -> Option<String> {
    if let Some(header_end) = definition_header_end(source, offset) {
        return triple_quoted_literal(source[header_end + 1..].trim_start());
    }
    let after = &source[offset..];
    let line_end = after.find('\n')?;
    let after_line = after[line_end..].trim_start();
    triple_quoted_literal(after_line)
}

fn definition_header_end(source: &str, offset: usize) -> Option<usize> {
    let line_start = source[..offset].rfind('\n').map_or(0, |index| index + 1);
    let mut depth = 0usize;
    for (relative, ch) in source[line_start..].char_indices() {
        let absolute = line_start + relative;
        match ch {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            ':' if depth == 0 => return Some(absolute),
            '\n' if depth == 0 && absolute > offset => return None,
            _ => {}
        }
    }
    None
}

fn triple_quoted_literal(text: &str) -> Option<String> {
    for prefix in ["", "r", "u", "b", "f", "br", "rb", "fr", "rf", "ur", "ru"] {
        if text.len() < prefix.len() || !text[..prefix.len()].eq_ignore_ascii_case(prefix) {
            continue;
        }
        let candidate = &text[prefix.len()..];
        if candidate.starts_with("\"\"\"") || candidate.starts_with("'''") {
            let quote = &candidate[..3];
            let rest = &candidate[3..];
            let end = rest.find(quote)?;
            return Some(rest[..end].trim().to_string());
        }
    }
    None
}

fn class_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?m)^\s*(?:class|cdef\s+class|cpdef\s+class)\s+(?P<name>[A-Za-z_][A-Za-z0-9_]*)",
        )
        .unwrap()
    })
}

fn function_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?m)^\s*(?:async\s+def|def|cpdef|cdef)(?:\s+(?:inline|api|public|readonly|nogil|gil|except|const|unsigned|signed|long|short|char|int|float|double|void|object|bint|size_t|Py_ssize_t|[A-Za-z_][A-Za-z0-9_\.\*\[\]]*))*\s+(?P<name>[A-Za-z_][A-Za-z0-9_]*)\s*\(").unwrap())
}

fn preparser_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?P<parent>\b[A-Za-z_][A-Za-z0-9_]*\b)\.<(?P<symbols>[A-Za-z_][A-Za-z0-9_]*(?:\s*,\s*[A-Za-z_][A-Za-z0-9_]*)*)>",
        )
        .unwrap()
    })
}

fn preparser_assignment_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"^\s*(?P<parent>[A-Za-z_][A-Za-z0-9_]*)\.<(?P<symbols>[A-Za-z_][A-Za-z0-9_]*(?:\s*,\s*[A-Za-z_][A-Za-z0-9_]*)*)>\s*=\s*(?P<rhs>.+)$",
        )
        .unwrap()
    })
}

fn assignment_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?m)^(?P<name>[A-Za-z_][A-Za-z0-9_]*)\s*(?::(?P<annotation>[^=\n]+))?=\s*(?P<rhs>[^=\n].*)$").unwrap()
    })
}

fn semantic_assignment_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?m)^\s*(?P<name>[A-Za-z_][A-Za-z0-9_]*)\s*(?::[^=\n]+)?=\s*[^=\n]").unwrap()
    })
}

fn assignment_call_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"^(?P<callee>[A-Za-z_][A-Za-z0-9_]*(?:\.[A-Za-z_][A-Za-z0-9_]*)*)\s*\(")
            .unwrap()
    })
}

fn assignment_constructor_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"^(?P<name>[A-Za-z_][A-Za-z0-9_]*)\s*(?::[^=\n]+)?=\s*(?P<ctor>[A-Za-z_][A-Za-z0-9_]*(?:\.[A-Za-z_][A-Za-z0-9_]*)*)\s*\(",
        )
        .unwrap()
    })
}

fn simple_assignment_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"^(?P<name>[A-Za-z_][A-Za-z0-9_]*)\s*(?::[^=\n]+)?=\s*(?P<rhs>[^=\n].*)$")
            .unwrap()
    })
}

fn static_member_alias_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"^\s*(?P<name>[A-Za-z_][A-Za-z0-9_]*)\s*=\s*staticmethod\(\s*(?P<module>[A-Za-z_][A-Za-z0-9_]*)\.(?P<member>[A-Za-z_][A-Za-z0-9_]*)\s*\)",
        )
        .unwrap()
    })
}

fn member_alias_assignment_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"^(?P<name>[A-Za-z_][A-Za-z0-9_]*)\s*=\s*(?P<owner>[A-Za-z_][A-Za-z0-9_]*)\.(?P<member>[A-Za-z_][A-Za-z0-9_]*)$",
        )
        .unwrap()
    })
}

fn member_reference_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^[A-Za-z_][A-Za-z0-9_]*\.[A-Za-z_][A-Za-z0-9_]*$").unwrap())
}

fn deprecated_function_alias_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"^\s*(?P<name>[A-Za-z_][A-Za-z0-9_]*)\s*=\s*deprecated_function_alias\(\s*(?P<issue>[0-9]+)\s*,\s*(?P<target>[A-Za-z_][A-Za-z0-9_\.]*)",
        )
        .unwrap()
    })
}

fn function_header_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"^(?:async\s+def|def|cpdef|cdef)(?:\s+(?:inline|api|public|readonly|nogil|gil|except|const|unsigned|signed|long|short|char|int|float|double|void|object|bint|size_t|Py_ssize_t|[A-Za-z_][A-Za-z0-9_\.\*\[\]]*))*\s+(?P<name>[A-Za-z_][A-Za-z0-9_]*)\s*\(").unwrap()
    })
}

fn identifier_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^[A-Za-z_][A-Za-z0-9_]*$").unwrap())
}

fn word_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\b(?P<name>[A-Za-z_][A-Za-z0-9_]*)\b").unwrap())
}

fn decorator_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?m)^\s*@(?P<name>[A-Za-z_][A-Za-z0-9_\.]*)").unwrap())
}

fn matrix_method_name_override_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r#"name\s*=\s*(?:"(?P<double>[A-Za-z_][A-Za-z0-9_]*)"|'(?P<single>[A-Za-z_][A-Za-z0-9_]*)')"#,
        )
        .unwrap()
    })
}

const SAGE_NAMESPACES: &[&str] = &[
    "graphs",
    "toric_varieties",
    "simplicial_complexes",
    "simplicial_sets",
    "matroids",
    "codes",
    "channels",
    "groups",
    "manifolds",
    "cones",
    "crystals",
    "lie_algebras",
    "valuations",
    "finite_dynamical_systems",
    "mq",
    "plot3d",
];
const SAGE_STATIC_NAV_NAMESPACES: &[&str] = &[
    "graphs",
    "toric_varieties",
    "simplicial_complexes",
    "matroids",
    "mq",
    "plot3d",
];
const SAGE_READONLY: &[&str] = &[
    "ZZ", "QQ", "RR", "CC", "SR", "GF", "QQbar", "AA", "pi", "e", "I", "oo", "Infinity",
];
const SAGE_TYPES: &[&str] = &[
    "PolynomialRing",
    "PowerSeriesRing",
    "LaurentSeriesRing",
    "NumberField",
    "MatrixSpace",
    "EllipticCurve",
    "Graph",
    "DiGraph",
    "FreeModule",
    "VectorSpace",
    "FilteredSimplicialComplex",
    "ChowGroup",
    "ToricVariety",
    "Partitions",
    "SymmetricGroup",
    "BooleanFunction",
];
const SAGE_FUNCTIONS: &[&str] = &[
    "matrix",
    "vector",
    "zero_matrix",
    "zero_vector",
    "identity_matrix",
    "random_matrix",
    "random_vector",
    "set_random_seed",
    "var",
    "latex",
    "factor",
    "factorial",
    "integrate",
    "diff",
    "sqrt",
    "sin",
    "cos",
    "plot",
    "sigma",
    "lazy_import",
    "cached_method",
    "cached_function",
    "PetersenGraph",
    "CompleteGraph",
    "CycleGraph",
];

fn hot_sage_symbol_names() -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for target in SAGE_EXPORT_MAP {
        names.insert(target.name.to_string());
        names.insert(target.source_name.to_string());
    }
    names
}

fn hot_sage_method_keys() -> Vec<(SageOwnerType, &'static str)> {
    let mut seen = BTreeSet::new();
    let mut keys = Vec::new();
    for spec in SAGE_METHOD_SPECS {
        if seen.insert((spec.owner_type.as_str(), spec.member)) {
            keys.push((spec.owner_type, spec.member));
        }
    }
    for spec in SAGE_METHOD_ALIAS_SPECS {
        if seen.insert((spec.owner_type.as_str(), spec.member)) {
            keys.push((spec.owner_type, spec.member));
        }
    }
    keys
}

fn module_is_sage_all_export_module(module: &str) -> bool {
    module == "sage.all" || (module.starts_with("sage.") && module.ends_with(".all"))
}

fn is_star_import_symbol(symbol: &SymbolRecord) -> bool {
    symbol.kind == SymbolKind::Import && symbol.name == SAGE_STAR_IMPORT_SENTINEL
}

fn is_all_export_symbol(symbol: &SymbolRecord) -> bool {
    symbol.kind == SymbolKind::Import && symbol.name == SAGE_ALL_EXPORT_SENTINEL
}

fn all_export_name(symbol: &SymbolRecord) -> Option<&str> {
    if !is_all_export_symbol(symbol) {
        return None;
    }
    let import_from = symbol.import_from.as_deref()?;
    if import_from == SAGE_ALL_EXPORT_MARKER {
        return None;
    }
    import_from.strip_prefix("__all__::")
}

fn explicit_all_names_from_symbols<'a, I>(symbols: I) -> Option<BTreeSet<String>>
where
    I: IntoIterator<Item = &'a SymbolRecord>,
{
    let mut saw_all = false;
    let mut names = BTreeSet::new();
    for symbol in symbols {
        if !is_all_export_symbol(symbol) {
            continue;
        }
        saw_all = true;
        if let Some(name) = all_export_name(symbol) {
            names.insert(name.to_string());
        }
    }
    saw_all.then_some(names)
}

fn is_star_namespace_export_candidate(
    symbol: &SymbolRecord,
    explicit_names: Option<&BTreeSet<String>>,
) -> bool {
    if is_star_import_symbol(symbol)
        || is_all_export_symbol(symbol)
        || symbol.kind == SymbolKind::Module
        || symbol.name == "__all__"
    {
        return false;
    }
    if let Some(names) = explicit_names {
        names.contains(&symbol.name)
    } else {
        !symbol.name.starts_with('_')
    }
}

fn star_import_source_module(symbol: &SymbolRecord) -> Option<String> {
    if !is_star_import_symbol(symbol) {
        return None;
    }
    let import_from = symbol.import_from.as_ref()?;
    let (module, source_name) = import_target_in_context(import_from, "*", &symbol.module);
    (source_name == "*").then_some(module)
}

fn insert_import_symbol_hot_names(names: &mut BTreeSet<String>, symbol: &SymbolRecord) {
    if is_star_import_symbol(symbol) || is_all_export_symbol(symbol) {
        return;
    }
    insert_import_target_hot_names(
        names,
        &symbol.name,
        symbol.import_from.as_deref(),
        &symbol.module,
    );
}

fn insert_import_target_hot_names(
    names: &mut BTreeSet<String>,
    binding_name: &str,
    import_from: Option<&str>,
    importer_module: &str,
) {
    names.insert(binding_name.to_string());
    if let Some(import_from) = import_from {
        let (_source_module, source_name) =
            import_target_in_context(import_from, binding_name, importer_module);
        names.insert(source_name);
    }
}

const SAGE_EXPORT_MAP: &[SageExportTarget] = &[
    SageExportTarget {
        import_module: "sage.all",
        name: "GF",
        source_module: "sage.rings.finite_rings.finite_field_constructor",
        source_name: "GF",
    },
    SageExportTarget {
        import_module: "sage.rings.finite_rings.all",
        name: "GF",
        source_module: "sage.rings.finite_rings.finite_field_constructor",
        source_name: "GF",
    },
    SageExportTarget {
        import_module: "sage.all",
        name: "PolynomialRing",
        source_module: "sage.rings.polynomial.polynomial_ring_constructor",
        source_name: "PolynomialRing",
    },
    SageExportTarget {
        import_module: "sage.all",
        name: "NumberField",
        source_module: "sage.rings.number_field.number_field",
        source_name: "NumberField",
    },
    SageExportTarget {
        import_module: "sage.rings.number_field.all",
        name: "NumberField",
        source_module: "sage.rings.number_field.number_field",
        source_name: "NumberField",
    },
    SageExportTarget {
        import_module: "sage.all",
        name: "CyclotomicField",
        source_module: "sage.rings.number_field.number_field",
        source_name: "CyclotomicField",
    },
    SageExportTarget {
        import_module: "sage.rings.number_field.all",
        name: "CyclotomicField",
        source_module: "sage.rings.number_field.number_field",
        source_name: "CyclotomicField",
    },
    SageExportTarget {
        import_module: "sage.all",
        name: "QuadraticField",
        source_module: "sage.rings.number_field.number_field",
        source_name: "QuadraticField",
    },
    SageExportTarget {
        import_module: "sage.rings.number_field.all",
        name: "QuadraticField",
        source_module: "sage.rings.number_field.number_field",
        source_name: "QuadraticField",
    },
    SageExportTarget {
        import_module: "sage.all",
        name: "Graph",
        source_module: "sage.graphs.graph",
        source_name: "Graph",
    },
    SageExportTarget {
        import_module: "sage.graphs.all",
        name: "Graph",
        source_module: "sage.graphs.graph",
        source_name: "Graph",
    },
    SageExportTarget {
        import_module: "sage.all",
        name: "DiGraph",
        source_module: "sage.graphs.digraph",
        source_name: "DiGraph",
    },
    SageExportTarget {
        import_module: "sage.graphs.all",
        name: "DiGraph",
        source_module: "sage.graphs.digraph",
        source_name: "DiGraph",
    },
    SageExportTarget {
        import_module: "sage.all",
        name: "EllipticCurve",
        source_module: "sage.schemes.elliptic_curves.constructor",
        source_name: "EllipticCurve",
    },
    SageExportTarget {
        import_module: "sage.schemes.elliptic_curves.all",
        name: "EllipticCurve",
        source_module: "sage.schemes.elliptic_curves.constructor",
        source_name: "EllipticCurve",
    },
    SageExportTarget {
        import_module: "sage.all",
        name: "matrix",
        source_module: "sage.matrix.constructor",
        source_name: "matrix",
    },
    SageExportTarget {
        import_module: "sage.matrix.all",
        name: "matrix",
        source_module: "sage.matrix.constructor",
        source_name: "matrix",
    },
    SageExportTarget {
        import_module: "sage.all",
        name: "zero_matrix",
        source_module: "sage.matrix.special",
        source_name: "zero_matrix",
    },
    SageExportTarget {
        import_module: "sage.matrix.all",
        name: "zero_matrix",
        source_module: "sage.matrix.special",
        source_name: "zero_matrix",
    },
    SageExportTarget {
        import_module: "sage.all",
        name: "random_matrix",
        source_module: "sage.matrix.special",
        source_name: "random_matrix",
    },
    SageExportTarget {
        import_module: "sage.matrix.all",
        name: "random_matrix",
        source_module: "sage.matrix.special",
        source_name: "random_matrix",
    },
    SageExportTarget {
        import_module: "sage.all",
        name: "identity_matrix",
        source_module: "sage.matrix.special",
        source_name: "identity_matrix",
    },
    SageExportTarget {
        import_module: "sage.matrix.all",
        name: "identity_matrix",
        source_module: "sage.matrix.special",
        source_name: "identity_matrix",
    },
    SageExportTarget {
        import_module: "sage.all",
        name: "column_matrix",
        source_module: "sage.matrix.special",
        source_name: "column_matrix",
    },
    SageExportTarget {
        import_module: "sage.matrix.all",
        name: "column_matrix",
        source_module: "sage.matrix.special",
        source_name: "column_matrix",
    },
    SageExportTarget {
        import_module: "sage.all",
        name: "diagonal_matrix",
        source_module: "sage.matrix.special",
        source_name: "diagonal_matrix",
    },
    SageExportTarget {
        import_module: "sage.matrix.all",
        name: "diagonal_matrix",
        source_module: "sage.matrix.special",
        source_name: "diagonal_matrix",
    },
    SageExportTarget {
        import_module: "sage.all",
        name: "block_matrix",
        source_module: "sage.matrix.special",
        source_name: "block_matrix",
    },
    SageExportTarget {
        import_module: "sage.matrix.all",
        name: "block_matrix",
        source_module: "sage.matrix.special",
        source_name: "block_matrix",
    },
    SageExportTarget {
        import_module: "sage.all",
        name: "block_diagonal_matrix",
        source_module: "sage.matrix.special",
        source_name: "block_diagonal_matrix",
    },
    SageExportTarget {
        import_module: "sage.matrix.all",
        name: "block_diagonal_matrix",
        source_module: "sage.matrix.special",
        source_name: "block_diagonal_matrix",
    },
    SageExportTarget {
        import_module: "sage.all",
        name: "ones_matrix",
        source_module: "sage.matrix.special",
        source_name: "ones_matrix",
    },
    SageExportTarget {
        import_module: "sage.matrix.all",
        name: "ones_matrix",
        source_module: "sage.matrix.special",
        source_name: "ones_matrix",
    },
    SageExportTarget {
        import_module: "sage.all",
        name: "elementary_matrix",
        source_module: "sage.matrix.special",
        source_name: "elementary_matrix",
    },
    SageExportTarget {
        import_module: "sage.matrix.all",
        name: "elementary_matrix",
        source_module: "sage.matrix.special",
        source_name: "elementary_matrix",
    },
    SageExportTarget {
        import_module: "sage.all",
        name: "companion_matrix",
        source_module: "sage.matrix.special",
        source_name: "companion_matrix",
    },
    SageExportTarget {
        import_module: "sage.matrix.all",
        name: "companion_matrix",
        source_module: "sage.matrix.special",
        source_name: "companion_matrix",
    },
    SageExportTarget {
        import_module: "sage.all",
        name: "vector",
        source_module: "sage.modules.free_module_element",
        source_name: "vector",
    },
    SageExportTarget {
        import_module: "sage.modules.all",
        name: "vector",
        source_module: "sage.modules.free_module_element",
        source_name: "vector",
    },
    SageExportTarget {
        import_module: "sage.all",
        name: "zero_vector",
        source_module: "sage.modules.free_module_element",
        source_name: "zero_vector",
    },
    SageExportTarget {
        import_module: "sage.modules.all",
        name: "zero_vector",
        source_module: "sage.modules.free_module_element",
        source_name: "zero_vector",
    },
    SageExportTarget {
        import_module: "sage.all",
        name: "random_vector",
        source_module: "sage.modules.free_module_element",
        source_name: "random_vector",
    },
    SageExportTarget {
        import_module: "sage.modules.all",
        name: "random_vector",
        source_module: "sage.modules.free_module_element",
        source_name: "random_vector",
    },
    SageExportTarget {
        import_module: "sage.all",
        name: "ZZ",
        source_module: "sage.rings.integer_ring",
        source_name: "ZZ",
    },
    SageExportTarget {
        import_module: "sage.all",
        name: "QQ",
        source_module: "sage.rings.rational_field",
        source_name: "QQ",
    },
    SageExportTarget {
        import_module: "sage.all",
        name: "SR",
        source_module: "sage.symbolic.ring",
        source_name: "SR",
    },
    SageExportTarget {
        import_module: "sage.all",
        name: "var",
        source_module: "sage.calculus.var",
        source_name: "var",
    },
    SageExportTarget {
        import_module: "sage.all",
        name: "latex",
        source_module: "sage.misc.latex",
        source_name: "latex",
    },
    SageExportTarget {
        import_module: "sage.all",
        name: "set_random_seed",
        source_module: "sage.misc.randstate",
        source_name: "set_random_seed",
    },
    SageExportTarget {
        import_module: "sage.all",
        name: "RR",
        source_module: "sage.rings.real_mpfr",
        source_name: "RR",
    },
];

const SAGE_OWNER_METHOD_MODULES: &[SageOwnerModuleSpec] = &[
    SageOwnerModuleSpec {
        owner_type: SageOwnerType::Matrix,
        module: "sage.matrix",
        recursive: true,
        priority: 10,
    },
    SageOwnerModuleSpec {
        owner_type: SageOwnerType::FreeModule,
        module: "sage.modules.free_module",
        recursive: false,
        priority: 10,
    },
    SageOwnerModuleSpec {
        owner_type: SageOwnerType::PolynomialRing,
        module: "sage.rings.polynomial.polynomial_ring",
        recursive: false,
        priority: 10,
    },
    SageOwnerModuleSpec {
        owner_type: SageOwnerType::PolynomialRing,
        module: "sage.rings.polynomial.multi_polynomial_libsingular",
        recursive: false,
        priority: 10,
    },
    SageOwnerModuleSpec {
        owner_type: SageOwnerType::PolynomialRing,
        module: "sage.structure.parent_gens",
        recursive: false,
        priority: 10,
    },
    SageOwnerModuleSpec {
        owner_type: SageOwnerType::PolynomialRing,
        module: "sage.structure.parent",
        recursive: false,
        priority: 20,
    },
    SageOwnerModuleSpec {
        owner_type: SageOwnerType::PolynomialRing,
        module: "sage.structure.category_object",
        recursive: false,
        priority: 20,
    },
    SageOwnerModuleSpec {
        owner_type: SageOwnerType::PolynomialElement,
        module: "sage.rings.polynomial.polynomial_element",
        recursive: false,
        priority: 10,
    },
    SageOwnerModuleSpec {
        owner_type: SageOwnerType::PolynomialElement,
        module: "sage.rings.polynomial.multi_polynomial_element",
        recursive: false,
        priority: 10,
    },
    SageOwnerModuleSpec {
        owner_type: SageOwnerType::PolynomialElement,
        module: "sage.rings.polynomial.polynomial_element_generic",
        recursive: false,
        priority: 20,
    },
    SageOwnerModuleSpec {
        owner_type: SageOwnerType::PolynomialElement,
        module: "sage.rings.polynomial.multi_polynomial",
        recursive: false,
        priority: 20,
    },
    SageOwnerModuleSpec {
        owner_type: SageOwnerType::PolynomialElement,
        module: "sage.rings.polynomial.polydict",
        recursive: false,
        priority: 30,
    },
    SageOwnerModuleSpec {
        owner_type: SageOwnerType::PolynomialElement,
        module: "sage.structure.element",
        recursive: false,
        priority: 30,
    },
    SageOwnerModuleSpec {
        owner_type: SageOwnerType::Ideal,
        module: "sage.rings.polynomial.multi_polynomial_ideal",
        recursive: false,
        priority: 10,
    },
    SageOwnerModuleSpec {
        owner_type: SageOwnerType::Field,
        module: "sage.rings.finite_rings",
        recursive: true,
        priority: 10,
    },
    SageOwnerModuleSpec {
        owner_type: SageOwnerType::FieldElement,
        module: "sage.rings.finite_rings",
        recursive: true,
        priority: 10,
    },
    SageOwnerModuleSpec {
        owner_type: SageOwnerType::FieldElement,
        module: "sage.structure.element",
        recursive: false,
        priority: 30,
    },
    SageOwnerModuleSpec {
        owner_type: SageOwnerType::Vector,
        module: "sage.modules.free_module_element",
        recursive: false,
        priority: 10,
    },
    SageOwnerModuleSpec {
        owner_type: SageOwnerType::Vector,
        module: "sage.structure.element",
        recursive: false,
        priority: 30,
    },
    SageOwnerModuleSpec {
        owner_type: SageOwnerType::Graph,
        module: "sage.graphs",
        recursive: true,
        priority: 10,
    },
    SageOwnerModuleSpec {
        owner_type: SageOwnerType::EllipticCurve,
        module: "sage.schemes.elliptic_curves",
        recursive: true,
        priority: 10,
    },
    SageOwnerModuleSpec {
        owner_type: SageOwnerType::NumberField,
        module: "sage.rings.number_field",
        recursive: true,
        priority: 10,
    },
];

const SAGE_METHOD_SPECS: &[SageMethodSpec] = &[
    SageMethodSpec {
        owner_type: SageOwnerType::Matrix,
        member: "rank",
        module: "sage.matrix.matrix0",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::Matrix,
        member: "base_ring",
        module: "sage.matrix.matrix0",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::Matrix,
        member: "dimensions",
        module: "sage.matrix.matrix0",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::Matrix,
        member: "list",
        module: "sage.matrix.matrix0",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::Matrix,
        member: "change_ring",
        module: "sage.matrix.matrix0",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::Matrix,
        member: "pivots",
        module: "sage.matrix.matrix0",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::Matrix,
        member: "rows",
        module: "sage.matrix.matrix1",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::Matrix,
        member: "row",
        module: "sage.matrix.matrix1",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::Matrix,
        member: "column",
        module: "sage.matrix.matrix1",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::Matrix,
        member: "augment",
        module: "sage.matrix.matrix1",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::Matrix,
        member: "matrix_from_columns",
        module: "sage.matrix.matrix1",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::Matrix,
        member: "matrix_from_rows",
        module: "sage.matrix.matrix1",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::Matrix,
        member: "matrix_from_rows_and_columns",
        module: "sage.matrix.matrix1",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::Matrix,
        member: "transpose",
        module: "sage.matrix.matrix_dense",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::Matrix,
        member: "solve_right",
        module: "sage.matrix.matrix2",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::Matrix,
        member: "subs",
        module: "sage.matrix.matrix2",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::Matrix,
        member: "right_kernel",
        module: "sage.matrix.matrix2",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::Matrix,
        member: "det",
        module: "sage.matrix.matrix2",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::Matrix,
        member: "inverse",
        module: "sage.matrix.matrix2",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::Matrix,
        member: "column_space",
        module: "sage.matrix.matrix2",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::Matrix,
        member: "charpoly",
        module: "sage.matrix.matrix2",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::Matrix,
        member: "adjugate",
        module: "sage.matrix.matrix2",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::Matrix,
        member: "nrows",
        module: "sage.matrix.matrix0",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::Matrix,
        member: "ncols",
        module: "sage.matrix.matrix0",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::FreeModule,
        member: "basis",
        module: "sage.modules.free_module",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::FreeModule,
        member: "basis_matrix",
        module: "sage.modules.free_module",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::FreeModule,
        member: "dimension",
        module: "sage.modules.free_module",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::FreeModule,
        member: "change_ring",
        module: "sage.modules.free_module",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::PolynomialRing,
        member: "gens",
        module: "sage.structure.parent_gens",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::PolynomialRing,
        member: "gen",
        module: "sage.structure.parent_gens",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::PolynomialRing,
        member: "ideal",
        module: "sage.rings.polynomial.multi_polynomial_libsingular",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::PolynomialRing,
        member: "base_ring",
        module: "sage.structure.category_object",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::PolynomialRing,
        member: "hom",
        module: "sage.structure.parent",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::PolynomialRing,
        member: "lagrange_polynomial",
        module: "sage.rings.polynomial.polynomial_ring",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::PolynomialElement,
        member: "degree",
        module: "sage.rings.polynomial.multi_polynomial_element",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::PolynomialElement,
        member: "factor",
        module: "sage.rings.polynomial.polynomial_element",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::PolynomialElement,
        member: "monic",
        module: "sage.rings.polynomial.polynomial_element",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::PolynomialElement,
        member: "map_coefficients",
        module: "sage.rings.polynomial.polynomial_element",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::PolynomialElement,
        member: "monomial_coefficient",
        module: "sage.rings.polynomial.multi_polynomial_element",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::PolynomialElement,
        member: "constant_coefficient",
        module: "sage.rings.polynomial.multi_polynomial_element",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::PolynomialElement,
        member: "base_ring",
        module: "sage.rings.polynomial.polynomial_element",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::PolynomialElement,
        member: "change_ring",
        module: "sage.rings.polynomial.polynomial_element",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::PolynomialElement,
        member: "list",
        module: "sage.rings.polynomial.polynomial_element_generic",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::PolynomialElement,
        member: "is_zero",
        module: "sage.structure.element",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::PolynomialElement,
        member: "parent",
        module: "sage.structure.element",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::PolynomialElement,
        member: "dict",
        module: "sage.rings.polynomial.polydict",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::PolynomialElement,
        member: "subs",
        module: "sage.rings.polynomial.multi_polynomial_element",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::PolynomialElement,
        member: "total_degree",
        module: "sage.rings.polynomial.multi_polynomial_element",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::PolynomialElement,
        member: "is_constant",
        module: "sage.rings.polynomial.multi_polynomial_element",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::PolynomialElement,
        member: "gcd",
        module: "sage.rings.polynomial.multi_polynomial",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::PolynomialElement,
        member: "resultant",
        module: "sage.rings.polynomial.multi_polynomial_element",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::PolynomialElement,
        member: "derivative",
        module: "sage.rings.polynomial.multi_polynomial",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::PolynomialElement,
        member: "roots",
        module: "sage.rings.polynomial.polynomial_element",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::Ideal,
        member: "dimension",
        module: "sage.rings.polynomial.multi_polynomial_ideal",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::Ideal,
        member: "variety",
        module: "sage.rings.polynomial.multi_polynomial_ideal",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::Field,
        member: "random_element",
        module: "sage.rings.finite_rings.finite_field_base",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::Field,
        member: "order",
        module: "sage.rings.finite_rings.finite_field_base",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::Field,
        member: "from_integer",
        module: "sage.rings.finite_rings.finite_field_base",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::FieldElement,
        member: "parent",
        module: "sage.structure.element",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::FieldElement,
        member: "polynomial",
        module: "sage.rings.finite_rings.element_givaro",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::FieldElement,
        member: "to_integer",
        module: "sage.rings.finite_rings.element_base",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::Vector,
        member: "base_ring",
        module: "sage.structure.element",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::Vector,
        member: "change_ring",
        module: "sage.modules.free_module_element",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::Vector,
        member: "list",
        module: "sage.modules.free_module_element",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::Vector,
        member: "row",
        module: "sage.modules.free_module_element",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::Vector,
        member: "column",
        module: "sage.modules.free_module_element",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::Graph,
        member: "vertices",
        module: "sage.graphs.generic_graph",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::Graph,
        member: "edges",
        module: "sage.graphs.generic_graph",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::Graph,
        member: "neighbors",
        module: "sage.graphs.generic_graph",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::Graph,
        member: "degree",
        module: "sage.graphs.generic_graph",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::Graph,
        member: "shortest_path",
        module: "sage.graphs.generic_graph",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::Graph,
        member: "adjacency_matrix",
        module: "sage.graphs.generic_graph",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::Graph,
        member: "plot",
        module: "sage.graphs.generic_graph",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::Graph,
        member: "is_connected",
        module: "sage.graphs.base.c_graph",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::EllipticCurve,
        member: "base_ring",
        module: "sage.schemes.elliptic_curves.ell_generic",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::EllipticCurve,
        member: "gens",
        module: "sage.schemes.elliptic_curves.ell_generic",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::EllipticCurve,
        member: "plot",
        module: "sage.schemes.elliptic_curves.ell_generic",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::EllipticCurve,
        member: "points",
        module: "sage.schemes.elliptic_curves.ell_finite_field",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::EllipticCurve,
        member: "cardinality",
        module: "sage.schemes.elliptic_curves.ell_finite_field",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::EllipticCurve,
        member: "torsion_subgroup",
        module: "sage.schemes.elliptic_curves.ell_finite_field",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::EllipticCurve,
        member: "rank",
        module: "sage.schemes.elliptic_curves.ell_rational_field",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::EllipticCurve,
        member: "integral_points",
        module: "sage.schemes.elliptic_curves.ell_rational_field",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::NumberField,
        member: "ring_of_integers",
        module: "sage.rings.number_field.number_field_base",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::NumberField,
        member: "degree",
        module: "sage.rings.number_field.number_field",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::NumberField,
        member: "absolute_degree",
        module: "sage.rings.number_field.number_field",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::NumberField,
        member: "relative_degree",
        module: "sage.rings.number_field.number_field",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::NumberField,
        member: "discriminant",
        module: "sage.rings.number_field.number_field",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::NumberField,
        member: "gen",
        module: "sage.rings.number_field.number_field",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::NumberField,
        member: "gens",
        module: "sage.rings.number_field.number_field_rel",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::NumberField,
        member: "embeddings",
        module: "sage.rings.number_field.number_field",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::NumberField,
        member: "places",
        module: "sage.rings.number_field.number_field",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::NumberField,
        member: "signature",
        module: "sage.rings.number_field.number_field",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::NumberField,
        member: "class_group",
        module: "sage.rings.number_field.number_field",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::NumberField,
        member: "unit_group",
        module: "sage.rings.number_field.number_field",
    },
    SageMethodSpec {
        owner_type: SageOwnerType::NumberField,
        member: "is_isomorphic",
        module: "sage.rings.number_field.number_field",
    },
];

const SAGE_METHOD_ALIAS_SPECS: &[SageMethodAliasSpec] = &[
    SageMethodAliasSpec {
        owner_type: SageOwnerType::MatrixConstructor,
        member: "random",
        source_name: "random_matrix",
        module: "sage.matrix.special",
    },
    SageMethodAliasSpec {
        owner_type: SageOwnerType::MatrixConstructor,
        member: "identity",
        source_name: "identity_matrix",
        module: "sage.matrix.special",
    },
    SageMethodAliasSpec {
        owner_type: SageOwnerType::MatrixConstructor,
        member: "column",
        source_name: "column_matrix",
        module: "sage.matrix.special",
    },
    SageMethodAliasSpec {
        owner_type: SageOwnerType::MatrixConstructor,
        member: "diagonal",
        source_name: "diagonal_matrix",
        module: "sage.matrix.special",
    },
    SageMethodAliasSpec {
        owner_type: SageOwnerType::MatrixConstructor,
        member: "zero",
        source_name: "zero_matrix",
        module: "sage.matrix.special",
    },
    SageMethodAliasSpec {
        owner_type: SageOwnerType::MatrixConstructor,
        member: "ones",
        source_name: "ones_matrix",
        module: "sage.matrix.special",
    },
    SageMethodAliasSpec {
        owner_type: SageOwnerType::MatrixConstructor,
        member: "block",
        source_name: "block_matrix",
        module: "sage.matrix.special",
    },
    SageMethodAliasSpec {
        owner_type: SageOwnerType::MatrixConstructor,
        member: "block_diagonal",
        source_name: "block_diagonal_matrix",
        module: "sage.matrix.special",
    },
    SageMethodAliasSpec {
        owner_type: SageOwnerType::EllipticCurve,
        member: "order",
        source_name: "cardinality",
        module: "sage.schemes.elliptic_curves.ell_finite_field",
    },
    SageMethodAliasSpec {
        owner_type: SageOwnerType::FieldElement,
        member: "integer_representation",
        source_name: "to_integer",
        module: "sage.rings.finite_rings.element_base",
    },
    SageMethodAliasSpec {
        owner_type: SageOwnerType::FieldElement,
        member: "_integer_representation",
        source_name: "_integer_representation",
        module: "sage.rings.finite_rings.element_givaro",
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH as STD_UNIX_EPOCH};

    fn test_root(name: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(STD_UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("sage-index-{name}-{stamp}"));
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn first_position(source: &str, needle: &str) -> (u32, u32) {
        for (line_index, line) in source.lines().enumerate() {
            if let Some(character) = line.find(needle) {
                return (line_index as u32, character as u32);
            }
        }
        panic!("missing {needle:?} in source");
    }

    fn position_in_line(source: &str, line_needle: &str, needle: &str) -> (u32, u32) {
        for (line_index, line) in source.lines().enumerate() {
            if line.contains(line_needle) {
                if let Some(character) = line.find(needle) {
                    return (line_index as u32, character as u32);
                }
                panic!("missing {needle:?} in line containing {line_needle:?}");
            }
        }
        panic!("missing line containing {line_needle:?}");
    }

    fn member_position(source: &str, member: &str) -> (u32, u32) {
        let dotted = format!(".{member}");
        let (line, character) = first_position(source, &dotted);
        (line, character + 1)
    }

    fn nth_member_position(source: &str, member: &str, occurrence: usize) -> (u32, u32) {
        let dotted = format!(".{member}");
        let mut seen = 0usize;
        for (line_index, line) in source.lines().enumerate() {
            let mut offset = 0usize;
            while let Some(character) = line[offset..].find(&dotted) {
                let start = offset + character;
                if seen == occurrence {
                    return (line_index as u32, (start + 1) as u32);
                }
                seen = seen.saturating_add(1);
                offset = start + dotted.len();
            }
        }
        panic!("missing occurrence {occurrence} of {dotted:?} in source");
    }

    fn sqlite_index_exists(connection: &Connection, table: &str, index_name: &str) -> bool {
        let mut statement = connection
            .prepare(&format!("pragma index_list({table})"))
            .unwrap();
        let rows = statement
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap();
        for name in rows.flatten() {
            if name == index_name {
                return true;
            }
        }
        false
    }

    #[test]
    fn preprocess_rewrites_caret_outside_strings_and_comments() {
        let result = preprocess_sage_source("x = y^2\ns = '^'\n# z^2\n");
        assert_eq!(result.generated, "x = y**2\ns = '^'\n# z^2\n");
        assert_eq!(result.edits.len(), 1);
    }

    #[test]
    fn preprocess_rewrites_sage_ranges_outside_strings_and_comments() {
        let result =
            preprocess_sage_source("xs = [1..5]\nys = [1 .. width]\ns = '1..5'\n# [1..5]\n");
        assert_eq!(
            result.generated,
            "xs = [1,5]\nys = [1 , width]\ns = '1..5'\n# [1..5]\n"
        );
        assert_eq!(
            result
                .edits
                .iter()
                .filter(|edit| edit.source_text == ".." && edit.generated_text == ",")
                .count(),
            2
        );
    }

    #[test]
    fn preprocess_rewrites_empty_sage_index_after_ring_owner() {
        let result = preprocess_sage_source("S = Kfun[]\nempty = []\ntext = 'Kfun[]'\n");
        assert_eq!(
            result.generated,
            "S = Kfun[0]\nempty = []\ntext = 'Kfun[]'\n"
        );
        assert_eq!(
            result
                .edits
                .iter()
                .filter(|edit| edit.source_text == "[]" && edit.generated_text == "[0]")
                .count(),
            1
        );
    }

    #[test]
    fn parser_extracts_python_and_preparser_symbols() {
        let file = parse_source(
            "demo",
            Path::new("demo.sage"),
            "R.<x, y> = PolynomialRing(QQ, 2)\nPublicFactory = object()\nclass Solver:\n    pass\n\ndef helper():\n    \"\"\"Return help.\"\"\"\n    return x\n",
        );
        let names: Vec<_> = file
            .symbols
            .iter()
            .map(|symbol| symbol.name.as_str())
            .collect();
        assert!(names.contains(&"R"));
        assert!(names.contains(&"x"));
        assert!(names.contains(&"PublicFactory"));
        assert!(names.contains(&"Solver"));
        assert!(names.contains(&"helper"));
        assert_eq!(
            file.symbols
                .iter()
                .find(|symbol| symbol.name == "helper")
                .and_then(|symbol| symbol.docstring.as_deref()),
            Some("Return help.")
        );
    }

    #[test]
    fn parser_extracts_lazy_import_lists_and_aliases() {
        let source = r#"
from sage.misc.lazy_import import lazy_import
from sage.misc.lazy_import import LazyImport

lazy_import("sage.future.module", ["FutureFactory", "FutureThing"])
lazy_import(
    'sage.future.aliases',
    ['FutureAliasSource', 'SecondAliasSource'],
    as_=['FutureAlias', 'SecondAlias'],
)
lazy_import('sage.future.scalar', 'ScalarSource', as_='ScalarAlias')
SymbolicRing = LazyImport('sage.symbolic.ring', 'SymbolicRing')
FiniteGroups = LazyImport(
    'sage.categories.finite_groups',
    'FiniteGroups',
    at_startup=True,
)
"#;
        let file = parse_source("sage.future.all", Path::new("sage/future/all.py"), source);
        let imports: BTreeMap<_, _> = file
            .symbols
            .iter()
            .filter(|symbol| symbol.kind == SymbolKind::Import)
            .map(|symbol| {
                (
                    symbol.name.as_str(),
                    symbol.import_from.as_deref().unwrap_or_default(),
                )
            })
            .collect();
        assert_eq!(
            imports.get("FutureFactory").copied(),
            Some("sage.future.module::FutureFactory")
        );
        assert_eq!(
            imports.get("FutureThing").copied(),
            Some("sage.future.module::FutureThing")
        );
        assert_eq!(
            imports.get("FutureAlias").copied(),
            Some("sage.future.aliases::FutureAliasSource")
        );
        assert_eq!(
            imports.get("SecondAlias").copied(),
            Some("sage.future.aliases::SecondAliasSource")
        );
        assert_eq!(
            imports.get("ScalarAlias").copied(),
            Some("sage.future.scalar::ScalarSource")
        );
        assert_eq!(
            imports.get("SymbolicRing").copied(),
            Some("sage.symbolic.ring::SymbolicRing")
        );
        assert_eq!(
            imports.get("FiniteGroups").copied(),
            Some("sage.categories.finite_groups::FiniteGroups")
        );
    }

    #[test]
    fn parser_extracts_deprecated_function_aliases() {
        let source = r#"
from sage.misc.superseded import deprecated_function_alias
from sage.future.module import replacement
old_replacement = deprecated_function_alias(12345, replacement)

def local_replacement():
    pass

class Wrapper:
    old_local = deprecated_function_alias(23456, local_replacement)
"#;
        let file = parse_source("sage.future.all", Path::new("sage/future/all.py"), source);
        let imports: BTreeMap<_, _> = file
            .symbols
            .iter()
            .filter(|symbol| symbol.kind == SymbolKind::Import)
            .map(|symbol| {
                (
                    symbol.name.as_str(),
                    symbol.import_from.as_deref().unwrap_or_default(),
                )
            })
            .collect();

        assert_eq!(
            imports.get("old_replacement").copied(),
            Some("sage.future.module::replacement")
        );
        assert_eq!(
            imports.get("old_local").copied(),
            Some("sage.future.all::local_replacement")
        );
    }

    #[test]
    fn parser_extracts_top_level_import_member_aliases() {
        let source = r#"
import sage.future.module as future_module
from sage.categories import finite_weyl_groups

class LocalFactory:
    pass

FutureAlias = future_module.FutureFactory
Example = finite_weyl_groups.Example
LocalAlias = LocalFactory

def local():
    Hidden = future_module.HiddenFactory
    LocalHidden = LocalFactory
"#;
        let file = parse_source("sage.future.all", Path::new("sage/future/all.py"), source);
        let imports: BTreeMap<_, _> = file
            .symbols
            .iter()
            .filter(|symbol| symbol.kind == SymbolKind::Import)
            .map(|symbol| {
                (
                    symbol.name.as_str(),
                    symbol.import_from.as_deref().unwrap_or_default(),
                )
            })
            .collect();

        assert_eq!(
            imports.get("FutureAlias").copied(),
            Some("sage.future.module::FutureFactory")
        );
        assert_eq!(
            imports.get("Example").copied(),
            Some("sage.categories.finite_weyl_groups::Example")
        );
        assert_eq!(
            imports.get("LocalAlias").copied(),
            Some("sage.future.all::LocalFactory")
        );
        assert!(!imports.contains_key("Hidden"));
        assert!(!imports.contains_key("LocalHidden"));
    }

    #[test]
    fn parser_extracts_class_method_aliases_without_local_assignments() {
        let source = r#"
class MatrixFuture:
    def trace_impl(self):
        """Return a source-derived trace."""
        return 0

    trace_alias = trace_impl
    Element = MatrixFutureElement

    def helper(self):
        hidden_alias = trace_impl
        return hidden_alias()

class MatrixFutureElement:
    pass
"#;
        let file = parse_source(
            "sage.matrix.future",
            Path::new("sage/matrix/future.py"),
            source,
        );
        let imports: BTreeMap<_, _> = file
            .symbols
            .iter()
            .filter(|symbol| symbol.kind == SymbolKind::Import)
            .map(|symbol| {
                (
                    symbol.name.as_str(),
                    (
                        symbol.import_from.as_deref().unwrap_or_default(),
                        symbol.detail.as_str(),
                    ),
                )
            })
            .collect();

        assert_eq!(
            imports.get("trace_alias").copied(),
            Some((
                "sage.matrix.future::trace_impl",
                "MethodAlias MatrixFuture.trace_alias for trace_impl"
            ))
        );
        assert!(!imports.contains_key("hidden_alias"));
        assert!(!imports.contains_key("Element"));
    }

    #[test]
    fn parser_extracts_matrix_constructor_method_aliases_from_sage_decorators() {
        let source = r#"
from sage.matrix.constructor import matrix

def matrix_method(func=None, name=None):
    return func

@matrix_method
def random_matrix(ring, nrows):
    return matrix([])

@matrix_method(name='unit')
def identity_matrix(ring, n):
    return matrix([])
"#;
        let file = parse_source(
            "sage.matrix.special",
            Path::new("sage/matrix/special.py"),
            source,
        );
        let imports: BTreeMap<_, _> = file
            .symbols
            .iter()
            .filter(|symbol| symbol.kind == SymbolKind::Import)
            .map(|symbol| {
                (
                    symbol.name.as_str(),
                    (
                        symbol.import_from.as_deref().unwrap_or_default(),
                        symbol.detail.as_str(),
                    ),
                )
            })
            .collect();

        assert_eq!(
            imports.get("random").copied(),
            Some((
                "sage.matrix.special::random_matrix",
                "MatrixConstructorMethodAlias matrix.random for random_matrix"
            ))
        );
        assert_eq!(
            imports.get("unit").copied(),
            Some((
                "sage.matrix.special::identity_matrix",
                "MatrixConstructorMethodAlias matrix.unit for identity_matrix"
            ))
        );
    }

    #[test]
    fn parser_extracts_raw_sage_docstrings() {
        let file = parse_source(
            "demo",
            Path::new("demo.py"),
            "r\"\"\"Module docs.\"\"\"\n\ndef helper():\n    r\"\"\"\n    Return raw docs.\n    \"\"\"\n    return 1\n",
        );

        assert_eq!(file.module_docstring.as_deref(), Some("Module docs."));
        assert_eq!(
            file.symbols
                .iter()
                .find(|symbol| symbol.name == "helper")
                .and_then(|symbol| symbol.docstring.as_deref()),
            Some("Return raw docs.")
        );
    }

    #[test]
    fn preprocess_maps_preparser_assignment() {
        let result = preprocess_sage_source(
            "R.<x, y> = PolynomialRing(QQ, 2)\nK.<i> = NumberField(x^2 + 1)\nS.<Y> = Kfun[]\nxs = [1..5]\nz = x^2\n",
        );
        assert!(result.generated.contains("R = PolynomialRing(QQ, 2)"));
        assert!(result.generated.contains("x = R.gen(0)"));
        assert!(result.generated.contains("K = NumberField(x**2 + 1)"));
        assert!(result.generated.contains("S = Kfun[0]"));
        assert!(result.generated.contains("xs = [1,5]"));
        assert!(result.generated.contains("z = x**2"));
        assert!(result
            .edits
            .iter()
            .any(|edit| edit.generated_text == "preparser-assignment"));
    }

    #[test]
    fn parser_ignores_docstring_examples() {
        let file = parse_source(
            "demo",
            Path::new("demo.py"),
            "\"\"\"\ndef not_real():\n    pass\nR.<a> = PolynomialRing(QQ)\n\"\"\"\ndef real():\n    pass\n",
        );
        let names: Vec<_> = file
            .symbols
            .iter()
            .map(|symbol| symbol.name.as_str())
            .collect();
        assert!(!names.contains(&"not_real"));
        assert!(!names.contains(&"R"));
        assert!(!names.contains(&"a"));
        assert!(names.contains(&"real"));
    }

    #[test]
    fn symbol_lookup_prefers_definitions_over_imports() {
        let path = PathBuf::from("demo.py");
        let import = SymbolRecord {
            name: "target".to_string(),
            kind: SymbolKind::Import,
            module: "consumer".to_string(),
            path: path.clone(),
            range: SourceRange {
                start_line: 0,
                start_character: 0,
                end_line: 0,
                end_character: 6,
            },
            detail: "Import target".to_string(),
            docstring: None,
            import_from: Some("provider".to_string()),
            signature: None,
        };
        let definition = SymbolRecord {
            name: "target".to_string(),
            kind: SymbolKind::Function,
            module: "provider".to_string(),
            path,
            range: SourceRange {
                start_line: 3,
                start_character: 4,
                end_line: 3,
                end_character: 10,
            },
            detail: "Function target".to_string(),
            docstring: Some("Definition docs.".to_string()),
            import_from: None,
            signature: Some("target()".to_string()),
        };
        let mut index = WorkspaceIndex::default();
        index
            .symbols_by_name
            .insert("target".to_string(), vec![import, definition]);

        let symbol = index.symbol("target").expect("symbol should resolve");
        assert_eq!(symbol.kind, SymbolKind::Function);
        assert_eq!(symbol.module, "provider");
    }

    #[test]
    fn import_resolution_prefers_import_source_module() {
        let root = test_root("import-resolution");
        let consumer = root.join("consumer.sage");
        let provider = root.join("provider.py");
        fs::write(&consumer, "from provider import target\nvalue = target()\n").unwrap();
        fs::write(
            &provider,
            "def target():\n    \"\"\"Provider docs.\"\"\"\n    return 1\n",
        )
        .unwrap();
        let mut index = WorkspaceIndex::new(IndexOptions {
            roots: vec![root.clone()],
            editable_roots: Vec::new(),
            exclude_globs: Vec::new(),
            cache_dir: root.join(".cache"),
            enable_pyx: true,
        });
        index.rebuild().unwrap();
        let resolved = index
            .resolve_symbol("target", Some("consumer"))
            .expect("target should resolve");
        assert_eq!(resolved.module, "provider");
        assert_eq!(resolved.docstring.as_deref(), Some("Provider docs."));
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn query_resolves_explicit_import_target_before_full_index() {
        let root = test_root("cold-explicit-import-target");
        fs::create_dir_all(root.join("sage/future")).unwrap();
        let consumer = root.join("consumer.py");
        let source = "from sage.future.module import FutureFactory as Future\nvalue = Future()\n";
        fs::write(&consumer, source).unwrap();
        fs::write(
            root.join("sage/future/module.py"),
            "def FutureFactory():\n    \"\"\"Build a future factory.\"\"\"\n    return None\n",
        )
        .unwrap();
        let index = WorkspaceIndex::new(IndexOptions {
            roots: vec![root.clone()],
            editable_roots: Vec::new(),
            exclude_globs: Vec::new(),
            cache_dir: root.join(".cache"),
            enable_pyx: true,
        });

        let query = index.query_source_symbol(&consumer, source, "Future", None, None, Vec::new());
        assert_eq!(
            query
                .definition
                .as_ref()
                .map(|definition| definition.path.as_path()),
            Some(normalize_path(root.join("sage/future/module.py")).as_path())
        );
        assert_eq!(
            query
                .definition
                .as_ref()
                .map(|definition| definition.name.as_str()),
            Some("FutureFactory")
        );
        assert_eq!(
            query
                .documentation
                .as_ref()
                .map(|documentation| documentation.summary.as_str()),
            Some("Build a future factory.")
        );
        assert_eq!(
            query.resolution_reason.as_deref(),
            Some("resolved `Future` from explicit import target sage.future.module")
        );
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn query_resolves_relative_import_target_before_full_index() {
        let root = test_root("cold-relative-import-target");
        let package = root.join("sage/future");
        fs::create_dir_all(&package).unwrap();
        let consumer = package.join("consumer.py");
        let source = "from .module import Feature\nvalue = Feature()\n";
        fs::write(&consumer, source).unwrap();
        fs::write(
            package.join("module.py"),
            "def Feature():\n    \"\"\"Build a relative import target.\"\"\"\n    return None\n",
        )
        .unwrap();
        let index = WorkspaceIndex::new(IndexOptions {
            roots: vec![root.clone()],
            editable_roots: Vec::new(),
            exclude_globs: Vec::new(),
            cache_dir: root.join(".cache"),
            enable_pyx: true,
        });

        let query = index.query_source_symbol(&consumer, source, "Feature", None, None, Vec::new());
        assert_eq!(
            query
                .definition
                .as_ref()
                .map(|definition| definition.path.as_path()),
            Some(normalize_path(package.join("module.py")).as_path())
        );
        assert_eq!(
            query
                .documentation
                .as_ref()
                .map(|documentation| documentation.summary.as_str()),
            Some("Build a relative import target.")
        );
        assert_eq!(
            query.resolution_reason.as_deref(),
            Some("resolved `Feature` from explicit import target sage.future.module")
        );
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn query_resolves_sage_all_fallback_target_before_full_index() {
        let root = test_root("cold-sage-all-fallback-target");
        fs::create_dir_all(root.join("sage/rings")).unwrap();
        let consumer = root.join("consumer.py");
        let source = "from sage.all import QQ\nbase = QQ\n";
        fs::write(&consumer, source).unwrap();
        fs::write(
            root.join("sage/rings/rational_field.py"),
            "def RationalField():\n    return object()\n\nQQ = RationalField()\n",
        )
        .unwrap();
        let index = WorkspaceIndex::new(IndexOptions {
            roots: vec![root.clone()],
            editable_roots: Vec::new(),
            exclude_globs: Vec::new(),
            cache_dir: root.join(".cache"),
            enable_pyx: true,
        });

        let query = index.query_source_symbol(&consumer, source, "QQ", None, None, Vec::new());
        assert_eq!(
            query
                .definition
                .as_ref()
                .map(|definition| definition.path.as_path()),
            Some(normalize_path(root.join("sage/rings/rational_field.py")).as_path())
        );
        assert_eq!(
            query
                .definition
                .as_ref()
                .map(|definition| definition.name.as_str()),
            Some("QQ")
        );
        assert_eq!(query.resolution_confidence.as_deref(), Some("high"));
        assert!(
            query
                .resolution_reason
                .as_deref()
                .is_some_and(|reason| reason.contains("built-in sage.all export fallback")),
            "{query:?}"
        );
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn query_prefers_source_sage_all_exports_over_static_fallback_before_full_index() {
        let root = test_root("cold-sage-all-source-before-fallback");
        fs::create_dir_all(root.join("sage/future")).unwrap();
        fs::create_dir_all(root.join("sage/rings/finite_rings")).unwrap();
        fs::write(
            root.join("sage/all.py"),
            "from sage.future.factory import GF\n",
        )
        .unwrap();
        fs::write(
            root.join("sage/future/factory.py"),
            "def GF(*args):\n    \"\"\"Construct the moved finite field factory.\"\"\"\n    return args\n",
        )
        .unwrap();
        fs::write(
            root.join("sage/rings/finite_rings/finite_field_constructor.py"),
            "def GF(*args):\n    \"\"\"Outdated static fallback target.\"\"\"\n    return args\n",
        )
        .unwrap();
        let consumer = root.join("consumer.py");
        let source = "from sage.all import GF\nfield = GF(7)\n";
        fs::write(&consumer, source).unwrap();
        let index = WorkspaceIndex::new(IndexOptions {
            roots: vec![root.clone()],
            editable_roots: Vec::new(),
            exclude_globs: Vec::new(),
            cache_dir: root.join(".cache"),
            enable_pyx: true,
        });

        let query = index.query_source_symbol(&consumer, source, "GF", None, None, Vec::new());

        assert_eq!(
            query
                .definition
                .as_ref()
                .map(|definition| definition.path.as_path()),
            Some(normalize_path(root.join("sage/future/factory.py")).as_path())
        );
        assert_eq!(
            query
                .documentation
                .as_ref()
                .map(|documentation| documentation.summary.as_str()),
            Some("Construct the moved finite field factory.")
        );
        assert_eq!(query.resolution_confidence.as_deref(), Some("high"));
        assert!(
            query
                .resolution_reason
                .as_deref()
                .is_some_and(|reason| reason.contains("source-derived sage.all export chain")),
            "{query:?}"
        );
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn hydrate_prefers_source_sage_all_exports_over_static_fallback_cache_rows() {
        let root = test_root("hydrate-sage-all-source-before-fallback");
        fs::create_dir_all(root.join("sage/future")).unwrap();
        fs::create_dir_all(root.join("sage/rings/finite_rings")).unwrap();
        fs::write(
            root.join("sage/all.py"),
            "from sage.future.factory import GF\n",
        )
        .unwrap();
        fs::write(
            root.join("sage/future/factory.py"),
            "def GF(*args):\n    \"\"\"Construct the moved finite field factory.\"\"\"\n    return args\n",
        )
        .unwrap();
        fs::write(
            root.join("sage/rings/finite_rings/finite_field_constructor.py"),
            "def GF(*args):\n    \"\"\"Outdated static fallback target.\"\"\"\n    return args\n",
        )
        .unwrap();
        let consumer = root.join("consumer.py");
        let source = "from sage.all import GF\nfield = GF(7)\n";
        fs::write(&consumer, source).unwrap();
        let options = IndexOptions {
            roots: vec![root.clone()],
            editable_roots: Vec::new(),
            exclude_globs: Vec::new(),
            cache_dir: root.join(".cache"),
            enable_pyx: true,
        };
        let mut index = WorkspaceIndex::new(options.clone());
        index.rebuild().unwrap();

        let mut hydrated = WorkspaceIndex::new(options);
        hydrated.hydrate_from_cache().unwrap();
        let query = hydrated.query_source_symbol(&consumer, source, "GF", None, None, Vec::new());

        assert_eq!(
            query
                .definition
                .as_ref()
                .map(|definition| definition.path.as_path()),
            Some(normalize_path(root.join("sage/future/factory.py")).as_path())
        );
        assert_eq!(
            query
                .documentation
                .as_ref()
                .map(|documentation| documentation.summary.as_str()),
            Some("Construct the moved finite field factory.")
        );
        assert_eq!(query.resolution_confidence.as_deref(), Some("high"));
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn hydrate_prefers_source_sage_all_star_reexports_over_static_fallback_cache_rows() {
        let root = test_root("hydrate-sage-all-star-before-fallback");
        fs::create_dir_all(root.join("sage/future")).unwrap();
        fs::create_dir_all(root.join("sage/rings/finite_rings")).unwrap();
        fs::write(root.join("sage/all.py"), "from sage.future.all import *\n").unwrap();
        fs::write(
            root.join("sage/future/all.py"),
            "__all__ = [\"GF\"]\nfrom sage.future.factory import GF\n",
        )
        .unwrap();
        fs::write(
            root.join("sage/future/factory.py"),
            "def GF(*args):\n    \"\"\"Construct the star-exported finite field factory.\"\"\"\n    return args\n",
        )
        .unwrap();
        fs::write(
            root.join("sage/rings/finite_rings/finite_field_constructor.py"),
            "def GF(*args):\n    \"\"\"Outdated static fallback target.\"\"\"\n    return args\n",
        )
        .unwrap();
        let consumer = root.join("consumer.py");
        let source = "from sage.all import GF\nfield = GF(7)\n";
        fs::write(&consumer, source).unwrap();
        let options = IndexOptions {
            roots: vec![root.clone()],
            editable_roots: Vec::new(),
            exclude_globs: Vec::new(),
            cache_dir: root.join(".cache"),
            enable_pyx: true,
        };
        let mut index = WorkspaceIndex::new(options.clone());
        index.rebuild().unwrap();

        let mut hydrated = WorkspaceIndex::new(options);
        hydrated.hydrate_from_cache().unwrap();
        let query = hydrated.query_source_symbol(&consumer, source, "GF", None, None, Vec::new());

        assert_eq!(
            query
                .definition
                .as_ref()
                .map(|definition| definition.path.as_path()),
            Some(normalize_path(root.join("sage/future/factory.py")).as_path())
        );
        assert_eq!(
            query
                .documentation
                .as_ref()
                .map(|documentation| documentation.summary.as_str()),
            Some("Construct the star-exported finite field factory.")
        );
        assert_eq!(query.resolution_confidence.as_deref(), Some("high"));
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn query_resolves_sage_all_source_import_before_full_index_without_fallback_map() {
        let root = test_root("cold-sage-all-source-import");
        fs::create_dir_all(root.join("sage/future")).unwrap();
        fs::write(
            root.join("sage/all.py"),
            "from sage.future.factory import NewConstructor\n",
        )
        .unwrap();
        fs::write(
            root.join("sage/future/factory.py"),
            "def NewConstructor():\n    \"\"\"Construct a source-derived object.\"\"\"\n    return None\n",
        )
        .unwrap();
        let consumer = root.join("consumer.py");
        let source = "from sage.all import NewConstructor\nvalue = NewConstructor()\n";
        fs::write(&consumer, source).unwrap();
        let index = WorkspaceIndex::new(IndexOptions {
            roots: vec![root.clone()],
            editable_roots: Vec::new(),
            exclude_globs: Vec::new(),
            cache_dir: root.join(".cache"),
            enable_pyx: true,
        });

        let query =
            index.query_source_symbol(&consumer, source, "NewConstructor", None, None, Vec::new());
        assert_eq!(
            query
                .definition
                .as_ref()
                .map(|definition| definition.path.as_path()),
            Some(normalize_path(root.join("sage/future/factory.py")).as_path())
        );
        assert_eq!(
            query
                .documentation
                .as_ref()
                .map(|documentation| documentation.summary.as_str()),
            Some("Construct a source-derived object.")
        );
        assert!(
            query
                .resolution_reason
                .as_deref()
                .is_some_and(|reason| reason.contains("source-derived sage.all export chain")),
            "{query:?}"
        );
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn query_resolves_sage_all_star_reexport_before_full_index_without_fallback_map() {
        let root = test_root("cold-sage-all-star-reexport");
        fs::create_dir_all(root.join("sage/future")).unwrap();
        fs::write(root.join("sage/all.py"), "from sage.future.all import *\n").unwrap();
        fs::write(
            root.join("sage/future/all.py"),
            "__all__ = [\"NewFactory\"]\nfrom sage.future.factory import NewFactory\n",
        )
        .unwrap();
        fs::write(
            root.join("sage/future/factory.py"),
            "def NewFactory():\n    \"\"\"Build a star re-exported object.\"\"\"\n    return None\n",
        )
        .unwrap();
        let consumer = root.join("consumer.py");
        let source = "from sage.all import NewFactory\nvalue = NewFactory()\n";
        fs::write(&consumer, source).unwrap();
        let index = WorkspaceIndex::new(IndexOptions {
            roots: vec![root.clone()],
            editable_roots: Vec::new(),
            exclude_globs: Vec::new(),
            cache_dir: root.join(".cache"),
            enable_pyx: true,
        });

        let query =
            index.query_source_symbol(&consumer, source, "NewFactory", None, None, Vec::new());
        assert_eq!(
            query
                .definition
                .as_ref()
                .map(|definition| definition.path.as_path()),
            Some(normalize_path(root.join("sage/future/factory.py")).as_path())
        );
        assert_eq!(
            query
                .documentation
                .as_ref()
                .map(|documentation| documentation.summary.as_str()),
            Some("Build a star re-exported object.")
        );
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn query_resolves_implicit_sage_all_name_in_sage_file_before_global_lookup() {
        let root = test_root("implicit-sage-all-name");
        fs::create_dir_all(root.join("sage/combinat")).unwrap();
        fs::write(
            root.join("sage/all.py"),
            "from sage.combinat.all import *\n",
        )
        .unwrap();
        fs::write(
            root.join("sage/combinat/all.py"),
            "__all__ = [\"Combinations\"]\nfrom sage.combinat.combination import Combinations\n",
        )
        .unwrap();
        fs::write(
            root.join("sage/combinat/combination.py"),
            "def Combinations(mset, k=None):\n    \"\"\"Return combinations quickly.\"\"\"\n    return None\n",
        )
        .unwrap();
        let consumer = root.join("consumer.sage");
        let source = "C2_list = Combinations(n, 2).list()\n";
        fs::write(&consumer, source).unwrap();
        let index = WorkspaceIndex::new(IndexOptions {
            roots: vec![root.clone()],
            editable_roots: Vec::new(),
            exclude_globs: Vec::new(),
            cache_dir: root.join(".cache"),
            enable_pyx: true,
        });

        let query =
            index.query_source_symbol(&consumer, source, "Combinations", None, None, Vec::new());

        assert_eq!(
            query
                .definition
                .as_ref()
                .map(|definition| definition.path.as_path()),
            Some(normalize_path(root.join("sage/combinat/combination.py")).as_path())
        );
        assert_eq!(
            query
                .documentation
                .as_ref()
                .map(|documentation| documentation.summary.as_str()),
            Some("Return combinations quickly.")
        );
        assert_eq!(query.resolution_confidence.as_deref(), Some("high"));
        assert!(
            query.resolution_reason.as_deref().is_some_and(
                |reason| reason.contains("implicit .sage source-derived sage.all export chain")
            ),
            "{query:?}"
        );
        assert_eq!(query.candidate_count, 1);
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn query_keeps_local_shadow_before_implicit_sage_all_name() {
        let root = test_root("implicit-sage-all-local-shadow");
        fs::create_dir_all(root.join("sage/combinat")).unwrap();
        fs::write(
            root.join("sage/all.py"),
            "from sage.combinat.all import *\n",
        )
        .unwrap();
        fs::write(
            root.join("sage/combinat/all.py"),
            "__all__ = [\"Combinations\"]\nfrom sage.combinat.combination import Combinations\n",
        )
        .unwrap();
        fs::write(
            root.join("sage/combinat/combination.py"),
            "def Combinations(mset, k=None):\n    return None\n",
        )
        .unwrap();
        let consumer = root.join("consumer.sage");
        let source = "def Combinations():\n    \"\"\"Local helper.\"\"\"\n    return []\n\nvalue = Combinations()\n";
        fs::write(&consumer, source).unwrap();
        let index = WorkspaceIndex::new(IndexOptions {
            roots: vec![root.clone()],
            editable_roots: Vec::new(),
            exclude_globs: Vec::new(),
            cache_dir: root.join(".cache"),
            enable_pyx: true,
        });
        let (line, character) = position_in_line(source, "value =", "Combinations");

        let query =
            index.query_source_at(&consumer, source, QueryPosition { line, character }, None);

        assert_eq!(
            query
                .definition
                .as_ref()
                .map(|definition| definition.path.as_path()),
            Some(normalize_path(consumer.clone()).as_path())
        );
        assert_eq!(
            query
                .documentation
                .as_ref()
                .map(|documentation| documentation.summary.as_str()),
            Some("Local helper.")
        );
        assert_eq!(query.resolution_confidence.as_deref(), Some("high"));
        assert!(
            query
                .resolution_reason
                .as_deref()
                .is_some_and(|reason| reason.contains("shadows Sage import/export")),
            "{query:?}"
        );
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn query_resolves_symbols_from_sage_load_spyx_targets() {
        let root = test_root("sage-load-spyx-symbols");
        fs::create_dir_all(root.join("workspace")).unwrap();
        let loaded = root.join("workspace/fast_rank1.spyx");
        fs::write(
            &loaded,
            "def find_rank1_matrices(A, K, int n):\n    \"\"\"Find rank-one matrices in a Cython helper.\"\"\"\n    return []\n",
        )
        .unwrap();
        let consumer = root.join("workspace/full_attack.sage");
        let source = "load(\"fast_rank1.spyx\")\nrank1 = find_rank1_matrices(PK, K, n)\n";
        fs::write(&consumer, source).unwrap();
        let mut index = WorkspaceIndex::new(IndexOptions {
            roots: vec![root.join("workspace")],
            editable_roots: vec![root.join("workspace")],
            exclude_globs: Vec::new(),
            cache_dir: root.join(".cache"),
            enable_pyx: true,
        });
        index.rebuild().unwrap();

        let (line, character) = position_in_line(source, "rank1 =", "find_rank1_matrices");
        let query =
            index.query_source_at_navigation(&consumer, source, QueryPosition { line, character });

        assert_eq!(query.resolution_confidence.as_deref(), Some("high"));
        assert!(
            query
                .resolution_reason
                .as_deref()
                .is_some_and(|reason| reason.contains("load/attach target")),
            "{query:?}"
        );
        assert_eq!(
            query
                .definition
                .as_ref()
                .map(|definition| definition.path.as_path()),
            Some(normalize_path(loaded).as_path())
        );
        assert_eq!(
            query
                .documentation
                .as_ref()
                .map(|documentation| documentation.summary.as_str()),
            Some("Find rank-one matrices in a Cython helper.")
        );
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn query_prefers_documented_python_constructor_over_pxd_declaration() {
        let root = test_root("python-constructor-over-pxd");
        let package = root.join("sage/rings/number_field");
        fs::create_dir_all(&package).unwrap();
        let declaration = package.join("number_field_base.pxd");
        let cython_base = package.join("number_field_base.pyx");
        let implementation = package.join("number_field.py");
        let consumer = root.join("consumer.sage");
        fs::write(
            &declaration,
            "from sage.rings.ring cimport Field\n\ncdef class NumberField(Field):\n    pass\n",
        )
        .unwrap();
        fs::write(
            &cython_base,
            "from sage.rings.ring cimport Field\n\ncdef class NumberField(Field):\n    \"\"\"Base class docs.\"\"\"\n    pass\n",
        )
        .unwrap();
        fs::write(
            &implementation,
            "def NumberField(polynomial, name=None,\n                check=True, names=None):\n    \"\"\"Return the readable number field constructor.\"\"\"\n    return polynomial\n",
        )
        .unwrap();
        let source = "K = NumberField(poly, \"a\")\n";
        fs::write(&consumer, source).unwrap();
        let mut index = WorkspaceIndex::new(IndexOptions {
            roots: vec![root.clone()],
            editable_roots: Vec::new(),
            exclude_globs: Vec::new(),
            cache_dir: root.join(".cache"),
            enable_pyx: true,
        });
        index.rebuild().unwrap();

        let query =
            index.query_source_symbol(&consumer, source, "NumberField", None, None, Vec::new());

        assert_eq!(
            query
                .definition
                .as_ref()
                .map(|definition| definition.path.as_path()),
            Some(normalize_path(implementation.clone()).as_path())
        );
        assert_eq!(
            query
                .documentation
                .as_ref()
                .map(|documentation| documentation.summary.as_str()),
            Some("Return the readable number field constructor.")
        );
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn query_returns_hover_docs_definition_references_rename_and_signature() {
        let root = test_root("query-api");
        let source_path = root.join("demo.sage");
        let source = "def make_demo_matrix(n, scale=1):\n    \"\"\"Build a demo matrix.\"\"\"\n    return n * scale\n\nvalue = make_demo_matrix(2, scale=3)\n";
        fs::write(&source_path, source).unwrap();
        let mut index = WorkspaceIndex::new(IndexOptions {
            roots: vec![root.clone()],
            editable_roots: Vec::new(),
            exclude_globs: Vec::new(),
            cache_dir: root.join(".cache"),
            enable_pyx: true,
        });
        index.rebuild().unwrap();

        let query = index.query_source_symbol(
            &source_path,
            source,
            "make_demo_matrix",
            None,
            Some("renamed_demo_matrix"),
            Vec::new(),
        );

        assert!(query
            .hover
            .as_ref()
            .is_some_and(|hover| hover.markdown.contains("Build a demo matrix.")));
        assert_eq!(
            query
                .documentation
                .as_ref()
                .map(|docs| docs.summary.as_str()),
            Some("Build a demo matrix.")
        );
        assert_eq!(
            query
                .definition
                .as_ref()
                .map(|definition| definition.name.as_str()),
            Some("make_demo_matrix")
        );
        assert!(query.references.len() >= 2);
        assert_eq!(query.rename_preview.len(), query.references.len());
        assert_eq!(
            query
                .signature
                .as_ref()
                .map(|signature| signature.label.as_str()),
            Some("make_demo_matrix(n, scale=1)")
        );
        let completion_labels: Vec<_> = query
            .completions
            .iter()
            .map(|completion| completion.label.as_str())
            .collect();
        assert_eq!(
            completion_labels.len(),
            completion_labels
                .iter()
                .copied()
                .collect::<BTreeSet<_>>()
                .len()
        );
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn navigation_query_skips_expensive_lists_but_keeps_editor_payload() {
        let root = test_root("query-navigation-api");
        let source_path = root.join("demo.sage");
        let source = "def make_demo_matrix(n, scale=1):\n    \"\"\"Build a demo matrix.\"\"\"\n    return n * scale\n\nvalue = make_demo_matrix(2, scale=3)\n";
        fs::write(&source_path, source).unwrap();
        let mut index = WorkspaceIndex::new(IndexOptions {
            roots: vec![root.clone()],
            editable_roots: vec![root.clone()],
            exclude_globs: Vec::new(),
            cache_dir: root.join(".cache"),
            enable_pyx: true,
        });
        index.rebuild().unwrap();

        let query = index.query_source_symbol_with_options(
            &source_path,
            source,
            "make_demo_matrix",
            None,
            QueryExecutionOptions {
                rename_to: Some("renamed_demo_matrix"),
                diagnostics: Vec::new(),
                features: QueryFeatures::navigation(),
            },
        );

        assert!(query.hover.is_some());
        assert!(query.documentation.is_some());
        assert!(query.definition.is_some());
        assert!(query.signature.is_some());
        assert!(query.completions.is_empty());
        assert!(query.references.is_empty());
        assert!(query.rename_preview.is_empty());
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn query_hover_infers_assignment_value_kind_for_variables() {
        let root = test_root("assignment-hover-detail");
        let source_path = root.join("demo.sage");
        let source = "poly_preview = named_polynomial(\"x\")\nnotebook = PolynomialNotebook()\ncount = 5\nannotated: Matrix = make_demo_matrix()\n";
        fs::write(&source_path, source).unwrap();
        let mut index = WorkspaceIndex::new(IndexOptions {
            roots: vec![root.clone()],
            editable_roots: Vec::new(),
            exclude_globs: Vec::new(),
            cache_dir: root.join(".cache"),
            enable_pyx: true,
        });
        index.rebuild().unwrap();

        for (name, expected_detail) in [
            (
                "poly_preview",
                "Variable poly_preview: result of named_polynomial(...)",
            ),
            ("notebook", "Variable notebook: PolynomialNotebook"),
            ("count", "Variable count: Integer"),
            ("annotated", "Variable annotated: Matrix"),
        ] {
            let query =
                index.query_source_symbol(&source_path, source, name, None, None, Vec::new());
            assert!(
                query
                    .hover
                    .as_ref()
                    .is_some_and(|hover| hover.markdown.contains(expected_detail)),
                "expected hover for {name} to contain {expected_detail:?}: {:?}",
                query.hover
            );
        }

        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn query_prefers_current_module_variable_over_same_name_elsewhere() {
        let root = test_root("current-module-variable");
        let other_path = root.join("a_other.sage");
        let current_path = root.join("z_current.sage");
        let current_source = "E = EllipticCurve(GF(431), [0, 1])\npoints = E.points()\n";
        fs::write(&other_path, "E = 1\nother_points = E\n").unwrap();
        fs::write(&current_path, current_source).unwrap();
        let mut index = WorkspaceIndex::new(IndexOptions {
            roots: vec![root.clone()],
            editable_roots: vec![root.clone()],
            exclude_globs: Vec::new(),
            cache_dir: root.join(".cache"),
            enable_pyx: true,
        });
        index.rebuild().unwrap();

        let query = index.query_source_symbol(
            &current_path,
            current_source,
            "E",
            None,
            Some("current_curve"),
            Vec::new(),
        );

        assert!(query
            .hover
            .as_ref()
            .is_some_and(|hover| hover.markdown.contains("Variable E: EllipticCurve")));
        assert_eq!(
            query
                .definition
                .as_ref()
                .map(|definition| definition.path.as_path()),
            Some(normalize_path(current_path.clone()).as_path())
        );
        let current_path = normalize_path(current_path);
        assert!(query
            .references
            .iter()
            .all(|reference| reference.path == current_path));
        assert_eq!(query.rename_preview.len(), query.references.len());

        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn query_rename_preview_filters_to_editable_roots() {
        let root = test_root("editable-roots");
        let workspace = root.join("workspace");
        let library = root.join("sage-src");
        fs::create_dir_all(&workspace).unwrap();
        fs::create_dir_all(&library).unwrap();
        let source_path = workspace.join("demo.sage");
        let library_path = library.join("lib.py");
        let source = "def shared_symbol():\n    return 1\n\nvalue = shared_symbol()\n";
        fs::write(&source_path, source).unwrap();
        fs::write(
            &library_path,
            "def shared_symbol():\n    return 2\n\nother = shared_symbol()\n",
        )
        .unwrap();
        let mut index = WorkspaceIndex::new(IndexOptions {
            roots: vec![workspace.clone(), library.clone()],
            editable_roots: vec![workspace.clone()],
            exclude_globs: Vec::new(),
            cache_dir: root.join(".cache"),
            enable_pyx: true,
        });
        index.rebuild().unwrap();

        assert!(
            index.references("shared_symbol").len()
                > index.editable_references("shared_symbol").len()
        );
        let query = index.query_source_symbol(
            &source_path,
            source,
            "shared_symbol",
            None,
            Some("renamed_symbol"),
            Vec::new(),
        );

        let workspace = normalize_path(workspace);
        assert!(query
            .rename_preview
            .iter()
            .all(|edit| edit.path.starts_with(&workspace)));
        assert!(query
            .references
            .iter()
            .all(|reference| reference.path.starts_with(&workspace)));
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn query_skips_reference_scan_for_read_only_definitions() {
        let root = test_root("read-only-query-target");
        let workspace = root.join("workspace");
        let library = root.join("sage-src");
        fs::create_dir_all(&workspace).unwrap();
        fs::create_dir_all(&library).unwrap();
        let source_path = workspace.join("demo.py");
        let library_path = library.join("lib.py");
        let source = "from lib import ExternalThing\n\nvalue = ExternalThing()\n";
        fs::write(&source_path, source).unwrap();
        fs::write(
            &library_path,
            "def ExternalThing():\n    \"\"\"External constructor.\"\"\"\n    return object()\n",
        )
        .unwrap();
        let mut index = WorkspaceIndex::new(IndexOptions {
            roots: vec![workspace.clone(), library.clone()],
            editable_roots: vec![workspace],
            exclude_globs: Vec::new(),
            cache_dir: root.join(".cache"),
            enable_pyx: true,
        });
        index.rebuild().unwrap();

        let query = index.query_source_symbol(
            &source_path,
            source,
            "ExternalThing",
            None,
            Some("RenamedThing"),
            Vec::new(),
        );

        assert_eq!(
            query
                .definition
                .as_ref()
                .map(|definition| definition.path.as_path()),
            Some(normalize_path(library_path).as_path())
        );
        assert!(query.references.is_empty());
        assert!(query.rename_preview.is_empty());
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn query_keeps_hover_compact_while_preserving_full_docs() {
        let root = test_root("compact-hover");
        let source_path = root.join("demo.sage");
        let long_doc = (0..40)
            .map(|index| format!("Documentation line {index}."))
            .collect::<Vec<_>>()
            .join("\n");
        let indented_doc = long_doc.replace('\n', "\n    ");
        let source = format!(
            "def verbose_symbol():\n    \"\"\"{indented_doc}\"\"\"\n    return 1\n\nvalue = verbose_symbol()\n"
        );
        fs::write(&source_path, &source).unwrap();
        let mut index = WorkspaceIndex::new(IndexOptions {
            roots: vec![root.clone()],
            editable_roots: Vec::new(),
            exclude_globs: Vec::new(),
            cache_dir: root.join(".cache"),
            enable_pyx: true,
        });
        index.rebuild().unwrap();

        let query = index.query_source_symbol(
            &source_path,
            &source,
            "verbose_symbol",
            None,
            None,
            Vec::new(),
        );

        let hover = query.hover.as_ref().expect("hover should exist");
        assert!(hover.markdown.contains("Documentation line 0."));
        assert!(!hover.markdown.contains("Documentation line 39."));
        assert!(hover.markdown.contains("full docstring"));
        let docs = query.documentation.as_ref().expect("docs should exist");
        assert_eq!(docs.summary, "Documentation line 0.");
        assert!(docs
            .docstring
            .as_ref()
            .is_some_and(|docstring| docstring.contains("Documentation line 39.")));
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn query_resolves_instance_method_from_constructor_assignment() {
        let root = test_root("method-resolution");
        let consumer = root.join("consumer.sage");
        let provider = root.join("provider.py");
        fs::write(
            &consumer,
            "from provider import RingFactory\nR = RingFactory()\nvalue = R.rank()\n",
        )
        .unwrap();
        fs::write(
            &provider,
            "def RingFactory():\n    \"\"\"Build a ring.\"\"\"\n    return None\n\nclass Ring:\n    def rank(self):\n        \"\"\"Return the ring rank.\"\"\"\n        return 1\n",
        )
        .unwrap();
        let mut index = WorkspaceIndex::new(IndexOptions {
            roots: vec![root.clone()],
            editable_roots: Vec::new(),
            exclude_globs: Vec::new(),
            cache_dir: root.join(".cache"),
            enable_pyx: true,
        });
        index.rebuild().unwrap();
        let source = fs::read_to_string(&consumer).unwrap();

        let query = index.query_source_symbol(&consumer, &source, "rank", None, None, Vec::new());

        assert_eq!(
            query
                .definition
                .as_ref()
                .map(|definition| definition.path.as_path()),
            Some(normalize_path(provider.clone()).as_path())
        );
        assert_eq!(
            query
                .documentation
                .as_ref()
                .map(|documentation| documentation.summary.as_str()),
            Some("Return the ring rank.")
        );
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn query_resolves_sage_all_reexports_to_source_modules() {
        let root = test_root("sage-all-reexports");
        fs::create_dir_all(root.join("sage/matrix")).unwrap();
        fs::create_dir_all(root.join("sage/modules")).unwrap();
        fs::create_dir_all(root.join("sage/rings/finite_rings")).unwrap();
        fs::write(
            root.join("sage/matrix/constructor.pyx"),
            "def matrix(*args):\n    \"\"\"Create a Sage matrix.\"\"\"\n    return args\n",
        )
        .unwrap();
        fs::write(
            root.join("sage/matrix/special.py"),
            "def zero_matrix(*args):\n    \"\"\"Create a zero matrix.\"\"\"\n    return args\n",
        )
        .unwrap();
        fs::write(
            root.join("sage/modules/free_module_element.pyx"),
            "def vector(*args):\n    \"\"\"Create a vector.\"\"\"\n    return args\n",
        )
        .unwrap();
        fs::write(
            root.join("sage/rings/finite_rings/finite_field_constructor.py"),
            "def GF(order, name=None):\n    \"\"\"Return a finite field.\"\"\"\n    return order\n",
        )
        .unwrap();
        let consumer = root.join("consumer.py");
        let source = "from sage.all import (\n    matrix,\n    vector,\n    GF,\n    zero_matrix,\n)\nfield = GF(7)\nmat = matrix(field, 2, 2)\nvec = vector(field, [1, 2])\nzero = zero_matrix(field, 2, 2)\n";
        fs::write(&consumer, source).unwrap();
        let mut index = WorkspaceIndex::new(IndexOptions {
            roots: vec![root.clone()],
            editable_roots: Vec::new(),
            exclude_globs: Vec::new(),
            cache_dir: root.join(".cache"),
            enable_pyx: true,
        });
        index.rebuild().unwrap();

        for (name, expected_path, expected_doc) in [
            (
                "matrix",
                root.join("sage/matrix/constructor.pyx"),
                "Create a Sage matrix.",
            ),
            (
                "vector",
                root.join("sage/modules/free_module_element.pyx"),
                "Create a vector.",
            ),
            (
                "GF",
                root.join("sage/rings/finite_rings/finite_field_constructor.py"),
                "Return a finite field.",
            ),
            (
                "zero_matrix",
                root.join("sage/matrix/special.py"),
                "Create a zero matrix.",
            ),
        ] {
            let query = index.query_source_symbol(&consumer, source, name, None, None, Vec::new());
            assert_eq!(query.resolution_confidence.as_deref(), Some("high"));
            assert_eq!(
                query
                    .definition
                    .as_ref()
                    .map(|definition| definition.path.as_path()),
                Some(normalize_path(expected_path).as_path()),
                "wrong definition for {name}: {:?}",
                query.definition
            );
            assert_eq!(
                query
                    .documentation
                    .as_ref()
                    .map(|documentation| documentation.summary.as_str()),
                Some(expected_doc)
            );
        }
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn query_resolves_sage_all_wildcard_exports_before_global_homonyms() {
        let root = test_root("sage-all-wildcard-exports");
        fs::create_dir_all(root.join("sage/matrix")).unwrap();
        fs::create_dir_all(root.join("sage/modules")).unwrap();
        fs::write(
            root.join("sage/matrix/constructor.pyx"),
            "def matrix(*args):\n    \"\"\"Create a Sage matrix.\"\"\"\n    return args\n",
        )
        .unwrap();
        fs::write(
            root.join("sage/matrix/special.py"),
            "def zero_matrix(*args):\n    \"\"\"Create a zero matrix.\"\"\"\n    return args\n",
        )
        .unwrap();
        fs::write(
            root.join("sage/modules/free_module_element.pyx"),
            "def vector(*args):\n    \"\"\"Create a vector.\"\"\"\n    return args\n",
        )
        .unwrap();
        fs::write(
            root.join("homonyms.py"),
            "def matrix(self):\n    return self\n\ndef vector(self):\n    return self\n\ndef zero_matrix(self):\n    return self\n",
        )
        .unwrap();
        let consumer = root.join("consumer.py");
        let source = "from sage.all import *\nmat = matrix(GF(7), 2, 2)\nvec = vector(GF(7), [1, 2])\nzero = zero_matrix(GF(7), 2, 2)\n";
        fs::write(&consumer, source).unwrap();
        let mut index = WorkspaceIndex::new(IndexOptions {
            roots: vec![root.clone()],
            editable_roots: Vec::new(),
            exclude_globs: Vec::new(),
            cache_dir: root.join(".cache"),
            enable_pyx: true,
        });
        index.rebuild().unwrap();

        for (name, expected_path) in [
            ("matrix", root.join("sage/matrix/constructor.pyx")),
            ("vector", root.join("sage/modules/free_module_element.pyx")),
            ("zero_matrix", root.join("sage/matrix/special.py")),
        ] {
            let query = index.query_source_symbol(&consumer, source, name, None, None, Vec::new());
            assert_eq!(query.resolution_confidence.as_deref(), Some("high"));
            assert_eq!(
                query
                    .definition
                    .as_ref()
                    .map(|definition| definition.path.as_path()),
                Some(normalize_path(expected_path).as_path()),
                "wrong wildcard definition for {name}: {:?}",
                query.definition
            );
        }
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn query_resolves_indexed_sage_all_reexport_chains_without_hardcoded_map() {
        let root = test_root("sage-all-dynamic-reexports");
        fs::create_dir_all(root.join("sage/future")).unwrap();
        fs::write(
            root.join("sage/all.py"),
            "from sage.future.all import ChainFactory\nfrom sage.future.module import FuturePolynomialFactory as FutureFactory\n",
        )
        .unwrap();
        fs::write(
            root.join("sage/future/all.py"),
            "from sage.future.module import ChainedFutureFactory as ChainFactory\n",
        )
        .unwrap();
        fs::write(
            root.join("sage/future/module.py"),
            "def FuturePolynomialFactory(*args):\n    \"\"\"Build a future Sage polynomial factory.\"\"\"\n    return args\n\n\ndef ChainedFutureFactory(*args):\n    \"\"\"Build a chained future Sage factory.\"\"\"\n    return args\n",
        )
        .unwrap();
        fs::write(
            root.join("homonyms.py"),
            "def FutureFactory(*args):\n    \"\"\"Wrong local homonym.\"\"\"\n    return args\n\n\ndef ChainFactory(*args):\n    \"\"\"Wrong chained homonym.\"\"\"\n    return args\n",
        )
        .unwrap();
        let wildcard_consumer = root.join("wildcard_consumer.py");
        let wildcard_source =
            "from sage.all import *\nvalue = FutureFactory()\nchained = ChainFactory()\n";
        fs::write(&wildcard_consumer, wildcard_source).unwrap();
        let explicit_consumer = root.join("explicit_consumer.py");
        let explicit_source = "from sage.all import FutureFactory\nvalue = FutureFactory()\n";
        fs::write(&explicit_consumer, explicit_source).unwrap();

        let mut index = WorkspaceIndex::new(IndexOptions {
            roots: vec![root.clone()],
            editable_roots: Vec::new(),
            exclude_globs: Vec::new(),
            cache_dir: root.join(".cache"),
            enable_pyx: true,
        });
        index.rebuild().unwrap();

        for (name, expected_doc) in [
            ("FutureFactory", "Build a future Sage polynomial factory."),
            ("ChainFactory", "Build a chained future Sage factory."),
        ] {
            let query = index.query_source_symbol(
                &wildcard_consumer,
                wildcard_source,
                name,
                None,
                None,
                Vec::new(),
            );
            assert_eq!(query.resolution_confidence.as_deref(), Some("high"));
            assert!(query
                .resolution_reason
                .as_deref()
                .is_some_and(|reason| reason.contains("sage.all")));
            assert_eq!(
                query
                    .definition
                    .as_ref()
                    .map(|definition| definition.path.as_path()),
                Some(normalize_path(root.join("sage/future/module.py")).as_path()),
                "wrong dynamic wildcard definition for {name}: {:?}",
                query.definition
            );
            assert_eq!(
                query
                    .documentation
                    .as_ref()
                    .map(|documentation| documentation.summary.as_str()),
                Some(expected_doc)
            );
        }

        let explicit_query = index.query_source_symbol(
            &explicit_consumer,
            explicit_source,
            "FutureFactory",
            None,
            None,
            Vec::new(),
        );
        assert_eq!(
            explicit_query
                .definition
                .as_ref()
                .map(|definition| definition.path.as_path()),
            Some(normalize_path(root.join("sage/future/module.py")).as_path())
        );
        assert_eq!(
            explicit_query
                .documentation
                .as_ref()
                .map(|documentation| documentation.summary.as_str()),
            Some("Build a future Sage polynomial factory.")
        );
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn query_resolves_source_derived_catalog_namespace_members() {
        let root = test_root("sage-catalog-namespace-members");
        fs::create_dir_all(root.join("sage/coding")).unwrap();
        fs::create_dir_all(root.join("sage/schemes/toric")).unwrap();
        fs::write(
            root.join("sage/all.py"),
            "from sage.coding.all import *\nfrom sage.schemes.toric.all import *\n",
        )
        .unwrap();
        fs::write(
            root.join("sage/coding/all.py"),
            "from sage.misc.lazy_import import lazy_import\nlazy_import('sage.coding', 'codes_catalog', 'codes')\n",
        )
        .unwrap();
        fs::write(
            root.join("sage/coding/codes_catalog.py"),
            "from sage.misc.lazy_import import lazy_import as _lazy_import\n_lazy_import('.hamming_code', 'HammingCode')\n",
        )
        .unwrap();
        fs::write(
            root.join("sage/coding/hamming_code.py"),
            "class HammingCode:\n    \"\"\"Representation of a Hamming code.\"\"\"\n    pass\n",
        )
        .unwrap();
        fs::write(
            root.join("sage/schemes/toric/all.py"),
            "from sage.misc.lazy_import import lazy_import\nlazy_import('sage.schemes.toric.library', 'toric_varieties')\n",
        )
        .unwrap();
        fs::write(
            root.join("sage/schemes/toric/library.py"),
            "class ToricVarietyFactory:\n    def P2(self):\n        \"\"\"Return the projective plane.\"\"\"\n        return self\n\ntoric_varieties = ToricVarietyFactory()\n",
        )
        .unwrap();
        let consumer = root.join("consumer.sage");
        let source =
            "from sage.all import *\nC = codes.HammingCode(GF(2), 3)\nX = toric_varieties.P2()\n";
        fs::write(&consumer, source).unwrap();
        let mut index = WorkspaceIndex::new(IndexOptions {
            roots: vec![root.clone()],
            editable_roots: Vec::new(),
            exclude_globs: Vec::new(),
            cache_dir: root.join(".cache"),
            enable_pyx: true,
        });
        index.rebuild().unwrap();

        let (line, character) = first_position(source, "codes");
        let query =
            index.query_source_at(&consumer, source, QueryPosition { line, character }, None);
        assert_eq!(query.resolution_confidence.as_deref(), Some("high"));
        assert!(query
            .resolution_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("Sage namespace member")));
        assert_eq!(
            query
                .definition
                .as_ref()
                .map(|definition| definition.path.as_path()),
            Some(normalize_path(root.join("sage/coding/hamming_code.py")).as_path()),
            "wrong catalog member definition: {:?}",
            query.definition
        );
        assert_eq!(
            query
                .documentation
                .as_ref()
                .map(|documentation| documentation.summary.as_str()),
            Some("Representation of a Hamming code.")
        );

        let (line, character) = first_position(source, "toric_varieties");
        let query =
            index.query_source_at(&consumer, source, QueryPosition { line, character }, None);
        assert_eq!(query.resolution_confidence.as_deref(), Some("high"));
        assert_eq!(
            query
                .definition
                .as_ref()
                .map(|definition| definition.path.as_path()),
            Some(normalize_path(root.join("sage/schemes/toric/library.py")).as_path()),
            "wrong factory namespace member definition: {:?}",
            query.definition
        );
        assert_eq!(
            query
                .documentation
                .as_ref()
                .map(|documentation| documentation.summary.as_str()),
            Some("Return the projective plane.")
        );
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn query_resolves_source_derived_staticmethod_namespace_members() {
        let root = test_root("sage-staticmethod-namespace-members");
        fs::create_dir_all(root.join("sage/graphs/generators")).unwrap();
        fs::write(root.join("sage/all.py"), "from sage.graphs.all import *\n").unwrap();
        fs::write(
            root.join("sage/graphs/all.py"),
            "from sage.graphs.graph_generators import graphs\n",
        )
        .unwrap();
        fs::write(
            root.join("sage/graphs/graph_generators.py"),
            "class GraphGenerators:\n    from sage.graphs.generators import smallgraphs\n    PetersenGraph = staticmethod(smallgraphs.PetersenGraph)\n\ngraphs = GraphGenerators()\n",
        )
        .unwrap();
        fs::write(
            root.join("sage/graphs/generators/smallgraphs.py"),
            "def PetersenGraph(immutable=False):\n    \"\"\"Return the Petersen Graph.\"\"\"\n    return None\n",
        )
        .unwrap();
        let consumer = root.join("consumer.sage");
        let source = "value = graphs.PetersenGraph()\n";
        fs::write(&consumer, source).unwrap();
        let mut index = WorkspaceIndex::new(IndexOptions {
            roots: vec![root.clone()],
            editable_roots: Vec::new(),
            exclude_globs: Vec::new(),
            cache_dir: root.join(".cache"),
            enable_pyx: true,
        });
        index.rebuild().unwrap();

        let (line, character) = first_position(source, "graphs");
        let query =
            index.query_source_at(&consumer, source, QueryPosition { line, character }, None);
        assert_eq!(query.resolution_confidence.as_deref(), Some("high"));
        assert_eq!(
            query
                .definition
                .as_ref()
                .map(|definition| definition.path.as_path()),
            Some(normalize_path(root.join("sage/graphs/generators/smallgraphs.py")).as_path()),
            "wrong staticmethod namespace member definition: {:?}",
            query.definition
        );
        assert_eq!(
            query
                .documentation
                .as_ref()
                .map(|documentation| documentation.summary.as_str()),
            Some("Return the Petersen Graph.")
        );
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn query_resolves_sage_method_owners_and_suppresses_wrong_global_fallback() {
        let root = test_root("sage-method-owners");
        fs::create_dir_all(root.join("sage/combinat/matrices")).unwrap();
        fs::create_dir_all(root.join("sage/matrix")).unwrap();
        fs::create_dir_all(root.join("sage/rings/polynomial")).unwrap();
        fs::create_dir_all(root.join("sage/calculus")).unwrap();
        fs::write(
            root.join("sage/combinat/matrices/latin.py"),
            "def dumps(value):\n    \"\"\"Wrong json.dumps fallback.\"\"\"\n    return value\n",
        )
        .unwrap();
        fs::write(
            root.join("sage/matrix/matrix0.pyx"),
            "def rank(self):\n    \"\"\"Return matrix rank.\"\"\"\n    return 0\n\ndef base_ring(self):\n    \"\"\"Return matrix base ring.\"\"\"\n    return None\n",
        )
        .unwrap();
        fs::write(
            root.join("sage/matrix/matrix2.pyx"),
            "def right_kernel(self):\n    \"\"\"Return matrix right kernel.\"\"\"\n    return None\n",
        )
        .unwrap();
        fs::write(
            root.join("sage/rings/polynomial/multi_polynomial.pyx"),
            "def derivative(self, *args):\n    \"\"\"Differentiate this polynomial.\"\"\"\n    return self\n",
        )
        .unwrap();
        fs::write(
            root.join("sage/rings/polynomial/multi_polynomial_libsingular.pyx"),
            "def ideal(self, *args):\n    \"\"\"Create an ideal from this ring.\"\"\"\n    return args\n",
        )
        .unwrap();
        fs::write(
            root.join("sage/rings/polynomial/multi_polynomial_ideal.py"),
            "def variety(self, **kwds):\n    \"\"\"Return ideal variety.\"\"\"\n    return []\n",
        )
        .unwrap();
        fs::write(
            root.join("sage/calculus/functional.py"),
            "def derivative(x):\n    \"\"\"Wrong global derivative fallback.\"\"\"\n    return x\n\ndef append(x):\n    return x\n",
        )
        .unwrap();
        let consumer = root.join("consumer.py");
        let source = "from sage.all import GF, PolynomialRing, matrix\nfield = GF(7)\nring = PolynomialRing(field, names=[\"x\"])\nmat = matrix(field, 2, 2)\neqs = []\nrank_value = mat.rank()\nqs_field = Qs[0].base_ring()\nkernel = A.right_kernel()\njac = matrix(ring, 1, 1, lambda i, j: eqs[i].derivative(ring.gen(0)))\nideal = ring.ideal(eqs)\nroots = ideal.variety()\nno_jump = mat.append(1)\nencoded = json.dumps({})\n";
        fs::write(&consumer, source).unwrap();
        let mut index = WorkspaceIndex::new(IndexOptions {
            roots: vec![root.clone()],
            editable_roots: vec![root.clone()],
            exclude_globs: Vec::new(),
            cache_dir: root.join(".cache"),
            enable_pyx: true,
        });
        index.rebuild().unwrap();

        for (needle, expected_owner, expected_path) in [
            ("rank", "Matrix", root.join("sage/matrix/matrix0.pyx")),
            ("base_ring", "Matrix", root.join("sage/matrix/matrix0.pyx")),
            (
                "right_kernel",
                "Matrix",
                root.join("sage/matrix/matrix2.pyx"),
            ),
            (
                "derivative",
                "PolynomialElement",
                root.join("sage/rings/polynomial/multi_polynomial.pyx"),
            ),
            (
                "ideal",
                "PolynomialRing",
                root.join("sage/rings/polynomial/multi_polynomial_libsingular.pyx"),
            ),
            (
                "variety",
                "Ideal",
                root.join("sage/rings/polynomial/multi_polynomial_ideal.py"),
            ),
        ] {
            let (line, character) = member_position(source, needle);
            let query =
                index.query_source_at(&consumer, source, QueryPosition { line, character }, None);
            assert_eq!(query.owner_type.as_deref(), Some(expected_owner));
            assert_eq!(query.resolution_confidence.as_deref(), Some("high"));
            assert_eq!(query.candidate_count, 1);
            assert_eq!(
                query
                    .definition
                    .as_ref()
                    .map(|definition| definition.path.as_path()),
                Some(normalize_path(expected_path).as_path()),
                "wrong method target for {needle}: {:?}",
                query.definition
            );
        }

        let (line, character) = member_position(source, "append");
        let query =
            index.query_source_at(&consumer, source, QueryPosition { line, character }, None);
        assert_eq!(query.owner_type.as_deref(), Some("Matrix"));
        assert!(query.definition.is_none(), "{:?}", query.definition);
        assert!(query.fallback_reason.is_some(), "{query:?}");
        let (line, character) = member_position(source, "dumps");
        let query =
            index.query_source_at(&consumer, source, QueryPosition { line, character }, None);
        assert!(
            query.definition.is_none(),
            "unknown dotted stdlib member should not jump to Sage homonym: {query:?}"
        );
        assert!(query.fallback_reason.is_some(), "{query:?}");
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn query_resolves_graph_curve_and_number_field_methods() {
        let root = test_root("sage-object-methods");
        fs::create_dir_all(root.join("sage/graphs/base")).unwrap();
        fs::create_dir_all(root.join("sage/graphs")).unwrap();
        fs::create_dir_all(root.join("sage/schemes/elliptic_curves")).unwrap();
        fs::create_dir_all(root.join("sage/rings/number_field")).unwrap();
        fs::write(
            root.join("sage/graphs/generic_graph.py"),
            "def vertices(self):\n    \"\"\"Return graph vertices.\"\"\"\n    return []\n\n\
def shortest_path(self, u, v):\n    \"\"\"Return a shortest path.\"\"\"\n    return []\n\n\
def adjacency_matrix(self):\n    \"\"\"Return the graph adjacency matrix.\"\"\"\n    return None\n",
        )
        .unwrap();
        fs::write(
            root.join("sage/graphs/base/c_graph.pyx"),
            "def is_connected(self):\n    \"\"\"Return whether the graph is connected.\"\"\"\n    return True\n",
        )
        .unwrap();
        fs::write(
            root.join("sage/schemes/elliptic_curves/ell_finite_field.py"),
            "def points(self):\n    \"\"\"Return rational points.\"\"\"\n    return []\n\n\
def cardinality(self):\n    \"\"\"Return finite-curve cardinality.\"\"\"\n    return 0\n",
        )
        .unwrap();
        fs::write(
            root.join("sage/schemes/elliptic_curves/ell_rational_field.py"),
            "def rank(self):\n    \"\"\"Return Mordell-Weil rank.\"\"\"\n    return 0\n",
        )
        .unwrap();
        fs::write(
            root.join("sage/rings/number_field/number_field.py"),
            "def NumberField(polynomial, name=None):\n    \"\"\"Construct a number field.\"\"\"\n    return polynomial\n\n\
def gen(self, n=0):\n    \"\"\"Return a number field generator.\"\"\"\n    return n\n\n\
def degree(self):\n    \"\"\"Return the number field degree.\"\"\"\n    return 0\n\n\
def discriminant(self):\n    \"\"\"Return the number field discriminant.\"\"\"\n    return 0\n",
        )
        .unwrap();
        fs::write(
            root.join("sage/rings/number_field/number_field_base.pyx"),
            "def ring_of_integers(self):\n    \"\"\"Return the ring of integers.\"\"\"\n    return self\n",
        )
        .unwrap();
        let consumer = root.join("consumer.py");
        let source = "from sage.all import Graph, DiGraph, EllipticCurve, NumberField, GF, PolynomialRing, QQ\n\
R = PolynomialRing(QQ, \"x\")\n\
x = R.gen()\n\
G = Graph([(0, 1), (1, 2)])\n\
DG = DiGraph({0: [1]})\n\
vertices = G.vertices()\n\
connected = G.is_connected()\n\
adjacency = G.adjacency_matrix()\n\
path = DG.shortest_path(0, 1)\n\
E = EllipticCurve(GF(431), [0, 1])\n\
pts = E.points()\n\
cardinality = E.order()\n\
mw_rank = E.rank()\n\
K = NumberField(x**2 + 1, \"a\")\n\
a = K.gen()\n\
degree = K.degree()\n\
integers = K.ring_of_integers()\n\
disc = K.discriminant()\n";
        fs::write(&consumer, source).unwrap();
        let mut index = WorkspaceIndex::new(IndexOptions {
            roots: vec![root.clone()],
            editable_roots: vec![root.clone()],
            exclude_globs: Vec::new(),
            cache_dir: root.join(".cache"),
            enable_pyx: true,
        });
        index.rebuild().unwrap();

        for (needle, occurrence, expected_owner, expected_path, expected_doc) in [
            (
                "vertices",
                0,
                "Graph",
                root.join("sage/graphs/generic_graph.py"),
                "Return graph vertices.",
            ),
            (
                "is_connected",
                0,
                "Graph",
                root.join("sage/graphs/base/c_graph.pyx"),
                "Return whether the graph is connected.",
            ),
            (
                "adjacency_matrix",
                0,
                "Graph",
                root.join("sage/graphs/generic_graph.py"),
                "Return the graph adjacency matrix.",
            ),
            (
                "shortest_path",
                0,
                "Graph",
                root.join("sage/graphs/generic_graph.py"),
                "Return a shortest path.",
            ),
            (
                "points",
                0,
                "EllipticCurve",
                root.join("sage/schemes/elliptic_curves/ell_finite_field.py"),
                "Return rational points.",
            ),
            (
                "order",
                0,
                "EllipticCurve",
                root.join("sage/schemes/elliptic_curves/ell_finite_field.py"),
                "Return finite-curve cardinality.",
            ),
            (
                "rank",
                0,
                "EllipticCurve",
                root.join("sage/schemes/elliptic_curves/ell_rational_field.py"),
                "Return Mordell-Weil rank.",
            ),
            (
                "gen",
                1,
                "NumberField",
                root.join("sage/rings/number_field/number_field.py"),
                "Return a number field generator.",
            ),
            (
                "degree",
                0,
                "NumberField",
                root.join("sage/rings/number_field/number_field.py"),
                "Return the number field degree.",
            ),
            (
                "ring_of_integers",
                0,
                "NumberField",
                root.join("sage/rings/number_field/number_field_base.pyx"),
                "Return the ring of integers.",
            ),
            (
                "discriminant",
                0,
                "NumberField",
                root.join("sage/rings/number_field/number_field.py"),
                "Return the number field discriminant.",
            ),
        ] {
            let (line, character) = nth_member_position(source, needle, occurrence);
            let query =
                index.query_source_at(&consumer, source, QueryPosition { line, character }, None);
            assert_eq!(query.owner_type.as_deref(), Some(expected_owner));
            assert_eq!(query.resolution_confidence.as_deref(), Some("high"));
            assert_eq!(
                query
                    .definition
                    .as_ref()
                    .map(|definition| definition.path.as_path()),
                Some(normalize_path(expected_path).as_path()),
                "wrong object method target for {needle}: {:?}",
                query.definition
            );
            assert_eq!(
                query
                    .documentation
                    .as_ref()
                    .map(|documentation| documentation.summary.as_str()),
                Some(expected_doc),
                "wrong object method docs for {needle}: {:?}",
                query.documentation
            );
        }

        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn type_definition_resolves_sage_object_variables() {
        let root = test_root("sage-type-definition");
        fs::create_dir_all(root.join("sage/graphs")).unwrap();
        fs::create_dir_all(root.join("sage/schemes/elliptic_curves")).unwrap();
        fs::create_dir_all(root.join("sage/rings/number_field")).unwrap();
        fs::write(
            root.join("sage/graphs/graph.py"),
            "class Graph:\n    \"\"\"Graph type docs.\"\"\"\n    pass\n",
        )
        .unwrap();
        fs::write(
            root.join("sage/schemes/elliptic_curves/constructor.py"),
            "def EllipticCurve(*args):\n    \"\"\"Build an elliptic curve.\"\"\"\n    return args\n",
        )
        .unwrap();
        fs::write(
            root.join("sage/rings/number_field/number_field.py"),
            "def NumberField(polynomial, name=None):\n    \"\"\"Construct a number field.\"\"\"\n    return polynomial\n",
        )
        .unwrap();
        let consumer = root.join("consumer.py");
        let source = "from sage.all import Graph, EllipticCurve, NumberField, GF\n\
graph = Graph([(0, 1)])\n\
curve = EllipticCurve(GF(431), [0, 1])\n\
field = NumberField(poly, \"a\")\n\
graph_vertices = graph.vertices()\n\
curve_points = curve.points()\n\
field_degree = field.degree()\n";
        fs::write(&consumer, source).unwrap();
        let mut index = WorkspaceIndex::new(IndexOptions {
            roots: vec![root.clone()],
            editable_roots: vec![root.clone()],
            exclude_globs: Vec::new(),
            cache_dir: root.join(".cache"),
            enable_pyx: true,
        });
        index.rebuild().unwrap();

        for (needle, expected_path, expected_name) in [
            ("graph.vertices", root.join("sage/graphs/graph.py"), "Graph"),
            (
                "curve.points",
                root.join("sage/schemes/elliptic_curves/constructor.py"),
                "EllipticCurve",
            ),
            (
                "field.degree",
                root.join("sage/rings/number_field/number_field.py"),
                "NumberField",
            ),
        ] {
            let (line, character) = first_position(source, needle);
            let definition = index
                .type_definition_at_source(&consumer, source, QueryPosition { line, character })
                .expect("type definition should resolve");
            assert_eq!(definition.name, expected_name);
            assert_eq!(
                definition.path.as_path(),
                normalize_path(expected_path).as_path(),
                "wrong type definition target for {needle}: {definition:?}"
            );
        }

        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn query_resolves_research_helper_sage_methods() {
        let root = test_root("research-helper-methods");
        fs::create_dir_all(root.join("sage/matrix")).unwrap();
        fs::create_dir_all(root.join("sage/modules")).unwrap();
        fs::create_dir_all(root.join("sage/rings/finite_rings")).unwrap();
        fs::create_dir_all(root.join("sage/rings/polynomial")).unwrap();
        fs::create_dir_all(root.join("sage/structure")).unwrap();
        fs::write(
            root.join("sage/matrix/matrix0.pyx"),
            "def change_ring(self, ring):\n    \"\"\"Return this matrix over another ring.\"\"\"\n    return self\n",
        )
        .unwrap();
        fs::write(
            root.join("sage/matrix/matrix1.pyx"),
            "def matrix_from_columns(self, columns):\n    \"\"\"Return a matrix built from selected columns.\"\"\"\n    return self\n\n\ndef matrix_from_rows(self, rows):\n    \"\"\"Return a matrix built from selected rows.\"\"\"\n    return self\n",
        )
        .unwrap();
        fs::write(
            root.join("sage/matrix/matrix2.pyx"),
            "def charpoly(self, var='x'):\n    \"\"\"Return the characteristic polynomial.\"\"\"\n    return None\n\n\ndef adjugate(self):\n    \"\"\"Return the adjugate matrix.\"\"\"\n    return self\n",
        )
        .unwrap();
        fs::write(
            root.join("sage/modules/free_module.py"),
            "def basis(self):\n    \"\"\"Return a module basis.\"\"\"\n    return []\n\n\ndef basis_matrix(self, ring=None):\n    \"\"\"Return a matrix whose rows are a basis.\"\"\"\n    return None\n\n\ndef dimension(self):\n    \"\"\"Return the module dimension.\"\"\"\n    return 0\n",
        )
        .unwrap();
        fs::write(
            root.join("sage/rings/polynomial/polynomial_element_generic.py"),
            "def list(self, copy=True):\n    \"\"\"Return polynomial coefficients as a list.\"\"\"\n    return []\n",
        )
        .unwrap();
        fs::write(
            root.join("sage/rings/polynomial/polynomial_element.pyx"),
            "def factor(self, **kwargs):\n    \"\"\"Factor this polynomial.\"\"\"\n    return []\n\n\ndef monic(self):\n    \"\"\"Return the monic polynomial.\"\"\"\n    return self\n",
        )
        .unwrap();
        fs::write(
            root.join("sage/rings/polynomial/multi_polynomial_element.py"),
            "def degree(self, x=None):\n    \"\"\"Return the polynomial degree.\"\"\"\n    return 0\n",
        )
        .unwrap();
        fs::write(
            root.join("sage/rings/finite_rings/finite_field_base.pyx"),
            "def order(self):\n    \"\"\"Return the finite field order.\"\"\"\n    return 0\n\n\ndef from_integer(self, n, reverse=False):\n    \"\"\"Create a field element from an integer.\"\"\"\n    return n\n",
        )
        .unwrap();
        fs::write(
            root.join("sage/rings/finite_rings/element_base.pyx"),
            "def to_integer(self, reverse=False):\n    \"\"\"Return this finite-field element as an integer.\"\"\"\n    return 0\n",
        )
        .unwrap();
        fs::write(
            root.join("sage/rings/finite_rings/element_givaro.pyx"),
            "def polynomial(self, name=None):\n    \"\"\"Return this finite-field element as a polynomial.\"\"\"\n    return None\n\n\ndef _integer_representation(self):\n    \"\"\"Return the packed integer representation.\"\"\"\n    return 0\n",
        )
        .unwrap();
        fs::write(
            root.join("sage/structure/element.pyx"),
            "def base_ring(self):\n    \"\"\"Return the base ring of this element.\"\"\"\n    return None\n",
        )
        .unwrap();
        fs::write(
            root.join("homonyms.py"),
            "def list(value):\n    return value\n\ndef change_ring(value):\n    return value\n\ndef base_ring(value):\n    return value\n",
        )
        .unwrap();
        let consumer = root.join("consumer.py");
        let source = "def helper(A, poly, vec_obj, symbolic_matrix_obj, substitutions, field, f, y):\n    kernel = A.right_kernel()\n    direct_basis = A.right_kernel().basis()\n    basis = kernel.basis_matrix()\n    dims = kernel.dimension()\n    value = field.from_integer(field.order())\n    packed = value.integer_representation()\n    elem_poly = y.polynomial()\n    elem_coeffs = y.polynomial().list()\n    coeffs = poly.list()\n    cp = symbolic_matrix_obj.charpoly()\n    factors = cp.factor()\n    monic = f.monic()\n    local_ring = f.parent()\n    f1 = local_ring(f)\n    deg = f1.degree()\n    submatrix = symbolic_matrix_obj.matrix_from_columns([0]).matrix_from_rows([0]).adjugate()\n    changed = symbolic_matrix_obj.subs(substitutions).change_ring(field)\n    base = vec_obj.base_ring()\n    return basis, direct_basis, dims, packed, elem_poly, elem_coeffs, coeffs, factors, monic, deg, submatrix, value, changed, base\n";
        fs::write(&consumer, source).unwrap();
        let mut index = WorkspaceIndex::new(IndexOptions {
            roots: vec![root.clone()],
            editable_roots: vec![root.clone()],
            exclude_globs: Vec::new(),
            cache_dir: root.join(".cache"),
            enable_pyx: true,
        });
        index.rebuild().unwrap();

        for (needle, expected_owner, expected_path, expected_doc) in [
            (
                "basis",
                "FreeModule",
                root.join("sage/modules/free_module.py"),
                "Return a module basis.",
            ),
            (
                "basis_matrix",
                "FreeModule",
                root.join("sage/modules/free_module.py"),
                "Return a matrix whose rows are a basis.",
            ),
            (
                "dimension",
                "FreeModule",
                root.join("sage/modules/free_module.py"),
                "Return the module dimension.",
            ),
            (
                "list",
                "PolynomialElement",
                root.join("sage/rings/polynomial/polynomial_element_generic.py"),
                "Return polynomial coefficients as a list.",
            ),
            (
                "integer_representation",
                "FieldElement",
                root.join("sage/rings/finite_rings/element_base.pyx"),
                "Return this finite-field element as an integer.",
            ),
            (
                "polynomial",
                "FieldElement",
                root.join("sage/rings/finite_rings/element_givaro.pyx"),
                "Return this finite-field element as a polynomial.",
            ),
            (
                "charpoly",
                "Matrix",
                root.join("sage/matrix/matrix2.pyx"),
                "Return the characteristic polynomial.",
            ),
            (
                "factor",
                "PolynomialElement",
                root.join("sage/rings/polynomial/polynomial_element.pyx"),
                "Factor this polynomial.",
            ),
            (
                "monic",
                "PolynomialElement",
                root.join("sage/rings/polynomial/polynomial_element.pyx"),
                "Return the monic polynomial.",
            ),
            (
                "degree",
                "PolynomialElement",
                root.join("sage/rings/polynomial/multi_polynomial_element.py"),
                "Return the polynomial degree.",
            ),
            (
                "matrix_from_columns",
                "Matrix",
                root.join("sage/matrix/matrix1.pyx"),
                "Return a matrix built from selected columns.",
            ),
            (
                "matrix_from_rows",
                "Matrix",
                root.join("sage/matrix/matrix1.pyx"),
                "Return a matrix built from selected rows.",
            ),
            (
                "adjugate",
                "Matrix",
                root.join("sage/matrix/matrix2.pyx"),
                "Return the adjugate matrix.",
            ),
            (
                "from_integer",
                "Field",
                root.join("sage/rings/finite_rings/finite_field_base.pyx"),
                "Create a field element from an integer.",
            ),
            (
                "order",
                "Field",
                root.join("sage/rings/finite_rings/finite_field_base.pyx"),
                "Return the finite field order.",
            ),
            (
                "change_ring",
                "Matrix",
                root.join("sage/matrix/matrix0.pyx"),
                "Return this matrix over another ring.",
            ),
            (
                "base_ring",
                "Vector",
                root.join("sage/structure/element.pyx"),
                "Return the base ring of this element.",
            ),
        ] {
            let (line, character) = member_position(source, needle);
            let query =
                index.query_source_at(&consumer, source, QueryPosition { line, character }, None);
            assert_eq!(query.owner_type.as_deref(), Some(expected_owner));
            assert_eq!(query.resolution_confidence.as_deref(), Some("high"));
            assert_eq!(
                query
                    .definition
                    .as_ref()
                    .map(|definition| definition.path.as_path()),
                Some(normalize_path(expected_path).as_path()),
                "wrong helper method target for {needle}: {:?}",
                query.definition
            );
            assert_eq!(
                query
                    .documentation
                    .as_ref()
                    .map(|documentation| documentation.summary.as_str()),
                Some(expected_doc),
                "wrong helper method docs for {needle}: {:?}",
                query.documentation
            );
        }

        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn completion_items_are_owner_aware_for_sage_member_access() {
        let root = test_root("owner-aware-completion");
        fs::create_dir_all(root.join("sage/matrix")).unwrap();
        fs::write(
            root.join("sage/matrix/matrix0.pyx"),
            "def rank(self):\n    \"\"\"Return matrix rank.\"\"\"\n    return 0\n",
        )
        .unwrap();
        let mut index = WorkspaceIndex::new(IndexOptions {
            roots: vec![root.clone()],
            editable_roots: vec![root.clone()],
            exclude_globs: Vec::new(),
            cache_dir: root.join(".cache"),
            enable_pyx: true,
        });
        index.rebuild().unwrap();
        let source = "def demo(A, field, value, signature):\n    A.ra\n    field.o\n    value.integer_\n    signature.ba\n    text = 'A.ra'\n";

        let labels_at = |needle: &str| -> Vec<String> {
            let (line, character) = first_position(source, needle);
            index
                .completion_items_at_source(
                    source,
                    QueryPosition {
                        line,
                        character: character + needle.len() as u32,
                    },
                    20,
                )
                .into_iter()
                .map(|completion| completion.label)
                .collect()
        };

        let matrix_labels = labels_at("A.ra");
        assert!(
            matrix_labels.contains(&"rank".to_string()),
            "{matrix_labels:?}"
        );
        assert!(
            matrix_labels
                .iter()
                .all(|label| label.starts_with("ra") || label.starts_with("r")),
            "{matrix_labels:?}"
        );
        let (rank_line, rank_character) = first_position(source, "A.ra");
        let rank_completion = index
            .completion_items_at_source(
                source,
                QueryPosition {
                    line: rank_line,
                    character: rank_character + "A.ra".len() as u32,
                },
                20,
            )
            .into_iter()
            .find(|completion| completion.label == "rank")
            .expect("rank completion should exist");
        assert_eq!(rank_completion.signature.as_deref(), Some("rank(self)"));
        assert!(
            rank_completion
                .documentation
                .as_deref()
                .is_some_and(|docs| docs.contains("Return matrix rank.")),
            "{rank_completion:?}"
        );

        let field_labels = labels_at("field.o");
        assert!(
            field_labels.contains(&"order".to_string()),
            "{field_labels:?}"
        );

        let field_element_labels = labels_at("value.integer_");
        assert!(
            field_element_labels.contains(&"integer_representation".to_string()),
            "{field_element_labels:?}"
        );

        let vector_labels = labels_at("signature.ba");
        assert!(
            vector_labels.contains(&"base_ring".to_string()),
            "{vector_labels:?}"
        );

        let (line, character) = first_position(source, "'A.ra");
        let string_labels = index.completion_items_at_source(
            source,
            QueryPosition {
                line,
                character: character + "'A.ra".len() as u32,
            },
            20,
        );
        assert!(
            string_labels.is_empty(),
            "string literal member completions should be suppressed: {string_labels:?}"
        );
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn completion_items_are_local_first_for_open_documents() {
        let root = test_root("local-first-completion");
        fs::write(
            root.join("indexed.py"),
            "def kernel_archive():\n    pass\n\ndef scratch_global():\n    pass\n",
        )
        .unwrap();
        let mut index = WorkspaceIndex::new(IndexOptions {
            roots: vec![root.clone()],
            editable_roots: vec![root.clone()],
            exclude_globs: Vec::new(),
            cache_dir: root.join(".cache"),
            enable_pyx: true,
        });
        index.rebuild().unwrap();

        let source = "from sage.all import matrix\n\n\
def kernel_columns(A):\n    \"\"\"Return a basis matrix for the right kernel.\"\"\"\n    return A.right_kernel().basis_matrix()\n\n\
def demo(A, field):\n    scratch_matrix = matrix([])\n    ker\n    scra\n    fi\n";
        let (kernel_line, kernel_character) = first_position(source, "    ker");
        let kernel_completions = index.completion_items_at_source(
            source,
            QueryPosition {
                line: kernel_line,
                character: kernel_character + "    ker".len() as u32,
            },
            20,
        );
        let kernel_labels: Vec<_> = kernel_completions
            .iter()
            .map(|completion| completion.label.clone())
            .collect();
        assert_eq!(
            kernel_labels.first().map(String::as_str),
            Some("kernel_columns")
        );
        let kernel_completion = kernel_completions
            .first()
            .expect("kernel_columns completion should exist");
        assert_eq!(
            kernel_completion.signature.as_deref(),
            Some("kernel_columns(A)")
        );
        assert!(
            kernel_completion
                .documentation
                .as_deref()
                .is_some_and(|docs| docs.contains("right kernel")),
            "{kernel_completion:?}"
        );
        assert!(
            kernel_labels.contains(&"kernel_archive".to_string()),
            "{kernel_labels:?}"
        );

        let (scratch_line, scratch_character) = first_position(source, "    scra");
        let scratch_labels: Vec<_> = index
            .completion_items_at_source(
                source,
                QueryPosition {
                    line: scratch_line,
                    character: scratch_character + "    scra".len() as u32,
                },
                20,
            )
            .into_iter()
            .map(|completion| completion.label)
            .collect();
        assert_eq!(
            scratch_labels.first().map(String::as_str),
            Some("scratch_matrix"),
            "{scratch_labels:?}"
        );

        let (field_line, field_character) = first_position(source, "    fi");
        let parameter_labels: Vec<_> = index
            .completion_items_at_source(
                source,
                QueryPosition {
                    line: field_line,
                    character: field_character + "    fi".len() as u32,
                },
                20,
            )
            .into_iter()
            .map(|completion| completion.label)
            .collect();
        assert!(
            parameter_labels.contains(&"field".to_string()),
            "{parameter_labels:?}"
        );

        let multiline_source = "def collect_spectral_kernel(\n    Qs,\n    Q0inv,\n    n,\n    max_candidates=None,\n    search_seed=1,\n):\n    max_\n    search_\n";
        let body_position = |line_text: &str, prefix: &str| -> QueryPosition {
            let (line, character) = multiline_source
                .lines()
                .enumerate()
                .find_map(|(line, line_source)| {
                    if line_source == line_text {
                        Some((line as u32, line_source.find(prefix).unwrap() as u32))
                    } else {
                        None
                    }
                })
                .unwrap();
            QueryPosition {
                line,
                character: character + prefix.len() as u32,
            }
        };
        let multiline_parameter_labels: Vec<_> = index
            .completion_items_at_source(multiline_source, body_position("    max_", "max_"), 20)
            .into_iter()
            .map(|completion| completion.label)
            .collect();
        assert!(
            multiline_parameter_labels.contains(&"max_candidates".to_string()),
            "{multiline_parameter_labels:?}"
        );
        let second_parameter_labels: Vec<_> = index
            .completion_items_at_source(
                multiline_source,
                body_position("    search_", "search_"),
                20,
            )
            .into_iter()
            .map(|completion| completion.label)
            .collect();
        assert!(
            second_parameter_labels.contains(&"search_seed".to_string()),
            "{second_parameter_labels:?}"
        );

        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn query_resolves_lazy_import_alias_to_source_definition() {
        let root = test_root("lazy-query");
        let consumer = root.join("consumer.sage");
        let provider = root.join("external_series.py");
        fs::write(
            &consumer,
            "def lazy_import(module, names, as_=None, *, at_startup=False):\n    pass\n\nlazy_import('external_series', 'alternating_square_sum', 'alt_square_sum')\nvalue = alt_square_sum(5)\n",
        )
        .unwrap();
        fs::write(
            &provider,
            "def alternating_square_sum(n):\n    \"\"\"Return an alternating square sum.\"\"\"\n    return n\n",
        )
        .unwrap();
        let mut index = WorkspaceIndex::new(IndexOptions {
            roots: vec![root.clone()],
            editable_roots: Vec::new(),
            exclude_globs: Vec::new(),
            cache_dir: root.join(".cache"),
            enable_pyx: true,
        });
        index.rebuild().unwrap();

        let source = fs::read_to_string(&consumer).unwrap();
        let query =
            index.query_source_symbol(&consumer, &source, "alt_square_sum", None, None, Vec::new());

        assert_eq!(
            query
                .definition
                .as_ref()
                .map(|definition| definition.name.as_str()),
            Some("alternating_square_sum")
        );
        assert_eq!(
            query
                .documentation
                .as_ref()
                .map(|docs| docs.summary.as_str()),
            Some("Return an alternating square sum.")
        );
        assert_eq!(
            query
                .signature
                .as_ref()
                .map(|signature| signature.label.as_str()),
            Some("alternating_square_sum(n)")
        );
        assert_eq!(
            query
                .definition
                .as_ref()
                .map(|definition| definition.path.as_path()),
            Some(normalize_path(provider.clone()).as_path())
        );
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn query_resolves_deprecated_function_alias_to_replacement() {
        let root = test_root("deprecated-alias-query");
        let source_path = root.join("consumer.sage");
        let source = "from sage.misc.superseded import deprecated_function_alias\n\n\
def replacement(n):\n    \"\"\"Return the replacement value.\"\"\"\n    return n\n\n\
old_replacement = deprecated_function_alias(12345, replacement)\nvalue = old_replacement(5)\n";
        fs::write(&source_path, source).unwrap();
        let mut index = WorkspaceIndex::new(IndexOptions {
            roots: vec![root.clone()],
            editable_roots: Vec::new(),
            exclude_globs: Vec::new(),
            cache_dir: root.join(".cache"),
            enable_pyx: true,
        });
        index.rebuild().unwrap();

        let query = index.query_source_symbol(
            &source_path,
            source,
            "old_replacement",
            None,
            None,
            Vec::new(),
        );

        assert_eq!(
            query
                .definition
                .as_ref()
                .map(|definition| definition.name.as_str()),
            Some("replacement")
        );
        assert_eq!(
            query
                .documentation
                .as_ref()
                .map(|docs| docs.summary.as_str()),
            Some("Return the replacement value.")
        );
        assert_eq!(
            query
                .signature
                .as_ref()
                .map(|signature| signature.label.as_str()),
            Some("replacement(n)")
        );
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn query_resolves_import_member_alias_to_source_definition() {
        let root = test_root("member-alias-query");
        let provider_dir = root.join("sage/future");
        fs::create_dir_all(&provider_dir).unwrap();
        let all_path = provider_dir.join("all.py");
        let provider_path = provider_dir.join("module.py");
        let consumer_path = root.join("consumer.sage");
        fs::write(
            &all_path,
            "import sage.future.module as future_module\nFutureAlias = future_module.FutureFactory\n",
        )
        .unwrap();
        fs::write(
            &provider_path,
            "class FutureFactory:\n    \"\"\"Build a source-owned future factory.\"\"\"\n    pass\n",
        )
        .unwrap();
        fs::write(
            &consumer_path,
            "from sage.future.all import FutureAlias\nvalue = FutureAlias()\n",
        )
        .unwrap();
        let mut index = WorkspaceIndex::new(IndexOptions {
            roots: vec![root.clone()],
            editable_roots: Vec::new(),
            exclude_globs: Vec::new(),
            cache_dir: root.join(".cache"),
            enable_pyx: true,
        });
        index.rebuild().unwrap();
        let source = fs::read_to_string(&consumer_path).unwrap();

        let query = index.query_source_symbol(
            &consumer_path,
            &source,
            "FutureAlias",
            None,
            None,
            Vec::new(),
        );

        assert_eq!(
            query
                .definition
                .as_ref()
                .map(|definition| definition.name.as_str()),
            Some("FutureFactory")
        );
        assert_eq!(
            query
                .documentation
                .as_ref()
                .map(|docs| docs.summary.as_str()),
            Some("Build a source-owned future factory.")
        );
        assert_eq!(
            query
                .definition
                .as_ref()
                .map(|definition| definition.path.as_path()),
            Some(normalize_path(provider_path.clone()).as_path())
        );
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn query_resolves_local_definition_alias_to_source_definition() {
        let root = test_root("local-definition-alias-query");
        let all_path = root.join("module.py");
        let consumer_path = root.join("consumer.sage");
        fs::write(
            &all_path,
            "class Replacement:\n    \"\"\"Replacement class docs.\"\"\"\n    pass\n\nAlias = Replacement\n",
        )
        .unwrap();
        fs::write(
            &consumer_path,
            "from module import Alias\nvalue = Alias()\n",
        )
        .unwrap();
        let mut index = WorkspaceIndex::new(IndexOptions {
            roots: vec![root.clone()],
            editable_roots: Vec::new(),
            exclude_globs: Vec::new(),
            cache_dir: root.join(".cache"),
            enable_pyx: true,
        });
        index.rebuild().unwrap();
        let source = fs::read_to_string(&consumer_path).unwrap();

        let query =
            index.query_source_symbol(&consumer_path, &source, "Alias", None, None, Vec::new());

        assert_eq!(
            query
                .definition
                .as_ref()
                .map(|definition| definition.name.as_str()),
            Some("Replacement")
        );
        assert_eq!(
            query
                .documentation
                .as_ref()
                .map(|docs| docs.summary.as_str()),
            Some("Replacement class docs.")
        );
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn ambiguous_dotted_member_returns_explainable_docs_without_wrong_definition() {
        let root = test_root("ambiguous-member-docs");
        fs::create_dir_all(root.join("sage/categories")).unwrap();
        fs::write(
            root.join("sage/factory.py"),
            "def WeylGroup(data=None):\n    \"\"\"Build a Weyl group.\"\"\"\n    return object()\n",
        )
        .unwrap();
        fs::write(
            root.join("sage/categories/coxeter.py"),
            "class ParentMethods:\n    def simple_reflections(self):\n        \"\"\"Return Coxeter simple reflections.\"\"\"\n        return {}\n",
        )
        .unwrap();
        fs::write(
            root.join("sage/other.py"),
            "class Other:\n    def simple_reflections(self):\n        \"\"\"Return another simple-reflection implementation.\"\"\"\n        return {}\n",
        )
        .unwrap();
        let consumer = root.join("consumer.py");
        let source =
            "from sage.factory import WeylGroup\nW = WeylGroup(['A', 2])\ns = W.simple_reflections()\n";
        fs::write(&consumer, source).unwrap();
        let mut index = WorkspaceIndex::new(IndexOptions {
            roots: vec![root.clone()],
            editable_roots: vec![root.clone()],
            exclude_globs: Vec::new(),
            cache_dir: root.join(".cache"),
            enable_pyx: true,
        });
        index.rebuild().unwrap();

        let (line, character) = member_position(source, "simple_reflections");
        let query =
            index.query_source_at_navigation(&consumer, source, QueryPosition { line, character });
        assert!(
            query.definition.is_none(),
            "ambiguous member resolution must not jump to an arbitrary candidate"
        );
        assert_eq!(query.resolution_confidence.as_deref(), Some("ambiguous"));
        assert_eq!(query.candidate_count, 2);
        assert!(query
            .fallback_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("did not match constructor module")));
        let docs = query
            .documentation
            .as_ref()
            .expect("ambiguous member should provide explainable documentation");
        assert_eq!(docs.kind, "AmbiguousMember");
        assert!(docs.summary.contains("no definition jump"));
        assert!(docs.markers.iter().any(|marker| marker == "ambiguous"));
        assert_eq!(docs.sections.len(), 2);
        assert!(query
            .hover
            .as_ref()
            .is_some_and(|hover| hover.markdown.contains("Top indexed candidates")));
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn cache_hydrates_symbols_and_docs() {
        let root = test_root("hydrate");
        let source = root.join("docs.py");
        fs::write(
            &source,
            "def cached_symbol():\n    \"\"\"Cached docs.\"\"\"\n    return 1\n",
        )
        .unwrap();
        let options = IndexOptions {
            roots: vec![root.clone()],
            editable_roots: Vec::new(),
            exclude_globs: Vec::new(),
            cache_dir: root.join(".cache"),
            enable_pyx: true,
        };
        let mut index = WorkspaceIndex::new(options.clone());
        index.rebuild().unwrap();
        let mut hydrated = WorkspaceIndex::new(options);
        hydrated.hydrate_from_cache().unwrap();
        let symbol = hydrated
            .symbol("cached_symbol")
            .expect("hydrated symbol should exist");
        assert_eq!(symbol.docstring.as_deref(), Some("Cached docs."));
        assert!(hydrated
            .symbols_with_prefix("cached", 10)
            .iter()
            .any(|symbol| symbol.name == "cached_symbol"));
        assert!(hydrated
            .workspace_symbols("cached_symbol", 10)
            .iter()
            .any(|symbol| symbol.name == "cached_symbol"));
        assert!(hydrated
            .file_for_path(&source)
            .expect("hydrated file should exist")
            .symbols
            .iter()
            .any(|symbol| symbol.name == "cached_symbol"));
        assert!(hydrated.status().cache_hit_count > 0);
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn workspace_symbols_rank_exact_prefix_and_word_boundary_matches() {
        let root = test_root("workspace-symbol-ranking");
        fs::write(
            root.join("constructors.py"),
            [
                "def buildPolynomialRing():",
                "    return None",
                "",
                "def PolynomialRing():",
                "    return None",
                "",
                "def polynomial_ring_factory():",
                "    return None",
                "",
                "def string_helper():",
                "    return None",
            ]
            .join("\n"),
        )
        .unwrap();
        fs::write(
            root.join("uses.py"),
            [
                "from constructors import PolynomialRing",
                "",
                "def use_constructor():",
                "    return PolynomialRing()",
            ]
            .join("\n"),
        )
        .unwrap();
        let options = IndexOptions {
            roots: vec![root.clone()],
            editable_roots: Vec::new(),
            exclude_globs: Vec::new(),
            cache_dir: root.join(".cache"),
            enable_pyx: true,
        };
        let mut index = WorkspaceIndex::new(options.clone());
        index.rebuild().unwrap();

        let exact = index.workspace_symbols("PolynomialRing", 10);
        assert_eq!(
            exact.first().map(|symbol| symbol.name.as_str()),
            Some("PolynomialRing")
        );
        assert!(
            exact.iter().all(|symbol| symbol.name == "PolynomialRing"),
            "{exact:?}"
        );
        assert!(
            exact.iter().all(|symbol| symbol.kind != SymbolKind::Import),
            "workspace symbols should hide import noise when definitions exist: {exact:?}"
        );

        let prefix = index.workspace_symbols("poly", 10);
        assert_eq!(
            prefix.first().map(|symbol| symbol.name.as_str()),
            Some("PolynomialRing")
        );
        assert!(
            prefix
                .iter()
                .filter(|symbol| symbol.name == "PolynomialRing")
                .all(|symbol| symbol.kind != SymbolKind::Import),
            "prefix search should also hide duplicate import entries: {prefix:?}"
        );

        let boundary = index.workspace_symbols("ring", 10);
        assert_eq!(
            boundary.first().map(|symbol| symbol.name.as_str()),
            Some("polynomial_ring_factory")
        );

        let mut hydrated = WorkspaceIndex::new(options);
        hydrated.hydrate_from_cache().unwrap();
        let hydrated_exact = hydrated.workspace_symbols("PolynomialRing", 10);
        assert_eq!(
            hydrated_exact.first().map(|symbol| symbol.name.as_str()),
            Some("PolynomialRing")
        );
        assert!(
            hydrated_exact
                .iter()
                .all(|symbol| symbol.kind != SymbolKind::Import),
            "hydrated workspace symbols should keep the same import-noise suppression: {hydrated_exact:?}"
        );
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn cache_metadata_hydrates_counts_without_path_scan() {
        let root = test_root("metadata-hydrate");
        fs::write(
            root.join("docs.py"),
            "def cached_symbol():\n    \"\"\"Cached docs.\"\"\"\n    return 1\n",
        )
        .unwrap();
        let options = IndexOptions {
            roots: vec![root.clone()],
            editable_roots: Vec::new(),
            exclude_globs: Vec::new(),
            cache_dir: root.join(".cache"),
            enable_pyx: true,
        };
        let mut index = WorkspaceIndex::new(options.clone());
        index.rebuild().unwrap();
        let connection = Connection::open(index.db_path()).unwrap();
        let metadata_counts = load_cached_counts_from_metadata(&connection, &index.options().roots)
            .unwrap()
            .expect("root metadata should be persisted");
        assert_eq!(metadata_counts, (1, 2, 1));

        let mut hydrated = WorkspaceIndex::new(options);
        let status = hydrated.hydrate_from_cache().unwrap();
        assert_eq!(status.indexed_file_count, 1);
        assert_eq!(status.symbol_count, 2);
        assert_eq!(status.doc_count, 1);
        assert_eq!(status.last_operation.as_deref(), Some("hydrate"));
        assert!(status.last_hydrate_ms <= status.last_index_ms);
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn cache_metadata_counts_overlapping_roots_without_double_counting() {
        let root = test_root("metadata-overlap");
        let src = root.join("src");
        fs::create_dir_all(&src).unwrap();
        fs::write(
            root.join("top_level.py"),
            "def top_level_symbol():\n    return 1\n",
        )
        .unwrap();
        fs::write(
            src.join("nested.py"),
            "def nested_symbol():\n    \"\"\"Nested docs.\"\"\"\n    return 2\n",
        )
        .unwrap();
        let options = IndexOptions {
            roots: vec![root.clone(), src],
            editable_roots: Vec::new(),
            exclude_globs: Vec::new(),
            cache_dir: root.join(".cache"),
            enable_pyx: true,
        };
        let mut index = WorkspaceIndex::new(options.clone());
        let rebuilt = index.rebuild().unwrap();
        let connection = Connection::open(index.db_path()).unwrap();
        let metadata_counts = load_cached_counts_from_metadata(&connection, &index.options().roots)
            .unwrap()
            .expect("overlapping roots should use the parent root metadata row");
        assert_eq!(
            metadata_counts,
            (
                rebuilt.indexed_file_count,
                rebuilt.symbol_count,
                rebuilt.doc_count,
            ),
        );

        let mut hydrated = WorkspaceIndex::new(options);
        let status = hydrated.hydrate_from_cache().unwrap();
        assert_eq!(status.indexed_file_count, rebuilt.indexed_file_count);
        assert_eq!(status.symbol_count, rebuilt.symbol_count);
        assert_eq!(status.doc_count, rebuilt.doc_count);
        assert_eq!(status.cache_miss_count, 0);
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn cache_namespace_tracks_source_roots_for_future_sage_updates() {
        let root_a = test_root("cache-root-a");
        let root_b = test_root("cache-root-b");
        let cache_dir = test_root("cache-namespace");
        fs::write(root_a.join("module.py"), "def from_a():\n    return 1\n").unwrap();
        fs::write(root_b.join("module.py"), "def from_b():\n    return 2\n").unwrap();
        let options_a = IndexOptions {
            roots: vec![root_a.clone()],
            editable_roots: Vec::new(),
            exclude_globs: Vec::new(),
            cache_dir: cache_dir.clone(),
            enable_pyx: true,
        };
        let options_b = IndexOptions {
            roots: vec![root_b.clone()],
            editable_roots: Vec::new(),
            exclude_globs: Vec::new(),
            cache_dir,
            enable_pyx: true,
        };

        let mut index_a = WorkspaceIndex::new(options_a);
        let mut index_b = WorkspaceIndex::new(options_b);
        assert_ne!(index_a.db_path(), index_b.db_path());

        index_a.rebuild().unwrap();
        index_b.rebuild().unwrap();

        let mut reopened_a = WorkspaceIndex::new(index_a.options().clone());
        reopened_a.hydrate_from_cache().unwrap();
        assert!(reopened_a.symbol("from_a").is_some());
        assert!(reopened_a.symbol("from_b").is_none());

        let mut reopened_b = WorkspaceIndex::new(index_b.options().clone());
        reopened_b.hydrate_from_cache().unwrap();
        assert!(reopened_b.symbol("from_b").is_some());
        assert!(reopened_b.symbol("from_a").is_none());

        fs::remove_dir_all(root_a).ok();
        fs::remove_dir_all(root_b).ok();
        fs::remove_dir_all(index_a.options().cache_dir.clone()).ok();
    }

    #[test]
    fn status_exposes_cache_namespace_and_source_root_fingerprints() {
        let root = test_root("root-fingerprint");
        fs::create_dir_all(root.join("sage")).unwrap();
        fs::write(root.join("sage").join("version.py"), "version = '10.6'\n").unwrap();
        fs::write(root.join("module.py"), "def exported():\n    return 1\n").unwrap();
        let options = IndexOptions {
            roots: vec![root.clone()],
            editable_roots: Vec::new(),
            exclude_globs: vec!["**/__pycache__/**".to_string()],
            cache_dir: root.join(".cache"),
            enable_pyx: true,
        };
        let mut index = WorkspaceIndex::new(options);
        let status = index.rebuild().unwrap();
        assert_eq!(status.cache_namespace.len(), 16);
        assert_eq!(status.source_root_fingerprints.len(), 1);
        assert!(status.source_root_fingerprints[0].exists);
        assert_eq!(status.source_root_fingerprints[0].digest.len(), 16);
        assert!(status.source_root_fingerprints[0]
            .marker
            .as_deref()
            .is_some_and(|marker| marker.ends_with("sage/version.py")));

        let before = status.source_root_fingerprints[0].digest.clone();
        fs::write(root.join("sage").join("version.py"), "version = '10.7'\n").unwrap();
        let after = source_root_fingerprint(&root).digest;
        assert_ne!(before, after);

        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn hydrate_reports_stale_cache_when_source_root_fingerprint_changes() {
        let root = test_root("root-fingerprint-stale");
        fs::create_dir_all(root.join("sage")).unwrap();
        fs::write(root.join("sage").join("version.py"), "version = '10.6'\n").unwrap();
        fs::write(root.join("module.py"), "def exported():\n    return 1\n").unwrap();
        let options = IndexOptions {
            roots: vec![root.clone()],
            editable_roots: Vec::new(),
            exclude_globs: Vec::new(),
            cache_dir: root.join(".cache"),
            enable_pyx: true,
        };
        let mut index = WorkspaceIndex::new(options.clone());
        let original = index.rebuild().unwrap();
        assert!(!original.cache_stale);

        fs::write(root.join("sage").join("version.py"), "version = '10.7'\n").unwrap();
        let mut hydrated = WorkspaceIndex::new(options);
        let stale = hydrated.hydrate_from_cache().unwrap();
        assert!(stale.cache_stale);
        assert_eq!(stale.stale_source_roots.len(), 1);
        assert_eq!(
            stale.stale_source_roots[0].root,
            normalize_path(root.clone()).display().to_string()
        );
        assert_ne!(
            stale.stale_source_roots[0].cached_digest,
            stale.stale_source_roots[0].current_digest
        );
        assert!(stale.stale_source_roots[0]
            .current_marker
            .as_deref()
            .is_some_and(|marker| marker.ends_with("sage/version.py")));

        let reconciled = hydrated.reconcile_with_cache().unwrap();
        assert!(!reconciled.cache_stale);
        assert!(reconciled.stale_source_roots.is_empty());

        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn root_delete_uses_bulk_path_match_without_touching_other_roots() {
        let root = test_root("bulk-root-delete");
        let outside = test_root("bulk-root-keep");
        let connection = Connection::open_in_memory().unwrap();
        create_schema(&connection).unwrap();
        for path in [
            root.join("a.py"),
            root.join("nested").join("b.py"),
            outside.join("c.py"),
        ] {
            let path_text = path.display().to_string();
            connection
                .execute(
                    "insert into files(path, module, fingerprint) values(?1, 'm', 'f')",
                    params![path_text],
                )
                .unwrap();
            connection
                .execute(
                    "insert into symbols(name, kind, module, path, start_line, start_character, end_line, end_character, detail) values('x', 'Function', 'm', ?1, 0, 0, 0, 1, 'x()')",
                    params![path_text],
                )
                .unwrap();
            connection
                .execute(
                    "insert into docs(name, module, path, detail, docstring) values('x', 'm', ?1, 'x()', 'docs')",
                    params![path_text],
                )
                .unwrap();
            connection
                .execute(
                    "insert into reference_spans(name, path, start_line, start_character, end_line, end_character) values('x', ?1, 0, 0, 0, 1)",
                    params![path_text],
                )
                .unwrap();
        }

        delete_roots_from_db(&connection, std::slice::from_ref(&root)).unwrap();

        for table in ["files", "symbols", "docs", "reference_spans"] {
            let remaining: i64 = connection
                .query_row(&format!("select count(*) from {table}"), [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(
                remaining, 1,
                "{table} should keep only the outside root row"
            );
        }
        fs::remove_dir_all(root).ok();
        fs::remove_dir_all(outside).ok();
    }

    #[test]
    fn editable_reference_cache_hydrates_and_refreshes() {
        let sage_root = test_root("reference-cache-sage");
        let workspace = test_root("reference-cache-workspace");
        fs::write(
            sage_root.join("library.py"),
            "def target():\n    return target\n",
        )
        .unwrap();
        let first = workspace.join("first.py");
        let second = workspace.join("second.py");
        fs::write(&first, "def target():\n    return target()\n").unwrap();
        fs::write(&second, "value = target()\n").unwrap();
        let options = IndexOptions {
            roots: vec![sage_root.clone(), workspace.clone()],
            editable_roots: vec![workspace.clone()],
            exclude_globs: Vec::new(),
            cache_dir: workspace.join(".cache"),
            enable_pyx: true,
        };
        let mut index = WorkspaceIndex::new(options.clone());
        index.rebuild().unwrap();

        let connection = Connection::open(index.db_path()).unwrap();
        let cached_reference_count: usize = connection
            .query_row(
                "select count(*) from reference_spans where name = 'target'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            cached_reference_count, 3,
            "only editable workspace references should be materialized"
        );

        let mut hydrated = WorkspaceIndex::new(options);
        hydrated.hydrate_from_cache().unwrap();
        assert_eq!(hydrated.editable_references("target").len(), 3);

        fs::write(&first, "def replacement():\n    return 1\n").unwrap();
        fs::write(&second, "value = target() + target()\n").unwrap();
        hydrated
            .refresh_paths(&[first.clone(), second.clone()], &[])
            .unwrap();
        let references = hydrated.editable_references("target");
        assert_eq!(
            references.len(),
            2,
            "file-level upsert should remove stale spans and add changed spans"
        );
        let normalized_second = normalize_path(second);
        assert!(references
            .iter()
            .all(|reference| reference.path == normalized_second));

        fs::remove_dir_all(sage_root).ok();
        fs::remove_dir_all(workspace).ok();
    }

    #[test]
    fn full_rebuild_keeps_lookup_indexes_available() {
        let root = test_root("bulk-index-recreate");
        fs::write(root.join("mod.py"), "def indexed_symbol():\n    return 1\n").unwrap();
        let options = IndexOptions {
            roots: vec![root.clone()],
            editable_roots: Vec::new(),
            exclude_globs: Vec::new(),
            cache_dir: root.join(".cache"),
            enable_pyx: true,
        };
        let mut index = WorkspaceIndex::new(options);
        index.rebuild().unwrap();
        let connection = Connection::open(index.db_path()).unwrap();

        for (table, index_name) in [
            ("symbols", "idx_symbols_name"),
            ("symbols", "idx_symbols_module"),
            ("symbols", "idx_symbols_path"),
            ("docs", "idx_docs_path"),
            ("docs", "idx_docs_symbol"),
            ("sage_export_cache", "idx_sage_export_cache_path"),
            ("sage_method_cache", "idx_sage_method_cache_path"),
        ] {
            assert!(
                sqlite_index_exists(&connection, table, index_name),
                "{index_name} should exist on {table}"
            );
        }
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn hydrate_resolves_materialized_sage_all_export_cache() {
        let root = test_root("materialized-export-cache");
        fs::create_dir_all(root.join("sage/future")).unwrap();
        fs::write(
            root.join("sage/all.py"),
            "from sage.future.all import FutureFactory\n",
        )
        .unwrap();
        fs::write(
            root.join("sage/future/all.py"),
            "from sage.future.module import FutureFactory\n",
        )
        .unwrap();
        fs::write(
            root.join("sage/future/module.py"),
            "def FutureFactory():\n    \"\"\"Build a future factory from cache.\"\"\"\n    return None\n",
        )
        .unwrap();
        let consumer = root.join("consumer.py");
        let source = "from sage.all import *\nvalue = FutureFactory()\n";
        fs::write(&consumer, source).unwrap();
        let options = IndexOptions {
            roots: vec![root.clone()],
            editable_roots: Vec::new(),
            exclude_globs: Vec::new(),
            cache_dir: root.join(".cache"),
            enable_pyx: true,
        };
        let mut index = WorkspaceIndex::new(options.clone());
        index.rebuild().unwrap();

        let mut hydrated = WorkspaceIndex::new(options);
        hydrated.hydrate_from_cache().unwrap();
        let query = hydrated.query_source_symbol(
            &consumer,
            source,
            "FutureFactory",
            None,
            None,
            Vec::new(),
        );
        assert_eq!(
            query
                .definition
                .as_ref()
                .map(|definition| definition.path.as_path()),
            Some(normalize_path(root.join("sage/future/module.py")).as_path())
        );
        assert!(query
            .resolution_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("materialized sage.all export cache")));
        assert_eq!(
            query
                .documentation
                .as_ref()
                .map(|documentation| documentation.summary.as_str()),
            Some("Build a future factory from cache.")
        );
        let workspace_symbols = hydrated.workspace_symbols("FutureFactory", 10);
        assert_eq!(
            workspace_symbols
                .first()
                .map(|symbol| symbol.path.as_path()),
            Some(normalize_path(root.join("sage/future/module.py")).as_path())
        );
        assert!(
            workspace_symbols
                .iter()
                .all(|symbol| symbol.kind != SymbolKind::Import),
            "workspace symbols should reuse materialized exports without import noise: {workspace_symbols:?}"
        );
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn hydrate_materializes_full_sage_export_cache_from_source() {
        let root = test_root("materialized-full-export-cache");
        fs::create_dir_all(root.join("sage/future")).unwrap();
        fs::write(root.join("sage/all.py"), "from sage.future.all import *\n").unwrap();
        let export_count = MAX_DYNAMIC_HOT_EXPORT_NAMES + 8;
        let mut all_source = String::new();
        let mut module_source = String::new();
        for index in 0..export_count {
            let name = format!("FutureFactory{index:03}");
            all_source.push_str(&format!("from sage.future.module import {name}\n"));
            module_source.push_str(&format!(
                "def {name}():\n    \"\"\"Build future factory {index}.\"\"\"\n    return {index}\n\n"
            ));
        }
        fs::write(root.join("sage/future/all.py"), all_source).unwrap();
        fs::write(root.join("sage/future/module.py"), module_source).unwrap();
        let target = format!("FutureFactory{:03}", export_count - 1);
        let consumer = root.join("consumer.py");
        let source = format!("from sage.all import {target}\nvalue = {target}()\n");
        fs::write(&consumer, &source).unwrap();
        let options = IndexOptions {
            roots: vec![root.clone()],
            editable_roots: Vec::new(),
            exclude_globs: Vec::new(),
            cache_dir: root.join(".cache"),
            enable_pyx: true,
        };
        let mut index = WorkspaceIndex::new(options.clone());
        index.rebuild().unwrap();

        let mut hydrated = WorkspaceIndex::new(options);
        hydrated.hydrate_from_cache().unwrap();
        let query =
            hydrated.query_source_symbol(&consumer, &source, &target, None, None, Vec::new());
        assert_eq!(
            query
                .definition
                .as_ref()
                .map(|definition| definition.path.as_path()),
            Some(normalize_path(root.join("sage/future/module.py")).as_path())
        );
        assert!(query
            .resolution_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("materialized sage.all export cache")));
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn hydrate_resolves_materialized_lazy_import_list_exports() {
        let root = test_root("materialized-lazy-list-export-cache");
        fs::create_dir_all(root.join("sage/future")).unwrap();
        fs::write(root.join("sage/all.py"), "from sage.future.all import *\n").unwrap();
        fs::write(
            root.join("sage/future/all.py"),
            "from sage.misc.lazy_import import lazy_import\n\
lazy_import('sage.future.module', ['FutureFactory', 'FutureThing'])\n\
lazy_import(\n\
    'sage.future.aliases',\n\
    ['FutureAliasSource', 'SecondAliasSource'],\n\
    as_=['FutureAlias', 'SecondAlias'],\n\
)\n",
        )
        .unwrap();
        fs::write(
            root.join("sage/future/module.py"),
            "def FutureFactory():\n    \"\"\"Build a future factory.\"\"\"\n    return None\n\n\
def FutureThing():\n    \"\"\"Build a future thing.\"\"\"\n    return None\n",
        )
        .unwrap();
        fs::write(
            root.join("sage/future/aliases.py"),
            "def FutureAliasSource():\n    \"\"\"Build an aliased future object.\"\"\"\n    return None\n\n\
def SecondAliasSource():\n    \"\"\"Build a second aliased future object.\"\"\"\n    return None\n",
        )
        .unwrap();
        let consumer = root.join("consumer.py");
        let source = "from sage.all import FutureThing, FutureAlias\nthing = FutureThing()\nalias = FutureAlias()\n";
        fs::write(&consumer, source).unwrap();
        let options = IndexOptions {
            roots: vec![root.clone()],
            editable_roots: Vec::new(),
            exclude_globs: Vec::new(),
            cache_dir: root.join(".cache"),
            enable_pyx: true,
        };
        let mut index = WorkspaceIndex::new(options.clone());
        index.rebuild().unwrap();

        let mut hydrated = WorkspaceIndex::new(options);
        hydrated.hydrate_from_cache().unwrap();
        for (name, expected_path, expected_doc) in [
            (
                "FutureThing",
                root.join("sage/future/module.py"),
                "Build a future thing.",
            ),
            (
                "FutureAlias",
                root.join("sage/future/aliases.py"),
                "Build an aliased future object.",
            ),
        ] {
            let query =
                hydrated.query_source_symbol(&consumer, source, name, None, None, Vec::new());
            assert_eq!(
                query
                    .definition
                    .as_ref()
                    .map(|definition| definition.path.as_path()),
                Some(normalize_path(expected_path).as_path()),
                "wrong materialized lazy import target for {name}: {:?}",
                query.definition
            );
            assert_eq!(
                query
                    .documentation
                    .as_ref()
                    .map(|documentation| documentation.summary.as_str()),
                Some(expected_doc)
            );
        }
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn hydrate_resolves_materialized_lazy_import_object_assignment() {
        let root = test_root("materialized-lazy-object-export-cache");
        fs::create_dir_all(root.join("sage/future")).unwrap();
        fs::write(root.join("sage/all.py"), "from sage.future.all import *\n").unwrap();
        fs::write(
            root.join("sage/future/all.py"),
            "from sage.misc.lazy_import import LazyImport\n\
FutureCategory = LazyImport(\n\
    'sage.future.categories',\n\
    'FutureCategory',\n\
    at_startup=True,\n\
)\n",
        )
        .unwrap();
        fs::write(
            root.join("sage/future/categories.py"),
            "class FutureCategory:\n    \"\"\"Describe a future category.\"\"\"\n    pass\n",
        )
        .unwrap();
        let consumer = root.join("consumer.py");
        let source = "from sage.all import FutureCategory\ncategory = FutureCategory()\n";
        fs::write(&consumer, source).unwrap();
        let options = IndexOptions {
            roots: vec![root.clone()],
            editable_roots: Vec::new(),
            exclude_globs: Vec::new(),
            cache_dir: root.join(".cache"),
            enable_pyx: true,
        };
        let mut index = WorkspaceIndex::new(options.clone());
        index.rebuild().unwrap();

        let mut hydrated = WorkspaceIndex::new(options);
        hydrated.hydrate_from_cache().unwrap();
        let query = hydrated.query_source_symbol(
            &consumer,
            source,
            "FutureCategory",
            None,
            None,
            Vec::new(),
        );
        assert_eq!(
            query
                .definition
                .as_ref()
                .map(|definition| definition.path.as_path()),
            Some(normalize_path(root.join("sage/future/categories.py")).as_path())
        );
        assert_eq!(
            query
                .documentation
                .as_ref()
                .map(|documentation| documentation.summary.as_str()),
            Some("Describe a future category.")
        );
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn hydrate_resolves_sage_all_alias_assignments_from_indexed_imports() {
        let root = test_root("materialized-alias-export-cache");
        fs::create_dir_all(root.join("sage/rings/finite_rings")).unwrap();
        fs::write(root.join("sage/all.py"), "from sage.rings.all import *\n").unwrap();
        fs::write(
            root.join("sage/rings/all.py"),
            "from sage.rings.finite_rings.all import *\n",
        )
        .unwrap();
        fs::write(
            root.join("sage/rings/finite_rings/all.py"),
            "from sage.rings.finite_rings.constructor import FiniteField\nGF = FiniteField\n",
        )
        .unwrap();
        fs::write(
            root.join("sage/rings/finite_rings/constructor.py"),
            "def FiniteField(order, name=None):\n    \"\"\"Return a finite field.\"\"\"\n    return None\n",
        )
        .unwrap();
        let consumer = root.join("consumer.py");
        let source = "from sage.all import GF\nfield = GF(2)\n";
        fs::write(&consumer, source).unwrap();
        let options = IndexOptions {
            roots: vec![root.clone()],
            editable_roots: Vec::new(),
            exclude_globs: Vec::new(),
            cache_dir: root.join(".cache"),
            enable_pyx: true,
        };
        let mut index = WorkspaceIndex::new(options.clone());
        index.rebuild().unwrap();

        let mut hydrated = WorkspaceIndex::new(options);
        hydrated.hydrate_from_cache().unwrap();
        let query = hydrated.query_source_symbol(&consumer, source, "GF", None, None, Vec::new());
        assert_eq!(
            query
                .definition
                .as_ref()
                .map(|definition| definition.path.as_path()),
            Some(normalize_path(root.join("sage/rings/finite_rings/constructor.py")).as_path())
        );
        assert_eq!(
            query
                .documentation
                .as_ref()
                .map(|documentation| documentation.summary.as_str()),
            Some("Return a finite field.")
        );
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn hydrate_resolves_transitive_sage_all_star_reexports_by_module() {
        let root = test_root("materialized-star-reexport-cache");
        fs::create_dir_all(root.join("sage/future")).unwrap();
        fs::create_dir_all(root.join("sage/private")).unwrap();
        fs::write(root.join("sage/all.py"), "from sage.future.all import *\n").unwrap();
        fs::write(
            root.join("sage/future/all.py"),
            "from sage.future.module import FutureOnly\n",
        )
        .unwrap();
        fs::write(
            root.join("sage/future/module.py"),
            "def FutureOnly():\n    \"\"\"Build a future-only public export.\"\"\"\n    return None\n",
        )
        .unwrap();
        fs::write(
            root.join("sage/private/all.py"),
            "from sage.private.module import PrivateOnly\n",
        )
        .unwrap();
        fs::write(
            root.join("sage/private/module.py"),
            "def PrivateOnly():\n    \"\"\"This is not exported by sage.all.\"\"\"\n    return None\n",
        )
        .unwrap();
        let public_consumer = root.join("public_consumer.py");
        let public_source = "from sage.all import FutureOnly\nvalue = FutureOnly()\n";
        fs::write(&public_consumer, public_source).unwrap();
        let private_consumer = root.join("private_consumer.py");
        let private_source = "from sage.all import PrivateOnly\nvalue = PrivateOnly()\n";
        fs::write(&private_consumer, private_source).unwrap();
        let options = IndexOptions {
            roots: vec![root.clone()],
            editable_roots: Vec::new(),
            exclude_globs: Vec::new(),
            cache_dir: root.join(".cache"),
            enable_pyx: true,
        };
        let mut index = WorkspaceIndex::new(options.clone());
        index.rebuild().unwrap();

        let mut hydrated = WorkspaceIndex::new(options);
        hydrated.hydrate_from_cache().unwrap();
        let public_query = hydrated.query_source_symbol(
            &public_consumer,
            public_source,
            "FutureOnly",
            None,
            None,
            Vec::new(),
        );
        assert_eq!(
            public_query
                .definition
                .as_ref()
                .map(|definition| definition.path.as_path()),
            Some(normalize_path(root.join("sage/future/module.py")).as_path())
        );
        assert!(public_query
            .resolution_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("materialized sage.all export cache")));
        let private_query = hydrated.query_source_symbol(
            &private_consumer,
            private_source,
            "PrivateOnly",
            None,
            None,
            Vec::new(),
        );
        assert_ne!(
            private_query
                .definition
                .as_ref()
                .map(|definition| definition.path.as_path()),
            Some(normalize_path(root.join("sage/private/module.py")).as_path()),
            "module-specific export cache should not treat every sage.*.all name as sage.all"
        );
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn hydrate_resolves_star_reexports_from_plain_sage_modules() {
        let root = test_root("materialized-plain-module-star-cache");
        fs::create_dir_all(root.join("sage/categories")).unwrap();
        fs::write(
            root.join("sage/all.py"),
            "from sage.categories.all import *\n",
        )
        .unwrap();
        fs::write(
            root.join("sage/categories/all.py"),
            "from sage.categories.basic import *\n",
        )
        .unwrap();
        fs::write(
            root.join("sage/categories/basic.py"),
            "from sage.categories.posets import Posets\n\
OrderedSets = Posets\n\
\n\
class LocalCategory:\n\
    \"\"\"A category defined in the star-imported module.\"\"\"\n\
    pass\n\
\n\
class _PrivateCategory:\n\
    \"\"\"This private helper should not be star-exported.\"\"\"\n\
    pass\n",
        )
        .unwrap();
        fs::write(
            root.join("sage/categories/posets.py"),
            "class Posets:\n    \"\"\"Category of posets.\"\"\"\n    pass\n",
        )
        .unwrap();
        let consumer = root.join("consumer.py");
        let source = "from sage.all import OrderedSets, LocalCategory\n\
ordered = OrderedSets()\n\
category = LocalCategory()\n";
        fs::write(&consumer, source).unwrap();
        let options = IndexOptions {
            roots: vec![root.clone()],
            editable_roots: Vec::new(),
            exclude_globs: Vec::new(),
            cache_dir: root.join(".cache"),
            enable_pyx: true,
        };
        let mut index = WorkspaceIndex::new(options.clone());
        index.rebuild().unwrap();
        let connection = Connection::open(index.db_path()).unwrap();
        refresh_materialized_caches(&connection, &index.options().roots).unwrap();

        let mut hydrated = WorkspaceIndex::new(options);
        hydrated.hydrate_from_cache().unwrap();
        for (name, expected_path, expected_doc) in [
            (
                "OrderedSets",
                root.join("sage/categories/posets.py"),
                "Category of posets.",
            ),
            (
                "LocalCategory",
                root.join("sage/categories/basic.py"),
                "A category defined in the star-imported module.",
            ),
        ] {
            let query =
                hydrated.query_source_symbol(&consumer, source, name, None, None, Vec::new());
            assert_eq!(
                query
                    .definition
                    .as_ref()
                    .map(|definition| definition.path.as_path()),
                Some(normalize_path(expected_path).as_path()),
                "wrong plain-module star export target for {name}: {:?}",
                query.definition
            );
            assert_eq!(
                query
                    .documentation
                    .as_ref()
                    .map(|documentation| documentation.summary.as_str()),
                Some(expected_doc)
            );
        }

        let private_query = hydrated.query_source_symbol(
            &consumer,
            "from sage.all import _PrivateCategory\nvalue = _PrivateCategory()\n",
            "_PrivateCategory",
            None,
            None,
            Vec::new(),
        );
        assert_ne!(
            private_query
                .definition
                .as_ref()
                .map(|definition| definition.path.as_path()),
            Some(normalize_path(root.join("sage/categories/basic.py")).as_path()),
            "private names from a plain star-imported module must not be re-exported"
        );
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn hydrate_respects_dunder_all_for_plain_module_star_reexports() {
        let root = test_root("materialized-plain-module-dunder-all-cache");
        fs::create_dir_all(root.join("sage/categories")).unwrap();
        fs::write(
            root.join("sage/all.py"),
            "from sage.categories.all import *\n",
        )
        .unwrap();
        fs::write(
            root.join("sage/categories/all.py"),
            "from sage.categories.basic import *\n",
        )
        .unwrap();
        fs::write(
            root.join("sage/categories/basic.py"),
            "from sage.categories.posets import Posets\n\
VisibleAlias = Posets\n\
__all__ = [\n\
    'VisibleAlias',\n\
]\n\
__all__.append('AppendedCategory')\n\
\n\
class AppendedCategory:\n\
    \"\"\"Category added through __all__.append.\"\"\"\n\
    pass\n\
\n\
class PublicButHidden:\n\
    \"\"\"This public-looking class is intentionally absent from __all__.\"\"\"\n\
    pass\n",
        )
        .unwrap();
        fs::write(
            root.join("sage/categories/posets.py"),
            "class Posets:\n    \"\"\"Category of posets.\"\"\"\n    pass\n",
        )
        .unwrap();
        let consumer = root.join("consumer.py");
        let source = "from sage.all import VisibleAlias, AppendedCategory\n\
visible = VisibleAlias()\n\
appended = AppendedCategory()\n";
        fs::write(&consumer, source).unwrap();
        let options = IndexOptions {
            roots: vec![root.clone()],
            editable_roots: Vec::new(),
            exclude_globs: Vec::new(),
            cache_dir: root.join(".cache"),
            enable_pyx: true,
        };
        let mut index = WorkspaceIndex::new(options.clone());
        index.rebuild().unwrap();
        let connection = Connection::open(index.db_path()).unwrap();
        refresh_materialized_caches(&connection, &index.options().roots).unwrap();

        let mut hydrated = WorkspaceIndex::new(options);
        hydrated.hydrate_from_cache().unwrap();
        for (name, expected_path, expected_doc) in [
            (
                "VisibleAlias",
                root.join("sage/categories/posets.py"),
                "Category of posets.",
            ),
            (
                "AppendedCategory",
                root.join("sage/categories/basic.py"),
                "Category added through __all__.append.",
            ),
        ] {
            let query =
                hydrated.query_source_symbol(&consumer, source, name, None, None, Vec::new());
            assert_eq!(
                query
                    .definition
                    .as_ref()
                    .map(|definition| definition.path.as_path()),
                Some(normalize_path(expected_path).as_path()),
                "wrong __all__ star export target for {name}: {:?}",
                query.definition
            );
            assert_eq!(
                query
                    .documentation
                    .as_ref()
                    .map(|documentation| documentation.summary.as_str()),
                Some(expected_doc)
            );
        }

        let hidden_source = "from sage.all import PublicButHidden\nvalue = PublicButHidden()\n";
        let hidden_query = hydrated.query_source_symbol(
            &consumer,
            hidden_source,
            "PublicButHidden",
            None,
            None,
            Vec::new(),
        );
        assert!(
            hidden_query.definition.is_none(),
            "__all__ should prevent fallback to a public-looking but unexported class: {:?}",
            hidden_query.definition
        );
        assert_eq!(
            hidden_query.resolution_confidence.as_deref(),
            Some("ambiguous")
        );
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn query_prefers_local_symbol_over_sage_all_wildcard_export() {
        let root = test_root("local-shadow-wildcard-export");
        fs::create_dir_all(root.join("sage/matrix")).unwrap();
        fs::write(
            root.join("sage/all.py"),
            "from sage.matrix.constructor import matrix\n",
        )
        .unwrap();
        fs::write(
            root.join("sage/matrix/constructor.py"),
            "def matrix(entries=None):\n    \"\"\"Build a Sage matrix.\"\"\"\n    return entries\n",
        )
        .unwrap();
        let consumer = root.join("consumer.py");
        let source = "from sage.all import *\n\
def matrix(value):\n\
    \"\"\"Local matrix helper.\"\"\"\n\
    return value\n\
\n\
result = matrix(1)\n";
        fs::write(&consumer, source).unwrap();
        let options = IndexOptions {
            roots: vec![root.clone()],
            editable_roots: vec![root.clone()],
            exclude_globs: Vec::new(),
            cache_dir: root.join(".cache"),
            enable_pyx: true,
        };
        let mut index = WorkspaceIndex::new(options);
        index.rebuild().unwrap();

        let (line, character) = position_in_line(source, "result =", "matrix");
        let query =
            index.query_source_at_navigation(&consumer, source, QueryPosition { line, character });
        assert_eq!(
            query
                .definition
                .as_ref()
                .map(|definition| definition.path.as_path()),
            Some(normalize_path(consumer.clone()).as_path())
        );
        assert_eq!(
            query
                .documentation
                .as_ref()
                .map(|documentation| documentation.summary.as_str()),
            Some("Local matrix helper.")
        );
        assert!(query
            .resolution_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("shadows Sage import/export")));
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn query_prefers_later_local_symbol_over_explicit_sage_import_for_usage() {
        let root = test_root("local-shadow-explicit-export");
        fs::create_dir_all(root.join("sage/matrix")).unwrap();
        fs::write(
            root.join("sage/all.py"),
            "from sage.matrix.constructor import matrix\n",
        )
        .unwrap();
        fs::write(
            root.join("sage/matrix/constructor.py"),
            "def matrix(entries=None):\n    \"\"\"Build a Sage matrix.\"\"\"\n    return entries\n",
        )
        .unwrap();
        let consumer = root.join("consumer.py");
        let source = "from sage.all import matrix\n\
def matrix(value):\n\
    \"\"\"Local matrix helper.\"\"\"\n\
    return value\n\
\n\
result = matrix(1)\n";
        fs::write(&consumer, source).unwrap();
        let options = IndexOptions {
            roots: vec![root.clone()],
            editable_roots: vec![root.clone()],
            exclude_globs: Vec::new(),
            cache_dir: root.join(".cache"),
            enable_pyx: true,
        };
        let mut index = WorkspaceIndex::new(options);
        index.rebuild().unwrap();

        let (line, character) = position_in_line(source, "result =", "matrix");
        let query =
            index.query_source_at_navigation(&consumer, source, QueryPosition { line, character });
        assert_eq!(
            query
                .definition
                .as_ref()
                .map(|definition| definition.path.as_path()),
            Some(normalize_path(consumer.clone()).as_path())
        );
        assert_eq!(
            query
                .documentation
                .as_ref()
                .map(|documentation| documentation.summary.as_str()),
            Some("Local matrix helper.")
        );

        let (import_line, import_character) = first_position(source, "matrix");
        let import_query = index.query_source_at_navigation(
            &consumer,
            source,
            QueryPosition {
                line: import_line,
                character: import_character,
            },
        );
        assert_eq!(
            import_query
                .definition
                .as_ref()
                .map(|definition| definition.path.as_path()),
            Some(normalize_path(root.join("sage/matrix/constructor.py")).as_path()),
            "the import binding itself should still navigate to the Sage export"
        );
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn hydrate_resolves_materialized_sage_method_cache() {
        let root = test_root("materialized-method-cache");
        fs::create_dir_all(root.join("sage/matrix")).unwrap();
        fs::write(
            root.join("sage/matrix/matrix0.pyx"),
            "def rank(self):\n    \"\"\"Return cached matrix rank without broad lookup.\"\"\"\n    return 0\n",
        )
        .unwrap();
        let consumer = root.join("consumer.py");
        let source = "from sage.all import matrix\nmat = matrix([])\nvalue = mat.rank()\n";
        fs::write(&consumer, source).unwrap();
        let options = IndexOptions {
            roots: vec![root.clone()],
            editable_roots: vec![root.clone()],
            exclude_globs: Vec::new(),
            cache_dir: root.join(".cache"),
            enable_pyx: true,
        };
        let mut index = WorkspaceIndex::new(options.clone());
        index.rebuild().unwrap();

        let mut hydrated = WorkspaceIndex::new(options);
        hydrated.hydrate_from_cache().unwrap();
        let (line, character) = member_position(source, "rank");
        let query = hydrated.query_source_at_navigation(
            &consumer,
            source,
            QueryPosition { line, character },
        );
        assert_eq!(query.owner_type.as_deref(), Some("Matrix"));
        assert_eq!(query.resolution_confidence.as_deref(), Some("high"));
        assert_eq!(
            query
                .definition
                .as_ref()
                .map(|definition| definition.path.as_path()),
            Some(normalize_path(root.join("sage/matrix/matrix0.pyx")).as_path())
        );
        assert_eq!(
            query
                .documentation
                .as_ref()
                .map(|documentation| documentation.summary.as_str()),
            Some("Return cached matrix rank without broad lookup.")
        );
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn hydrate_resolves_source_derived_matrix_constructor_methods() {
        let root = test_root("materialized-matrix-constructor-method-cache");
        fs::create_dir_all(root.join("sage/matrix")).unwrap();
        fs::write(
            root.join("sage/matrix/special.py"),
            "from sage.matrix.constructor import matrix\n\
def matrix_method(func=None, name=None):\n\
    return func\n\
\n\
@matrix_method\n\
def random_matrix(ring, nrows, ncols=None):\n\
    \"\"\"Return a random matrix from a matrix constructor method.\"\"\"\n\
    return matrix([])\n\
\n\
@matrix_method(name='unit')\n\
def identity_matrix(ring, n=0):\n\
    \"\"\"Return an identity matrix from an explicit alias.\"\"\"\n\
    return matrix([])\n",
        )
        .unwrap();
        fs::write(
            root.join("sage/matrix/constructor.py"),
            "def matrix(entries=None):\n    \"\"\"Build a matrix.\"\"\"\n    return entries\n",
        )
        .unwrap();
        let consumer = root.join("consumer.sage");
        let source = "A = matrix.random(GF(2), 3)\nB = matrix.unit(GF(2), 3)\n";
        fs::write(&consumer, source).unwrap();
        let options = IndexOptions {
            roots: vec![root.clone()],
            editable_roots: vec![root.clone()],
            exclude_globs: Vec::new(),
            cache_dir: root.join(".cache"),
            enable_pyx: true,
        };
        let mut index = WorkspaceIndex::new(options.clone());
        index.rebuild().unwrap();

        let mut hydrated = WorkspaceIndex::new(options);
        hydrated.hydrate_from_cache().unwrap();
        for (needle, expected_doc) in [
            (
                "random",
                "Return a random matrix from a matrix constructor method.",
            ),
            ("unit", "Return an identity matrix from an explicit alias."),
        ] {
            let (line, character) = member_position(source, needle);
            let query = hydrated.query_source_at_navigation(
                &consumer,
                source,
                QueryPosition { line, character },
            );
            assert_eq!(query.owner_type.as_deref(), Some("MatrixConstructor"));
            assert_eq!(query.resolution_confidence.as_deref(), Some("high"));
            assert_eq!(
                query
                    .definition
                    .as_ref()
                    .map(|definition| definition.path.as_path()),
                Some(normalize_path(root.join("sage/matrix/special.py")).as_path())
            );
            assert_eq!(
                query
                    .documentation
                    .as_ref()
                    .map(|documentation| documentation.summary.as_str()),
                Some(expected_doc)
            );
        }
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn hydrate_resolves_source_derived_sage_methods_without_static_spec() {
        let root = test_root("source-derived-method-cache");
        fs::create_dir_all(root.join("sage/graphs")).unwrap();
        fs::write(
            root.join("sage/graphs/generic_graph.py"),
            "class GenericGraph:\n    def chromatic_polynomial(self, algorithm=None):\n        \"\"\"Return the graph chromatic polynomial.\"\"\"\n        return None\n",
        )
        .unwrap();
        let consumer = root.join("consumer.py");
        let source =
            "from sage.all import Graph\nG = Graph()\npoly = G.chromatic_polynomial()\nG.chroma\n";
        fs::write(&consumer, source).unwrap();
        let options = IndexOptions {
            roots: vec![root.clone()],
            editable_roots: vec![root.clone()],
            exclude_globs: Vec::new(),
            cache_dir: root.join(".cache"),
            enable_pyx: true,
        };
        let mut index = WorkspaceIndex::new(options.clone());
        index.rebuild().unwrap();
        let rebuilt_status = index.status();
        assert_eq!(
            rebuilt_status.source_derived_method_cache_count, 1,
            "new Sage methods should be counted as source-derived cache rows"
        );
        assert_eq!(rebuilt_status.static_method_cache_count, 0);

        let mut hydrated = WorkspaceIndex::new(options);
        hydrated.hydrate_from_cache().unwrap();
        let (line, character) = member_position(source, "chromatic_polynomial");
        let query = hydrated.query_source_at_navigation(
            &consumer,
            source,
            QueryPosition { line, character },
        );
        assert_eq!(query.owner_type.as_deref(), Some("Graph"));
        assert_eq!(query.resolution_confidence.as_deref(), Some("high"));
        assert_eq!(
            query
                .definition
                .as_ref()
                .map(|definition| definition.path.as_path()),
            Some(normalize_path(root.join("sage/graphs/generic_graph.py")).as_path())
        );
        assert_eq!(
            query
                .documentation
                .as_ref()
                .map(|documentation| documentation.summary.as_str()),
            Some("Return the graph chromatic polynomial.")
        );

        let (completion_line, completion_character) = first_position(source, "G.chroma");
        let completion_position = QueryPosition {
            line: completion_line,
            character: completion_character + "G.chroma".len() as u32,
        };
        let labels: Vec<_> = hydrated
            .completion_items_at_source(source, completion_position, 20)
            .into_iter()
            .map(|item| item.label)
            .collect();
        assert!(
            labels.contains(&"chromatic_polynomial".to_string()),
            "source-derived method completion missing: {labels:?}"
        );
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn hydrate_resolves_source_derived_class_method_aliases() {
        let root = test_root("source-derived-method-alias-cache");
        fs::create_dir_all(root.join("sage/matrix")).unwrap();
        fs::write(
            root.join("sage/matrix/future.py"),
            "class MatrixFuture:\n    def trace_impl(self, algorithm=None):\n        \"\"\"Return aliased matrix trace docs.\"\"\"\n        return None\n\n    trace_alias = trace_impl\n\n    def helper(self):\n        hidden_alias = trace_impl\n        return hidden_alias()\n",
        )
        .unwrap();
        let consumer = root.join("consumer.py");
        let source = "mat = matrix([])\nvalue = mat.trace_alias()\nmat.trace_\n";
        fs::write(&consumer, source).unwrap();
        let options = IndexOptions {
            roots: vec![root.clone()],
            editable_roots: vec![root.clone()],
            exclude_globs: Vec::new(),
            cache_dir: root.join(".cache"),
            enable_pyx: true,
        };
        let mut index = WorkspaceIndex::new(options.clone());
        index.rebuild().unwrap();

        let mut hydrated = WorkspaceIndex::new(options);
        hydrated.hydrate_from_cache().unwrap();
        let (line, character) = member_position(source, "trace_alias");
        let query = hydrated.query_source_at_navigation(
            &consumer,
            source,
            QueryPosition { line, character },
        );
        assert_eq!(query.owner_type.as_deref(), Some("Matrix"));
        assert_eq!(query.resolution_confidence.as_deref(), Some("high"));
        assert_eq!(
            query
                .definition
                .as_ref()
                .map(|definition| definition.name.as_str()),
            Some("trace_impl")
        );
        assert_eq!(
            query
                .definition
                .as_ref()
                .map(|definition| definition.path.as_path()),
            Some(normalize_path(root.join("sage/matrix/future.py")).as_path())
        );
        assert_eq!(
            query
                .documentation
                .as_ref()
                .map(|documentation| documentation.summary.as_str()),
            Some("Return aliased matrix trace docs.")
        );

        let (completion_line, completion_character) = first_position(source, "mat.trace_");
        let completion_position = QueryPosition {
            line: completion_line,
            character: completion_character + "mat.trace_".len() as u32,
        };
        let completions = hydrated.completion_items_at_source(source, completion_position, 20);
        assert!(
            completions
                .iter()
                .any(|item| item.label == "trace_alias"
                    && item.detail.contains("alias for trace_impl")),
            "source-derived method alias completion missing: {completions:?}"
        );
        assert!(
            completions.iter().all(|item| item.label != "hidden_alias"),
            "function-local aliases must not enter the method cache: {completions:?}"
        );
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn hydrate_resolves_class_derived_sage_methods_outside_module_buckets() {
        let root = test_root("class-derived-method-cache");
        fs::create_dir_all(root.join("sage/future")).unwrap();
        fs::write(
            root.join("sage/future/graph_algorithms.py"),
            "class FutureGraphAlgorithms:\n    def experimental_walks(self, limit=None):\n        \"\"\"Return experimental graph walks.\"\"\"\n        return []\n",
        )
        .unwrap();
        let consumer = root.join("consumer.py");
        let source = "from sage.all import Graph\nG = Graph()\nwalks = G.experimental_walks()\nG.experimental_\n";
        fs::write(&consumer, source).unwrap();
        let options = IndexOptions {
            roots: vec![root.clone()],
            editable_roots: vec![root.clone()],
            exclude_globs: Vec::new(),
            cache_dir: root.join(".cache"),
            enable_pyx: true,
        };
        let mut index = WorkspaceIndex::new(options.clone());
        index.rebuild().unwrap();

        let parsed = parse_source(
            "sage.future.graph_algorithms",
            &root.join("sage/future/graph_algorithms.py"),
            &fs::read_to_string(root.join("sage/future/graph_algorithms.py")).unwrap(),
        );
        assert!(parsed.symbols.iter().any(|symbol| {
            symbol.name == "experimental_walks"
                && symbol.detail == "Method FutureGraphAlgorithms.experimental_walks"
        }));

        let mut hydrated = WorkspaceIndex::new(options);
        hydrated.hydrate_from_cache().unwrap();
        let (line, character) = member_position(source, "experimental_walks");
        let query = hydrated.query_source_at_navigation(
            &consumer,
            source,
            QueryPosition { line, character },
        );
        assert_eq!(query.owner_type.as_deref(), Some("Graph"));
        assert_eq!(query.resolution_confidence.as_deref(), Some("high"));
        assert_eq!(
            query
                .definition
                .as_ref()
                .map(|definition| definition.path.as_path()),
            Some(normalize_path(root.join("sage/future/graph_algorithms.py")).as_path())
        );
        assert_eq!(
            query
                .documentation
                .as_ref()
                .map(|documentation| documentation.summary.as_str()),
            Some("Return experimental graph walks.")
        );

        let (completion_line, completion_character) = first_position(source, "G.experimental_");
        let completion_position = QueryPosition {
            line: completion_line,
            character: completion_character + "G.experimental_".len() as u32,
        };
        let labels: Vec<_> = hydrated
            .completion_items_at_source(source, completion_position, 20)
            .into_iter()
            .map(|item| item.label)
            .collect();
        assert_eq!(
            labels.first().map(String::as_str),
            Some("experimental_walks")
        );
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn runtime_documentation_writeback_survives_hydrate() {
        let root = test_root("runtime-doc-writeback");
        fs::write(
            root.join("provider.py"),
            "def undocumented():\n    return 1\n",
        )
        .unwrap();
        let options = IndexOptions {
            roots: vec![root.clone()],
            editable_roots: Vec::new(),
            exclude_globs: Vec::new(),
            cache_dir: root.join(".cache"),
            enable_pyx: true,
        };
        let mut index = WorkspaceIndex::new(options.clone());
        index.rebuild().unwrap();

        assert!(index
            .documentation_for_symbol("undocumented")
            .and_then(|documentation| documentation.docstring)
            .is_none());
        index
            .write_runtime_documentation(
                "undocumented",
                &DocumentationRecord {
                    name: "undocumented".to_string(),
                    module_name: "runtime.provider".to_string(),
                    kind: "function".to_string(),
                    detail: "undocumented()".to_string(),
                    summary: "Runtime generated docs.".to_string(),
                    docstring: Some("Runtime generated docs.\n\nFull runtime body.".to_string()),
                    uri: Some(root.join("provider.py").display().to_string()),
                    markers: vec!["runtime".to_string()],
                    sections: Vec::new(),
                },
            )
            .unwrap();

        let mut hydrated = WorkspaceIndex::new(options);
        hydrated.hydrate_from_cache().unwrap();
        let documentation = hydrated
            .documentation_for_symbol("undocumented")
            .expect("runtime documentation should be readable after hydrate");
        assert_eq!(documentation.summary, "Runtime generated docs.");
        assert_eq!(
            documentation.docstring.as_deref(),
            Some("Runtime generated docs.\n\nFull runtime body.")
        );
        assert!(documentation
            .markers
            .iter()
            .any(|marker| marker == "runtime-writeback"));
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn runtime_documentation_writeback_covers_runtime_only_symbols() {
        let root = test_root("runtime-doc-runtime-only");
        let options = IndexOptions {
            roots: vec![root.clone()],
            editable_roots: Vec::new(),
            exclude_globs: Vec::new(),
            cache_dir: root.join(".cache"),
            enable_pyx: true,
        };
        let mut index = WorkspaceIndex::new(options.clone());
        index.rebuild().unwrap();
        index
            .write_runtime_documentation(
                "FutureRuntimeOnly",
                &DocumentationRecord {
                    name: "FutureRuntimeOnly".to_string(),
                    module_name: "sage.runtime".to_string(),
                    kind: "type".to_string(),
                    detail: "FutureRuntimeOnly".to_string(),
                    summary: "Runtime-only Sage API docs.".to_string(),
                    docstring: Some("Runtime-only Sage API docs.".to_string()),
                    uri: None,
                    markers: vec!["runtime".to_string()],
                    sections: Vec::new(),
                },
            )
            .unwrap();

        let mut hydrated = WorkspaceIndex::new(options);
        hydrated.hydrate_from_cache().unwrap();
        assert_eq!(
            hydrated
                .documentation_for_symbol("FutureRuntimeOnly")
                .map(|documentation| documentation.summary),
            Some("Runtime-only Sage API docs.".to_string())
        );
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn reconcile_prewarms_hot_sage_symbols_and_method_docs() {
        let root = test_root("hot-symbol-hydrate");
        fs::create_dir_all(root.join("sage/rings/polynomial")).unwrap();
        fs::create_dir_all(root.join("sage/matrix")).unwrap();
        fs::write(
            root.join("sage/rings/polynomial/polynomial_ring_constructor.py"),
            "def PolynomialRing(*args):\n    \"\"\"Construct a cached polynomial ring.\"\"\"\n    return args\n",
        )
        .unwrap();
        fs::write(
            root.join("sage/matrix/matrix0.pyx"),
            "def rank(self):\n    \"\"\"Return cached matrix rank.\"\"\"\n    return 0\n",
        )
        .unwrap();
        let consumer = root.join("consumer.py");
        let source = "from sage.all import matrix\nmat = matrix([])\nvalue = mat.rank()\n";
        fs::write(&consumer, source).unwrap();
        let options = IndexOptions {
            roots: vec![root.clone()],
            editable_roots: vec![root.clone()],
            exclude_globs: Vec::new(),
            cache_dir: root.join(".cache"),
            enable_pyx: true,
        };
        let mut index = WorkspaceIndex::new(options.clone());
        index.rebuild().unwrap();
        let mut hydrated = WorkspaceIndex::new(options);
        hydrated.hydrate_from_cache().unwrap();
        hydrated.reconcile_with_cache().unwrap();

        assert!(
            hydrated.status().hot_symbol_cache_count >= hot_sage_symbol_names().len(),
            "hot cache should be populated after hydrate: {:?}",
            hydrated.status()
        );
        assert_eq!(
            hydrated
                .documentation_for_symbol("PolynomialRing")
                .map(|docs| docs.summary),
            Some("Construct a cached polynomial ring.".to_string())
        );
        let (line, character) = member_position(source, "rank");
        let query = hydrated.query_source_at_navigation(
            &consumer,
            source,
            QueryPosition { line, character },
        );
        assert_eq!(
            query
                .documentation
                .as_ref()
                .map(|docs| docs.summary.as_str()),
            Some("Return cached matrix rank.")
        );
        assert_eq!(
            query
                .definition
                .as_ref()
                .map(|definition| definition.path.as_path()),
            Some(normalize_path(root.join("sage/matrix/matrix0.pyx")).as_path())
        );
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn fast_reconcile_keeps_indexed_all_py_exports_queryable_for_future_sage_apis() {
        let root = test_root("dynamic-hot-all-exports");
        fs::create_dir_all(root.join("sage/future")).unwrap();
        fs::write(
            root.join("sage/all.py"),
            "from sage.future.all import FuturePublic\n",
        )
        .unwrap();
        fs::write(
            root.join("sage/future/all.py"),
            "from sage.future.module import FutureImplementation as FuturePublic\n",
        )
        .unwrap();
        fs::write(
            root.join("sage/future/module.py"),
            "def FutureImplementation(*args):\n    \"\"\"Future Sage API docs.\"\"\"\n    return args\n",
        )
        .unwrap();
        let consumer = root.join("consumer.py");
        let source = "from sage.all import FuturePublic\nvalue = FuturePublic()\n";
        fs::write(&consumer, source).unwrap();
        let options = IndexOptions {
            roots: vec![root.clone()],
            editable_roots: vec![root.clone()],
            exclude_globs: Vec::new(),
            cache_dir: root.join(".cache"),
            enable_pyx: true,
        };
        let mut index = WorkspaceIndex::new(options.clone());
        index.rebuild().unwrap();
        let mut hydrated = WorkspaceIndex::new(options);
        hydrated.hydrate_from_cache().unwrap();
        hydrated.reconcile_with_cache().unwrap();

        assert!(
            matches!(
                hydrated.status().last_operation.as_deref(),
                Some("fast-reconcile")
            ),
            "{:?}",
            hydrated.status()
        );

        let query =
            hydrated.query_source_symbol(&consumer, source, "FuturePublic", None, None, Vec::new());
        assert_eq!(
            query
                .definition
                .as_ref()
                .map(|definition| definition.path.as_path()),
            Some(normalize_path(root.join("sage/future/module.py")).as_path())
        );
        assert_eq!(
            query
                .documentation
                .as_ref()
                .map(|documentation| documentation.summary.as_str()),
            Some("Future Sage API docs.")
        );
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn root_aware_cache_reuses_same_root_set_and_isolates_other_projects() {
        let root = test_root("shared-cache");
        let cache_dir = root.join(".cache");
        let shared = root.join("shared-sage-root");
        let project_a = root.join("project-a");
        let project_b = root.join("project-b");
        fs::create_dir_all(&shared).unwrap();
        fs::create_dir_all(&project_a).unwrap();
        fs::create_dir_all(&project_b).unwrap();
        fs::write(
            shared.join("common.py"),
            "def common_symbol():\n    \"\"\"Shared docs.\"\"\"\n    return 1\n",
        )
        .unwrap();
        fs::write(
            project_a.join("a.sage"),
            "from common import common_symbol\n",
        )
        .unwrap();
        fs::write(
            project_b.join("b.sage"),
            "from common import common_symbol\n",
        )
        .unwrap();

        let mut first = WorkspaceIndex::new(IndexOptions {
            roots: vec![project_a.clone(), shared.clone()],
            editable_roots: vec![project_a.clone()],
            exclude_globs: Vec::new(),
            cache_dir: cache_dir.clone(),
            enable_pyx: true,
        });
        first.reconcile_with_cache().unwrap();
        assert_eq!(first.status().cache_miss_count, 2);

        let mut same_roots = WorkspaceIndex::new(IndexOptions {
            roots: vec![project_a.clone(), shared.clone()],
            editable_roots: vec![project_a.clone()],
            exclude_globs: Vec::new(),
            cache_dir: cache_dir.clone(),
            enable_pyx: true,
        });
        assert_eq!(first.db_path(), same_roots.db_path());
        same_roots.hydrate_from_cache().unwrap();
        same_roots.reconcile_with_cache().unwrap();
        let same_status = same_roots.status();
        assert!(same_status.cache_hit_count >= 2, "{same_status:?}");
        assert_eq!(same_status.cache_miss_count, 0, "{same_status:?}");
        assert!(same_roots.symbol("common_symbol").is_some());
        assert!(same_roots
            .file_for_path(&project_a.join("a.sage"))
            .is_some());

        let mut other_project = WorkspaceIndex::new(IndexOptions {
            roots: vec![project_b.clone(), shared.clone()],
            editable_roots: vec![project_b.clone()],
            exclude_globs: Vec::new(),
            cache_dir,
            enable_pyx: true,
        });
        assert_ne!(first.db_path(), other_project.db_path());
        let cold_hydrate = other_project.hydrate_from_cache().unwrap();
        assert_eq!(cold_hydrate.peer_seed_file_count, 0, "{cold_hydrate:?}");
        assert_eq!(cold_hydrate.cache_miss_count, 1, "{cold_hydrate:?}");
        other_project.reconcile_with_cache().unwrap();

        let status = other_project.status();
        assert!(status.cache_hit_count >= 1, "{status:?}");
        assert_eq!(status.cache_miss_count, 2, "{status:?}");
        assert!(status.peer_seed_file_count >= 1, "{status:?}");
        assert!(other_project.symbol("common_symbol").is_some());
        assert!(other_project
            .file_for_path(&project_a.join("a.sage"))
            .is_none());
        assert!(other_project
            .file_for_path(&project_b.join("b.sage"))
            .is_some());
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn cached_lookup_survives_single_file_refresh_overlay() {
        let root = test_root("cache-overlay");
        let src = root.join("src");
        fs::create_dir_all(&src).unwrap();
        let bridge = src.join("cythonish_bridge.pyx");
        let declaration = src.join("native_support.pxd");
        let source = "from native_support cimport NativeAccumulator\n\ncdef class StepCounter(NativeAccumulator):\n    pass\n";
        fs::write(&bridge, source).unwrap();
        fs::write(&declaration, "cdef class NativeAccumulator:\n    pass\n").unwrap();
        let options = IndexOptions {
            roots: vec![root.clone()],
            editable_roots: vec![root.clone()],
            exclude_globs: Vec::new(),
            cache_dir: root.join(".cache"),
            enable_pyx: true,
        };
        WorkspaceIndex::new(options.clone())
            .reconcile_with_cache()
            .unwrap();

        let mut reopened = WorkspaceIndex::new(options);
        reopened.hydrate_from_cache().unwrap();
        reopened
            .refresh_paths(std::slice::from_ref(&bridge), &[])
            .unwrap();

        let query = reopened.query_source_symbol(
            &bridge,
            source,
            "NativeAccumulator",
            None,
            None,
            Vec::new(),
        );
        assert_eq!(
            query
                .definition
                .as_ref()
                .map(|definition| definition.path.as_path()),
            Some(normalize_path(declaration.clone()).as_path())
        );
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn cache_falls_back_when_configured_cache_dir_is_unusable() {
        let root = test_root("cache-fallback");
        let source = root.join("docs.py");
        fs::write(&source, "def fallback_symbol():\n    return 1\n").unwrap();
        let unusable_cache_path = root.join("not-a-directory");
        fs::write(&unusable_cache_path, "blocks cache directory creation").unwrap();
        let mut index = WorkspaceIndex::new(IndexOptions {
            roots: vec![root.clone()],
            editable_roots: Vec::new(),
            exclude_globs: Vec::new(),
            cache_dir: unusable_cache_path.clone(),
            enable_pyx: true,
        });

        let status = index.rebuild().unwrap();

        assert!(status.last_error.is_none());
        assert_ne!(
            Path::new(&status.cache_path).parent(),
            Some(unusable_cache_path.as_path())
        );
        assert!(Path::new(&status.cache_path).exists());
        assert!(index.symbol("fallback_symbol").is_some());
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn hydrate_missing_configured_cache_stays_on_configured_path() {
        let root = test_root("cache-hydrate-missing-primary");
        let configured_cache = root.join(".cache");
        let mut index = WorkspaceIndex::new(IndexOptions {
            roots: vec![root.clone()],
            editable_roots: Vec::new(),
            exclude_globs: Vec::new(),
            cache_dir: configured_cache.clone(),
            enable_pyx: true,
        });

        let status = index.hydrate_from_cache().unwrap();

        assert_eq!(
            Path::new(&status.cache_path).parent(),
            Some(configured_cache.as_path())
        );
        assert_eq!(status.cache_miss_count, 1);
        assert!(configured_cache.exists());
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn refresh_paths_updates_and_deletes_files() {
        let root = test_root("refresh");
        let source = root.join("module.py");
        fs::write(&source, "def before():\n    return 1\n").unwrap();
        let mut index = WorkspaceIndex::new(IndexOptions {
            roots: vec![root.clone()],
            editable_roots: Vec::new(),
            exclude_globs: Vec::new(),
            cache_dir: root.join(".cache"),
            enable_pyx: true,
        });
        index.rebuild().unwrap();
        fs::write(&source, "def after():\n    return 2\n").unwrap();
        index
            .refresh_paths(std::slice::from_ref(&source), &[])
            .unwrap();
        assert!(index.symbol("after").is_some());
        assert!(index.symbol("before").is_none());
        fs::remove_file(&source).unwrap();
        index
            .refresh_paths(&[], std::slice::from_ref(&source))
            .unwrap();
        assert!(index.symbol("after").is_none());
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn preload_paths_warms_symbols_without_advancing_generation() {
        let root = test_root("preload");
        let source = root.join("local_docs.py");
        fs::write(&source, "class PolynomialNotebook:\n    pass\n").unwrap();
        let mut index = WorkspaceIndex::new(IndexOptions {
            roots: vec![root.clone()],
            editable_roots: Vec::new(),
            exclude_globs: Vec::new(),
            cache_dir: root.join(".cache"),
            enable_pyx: true,
        });
        let generation = index.status().generation;

        assert_eq!(index.preload_paths(std::slice::from_ref(&source)), 1);

        assert_eq!(index.status().generation, generation);
        assert!(index.symbol("PolynomialNotebook").is_some());
        let canonical_source = source.canonicalize().unwrap();
        assert_eq!(
            index.source_path_for_module("local_docs").as_deref(),
            Some(canonical_source.as_path())
        );
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn collect_indexable_paths_limits_installed_site_packages_to_sage_package() {
        let root = test_root("site-packages-scan");
        let site_packages = root.join("local/lib/python3.14/site-packages");
        let sage_module = site_packages.join("sage/matrix/matrix0.pyx");
        let dependency_module = site_packages.join("numpy/core.py");
        fs::create_dir_all(sage_module.parent().unwrap()).unwrap();
        fs::create_dir_all(dependency_module.parent().unwrap()).unwrap();
        fs::write(&sage_module, "def rank(self):\n    pass\n").unwrap();
        fs::write(&dependency_module, "def array():\n    pass\n").unwrap();

        let paths = collect_indexable_paths(&IndexOptions {
            roots: vec![site_packages],
            editable_roots: Vec::new(),
            exclude_globs: Vec::new(),
            cache_dir: root.join(".cache"),
            enable_pyx: true,
        });

        assert_eq!(paths, vec![sage_module]);
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn sage_prewarm_modules_cover_sage_heavy_method_paths() {
        let modules = sage_prewarm_modules_for_source(
            "from sage.all import GF, PolynomialRing, matrix\nmat = matrix(GF(7), 2, 2)\nrank = mat.rank()\nR = PolynomialRing(GF(7), 'x')\nI = R.ideal([])\n",
        );

        assert!(modules.contains(&"sage.matrix.matrix0"));
        assert!(modules.contains(&"sage.matrix.matrix2"));
        assert!(modules.contains(&"sage.rings.polynomial.multi_polynomial_libsingular"));
        assert!(modules.contains(&"sage.rings.polynomial.multi_polynomial_ideal"));
    }

    #[test]
    fn prewarmed_sage_method_modules_resolve_before_full_rebuild() {
        let root = test_root("method-prewarm");
        let site_packages = root.join("local/lib/python3.14/site-packages");
        let matrix0 = site_packages.join("sage/matrix/matrix0.pyx");
        let consumer = root.join("workspace/demo.py");
        fs::create_dir_all(matrix0.parent().unwrap()).unwrap();
        fs::create_dir_all(consumer.parent().unwrap()).unwrap();
        fs::write(
            &matrix0,
            "def rank(self):\n    \"\"\"Return the rank of this matrix.\"\"\"\n    return 0\n",
        )
        .unwrap();
        let source = "from sage.all import matrix\nmat = matrix([])\nvalue = mat.rank()\n";
        fs::write(&consumer, source).unwrap();
        let mut index = WorkspaceIndex::new(IndexOptions {
            roots: vec![consumer.parent().unwrap().to_path_buf(), site_packages],
            editable_roots: vec![consumer.parent().unwrap().to_path_buf()],
            exclude_globs: Vec::new(),
            cache_dir: root.join(".cache"),
            enable_pyx: true,
        });
        let prewarm_targets: Vec<_> = sage_prewarm_modules_for_source(source)
            .into_iter()
            .filter_map(|module| index.source_path_for_module(module))
            .collect();

        index.preload_paths(&prewarm_targets);
        let (line, character) = member_position(source, "rank");
        let query =
            index.query_source_at_navigation(&consumer, source, QueryPosition { line, character });

        assert_eq!(
            query
                .documentation
                .as_ref()
                .map(|docs| docs.summary.as_str()),
            Some("Return the rank of this matrix.")
        );
        assert_eq!(query.resolution_confidence.as_deref(), Some("high"));
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn diagnostics_report_incomplete_sage_caret() {
        let diagnostics = diagnostics_for_source(Path::new("demo.sage"), "value = 2^\n");
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].code, "syntax-error");
        assert_eq!(diagnostics[0].severity, "error");
        assert_eq!(diagnostics[0].range.start_character, 9);
    }

    #[test]
    fn diagnostics_warn_for_sage_caret_exponents_in_python_only() {
        let source = [
            "from sage.all import PolynomialRing, QQ",
            "R = PolynomialRing(QQ, 'x')",
            "value = x^2 + 1",
            "text = 'x^2'",
            "# y^3 stays a comment",
        ]
        .join("\n");
        let diagnostics = diagnostics_for_source(Path::new("demo.py"), &source);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        assert_eq!(diagnostics[0].code, "sage-python-caret-exponent");
        assert_eq!(diagnostics[0].severity, "warning");
        assert_eq!(diagnostics[0].range.start_line, 2);
        assert_eq!(diagnostics[0].range.start_character, 9);

        let ordinary_python =
            diagnostics_for_source(Path::new("ordinary.py"), "value = flags ^ mask\n");
        assert!(ordinary_python.is_empty(), "{ordinary_python:?}");

        let sage_source = diagnostics_for_source(Path::new("demo.sage"), "value = x^2 + 1\n");
        assert!(sage_source.is_empty(), "{sage_source:?}");
    }

    #[test]
    fn diagnostics_allow_sage_range_syntax() {
        let diagnostics =
            diagnostics_for_source(Path::new("demo.sage"), "values = [n^2 for n in [1..5]]\n");
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }

    #[test]
    fn diagnostics_allow_preparser_assignment_rhs_operators() {
        let source = "K.<i> = NumberField(w^2 + 1)\nF.<a> = GF(2^8, name=\"a\")\nS.<Y> = Kfun[]\n";
        let diagnostics = diagnostics_for_source(Path::new("demo.sage"), source);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }

    #[test]
    fn function_call_context_tracks_keyword_arguments() {
        let source = "result = trace_window(w^2 + 3*w + 1, width=7)\n";
        let character = source.find("width").unwrap() as u32 + 2;
        assert_eq!(
            function_call_at_position(source, 0, character),
            Some(("trace_window".to_string(), 1))
        );
    }

    #[test]
    fn function_call_context_ignores_nested_tuple_commas() {
        let source = "quotient_ring = R.quotient(I, names=(\"xb\", \"yb\", \"zb\"))\n";
        let character = source.find("\"yb\"").unwrap() as u32 + 2;
        assert_eq!(
            function_call_at_position(source, 0, character),
            Some(("quotient".to_string(), 1))
        );
    }

    #[test]
    fn function_call_context_spans_multiline_calls() {
        let source = "result = trace_window(\n    w^2 + 1,\n    width=7,\n)\n";
        let character = source.lines().nth(2).unwrap().find("width").unwrap() as u32 + 2;
        assert_eq!(
            function_call_at_position(source, 2, character),
            Some(("trace_window".to_string(), 1))
        );
    }

    #[test]
    fn cython_declaration_signature_does_not_require_colon() {
        let file = parse_source(
            "native_support",
            Path::new("native_support.pxd"),
            "cpdef int native_step(int value)\n",
        );
        assert_eq!(
            file.symbols
                .iter()
                .find(|symbol| symbol.name == "native_step")
                .and_then(|symbol| symbol.signature.as_deref()),
            Some("native_step(int value)")
        );
    }

    #[test]
    fn references_skip_strings_and_comments() {
        let refs = references_in_source(
            Path::new("demo.py"),
            "target()\ntext = 'target'\n# target\n",
            "target",
        );
        assert_eq!(refs.len(), 1);
    }

    #[test]
    fn code_reference_range_checks_one_token_without_full_reference_scan() {
        let source = "target()\ntext = 'target'\n# target\nother_target()\n";
        assert!(is_code_reference_at_range(
            source,
            "target",
            &SourceRange {
                start_line: 0,
                start_character: 0,
                end_line: 0,
                end_character: 6,
            },
        ));
        assert!(!is_code_reference_at_range(
            source,
            "target",
            &SourceRange {
                start_line: 1,
                start_character: 8,
                end_line: 1,
                end_character: 14,
            },
        ));
        assert!(!is_code_reference_at_range(
            source,
            "target",
            &SourceRange {
                start_line: 3,
                start_character: 6,
                end_line: 3,
                end_character: 12,
            },
        ));
    }

    #[test]
    fn semantic_spans_include_sage_domains() {
        let spans = semantic_spans("R.<x> = PolynomialRing(QQ)\nvalue = PolynomialRing(QQ)\n@cached_method\ndef f():\n    local_value = 2\n    return graphs.PetersenGraph()\n");
        assert!(spans.iter().any(|span| span.token_type == "type"));
        assert!(spans.iter().any(|span| span.token_type == "namespace"));
        assert!(spans.iter().any(|span| span.token_type == "parameter"));
        assert!(spans.iter().any(|span| span.token_type == "decorator"));
        assert!(spans.iter().any(|span| span.line == 1
            && span.start == 0
            && span.length == "value".len() as u32
            && span.token_type == "variable"
            && span
                .modifiers
                .iter()
                .any(|modifier| modifier == "declaration")));
        assert!(spans.iter().any(|span| span.line == 4
            && span.start == 4
            && span.length == "local_value".len() as u32
            && span.token_type == "variable"
            && span
                .modifiers
                .iter()
                .any(|modifier| modifier == "declaration")));
        for pair in spans.windows(2) {
            if pair[0].line == pair[1].line {
                assert!(pair[0].start + pair[0].length <= pair[1].start);
            }
        }
    }

    #[test]
    fn semantic_spans_skip_strings_and_comments() {
        let spans =
            semantic_spans("text = 'PolynomialRing'\n# graphs\nvalue = PolynomialRing(QQ)\n");
        assert_eq!(
            spans
                .iter()
                .filter(|span| span.token_type == "type")
                .count(),
            1
        );
    }
}
