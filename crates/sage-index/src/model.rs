use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

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
pub(super) struct SageMethodCacheStats {
    pub(super) total: usize,
    pub(super) source_derived: usize,
    pub(super) static_fallback: usize,
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
    #[serde(default)]
    pub(super) identifier_filter: Vec<u8>,
}

pub(super) type SymbolLookupCache = Arc<Mutex<HashMap<String, Vec<SymbolRecord>>>>;
pub(super) type FileLookupCache = Arc<Mutex<HashMap<PathBuf, IndexedFile>>>;
pub(super) type SageMethodLookupCache = Arc<Mutex<HashMap<(String, String), Option<SymbolRecord>>>>;
pub(super) type ReferenceLookupCache = Arc<Mutex<HashMap<String, Vec<ReferenceRecord>>>>;
pub(super) type IdentifierFilterCache = Arc<Mutex<Option<BTreeMap<PathBuf, Vec<u8>>>>>;
pub(super) type PendingRefreshPaths = Arc<Mutex<BTreeMap<PathBuf, u64>>>;

#[derive(Clone, Debug, Default)]
pub struct WorkspaceIndex {
    pub(super) options: IndexOptions,
    pub(super) db_path: PathBuf,
    pub(super) files: BTreeMap<PathBuf, IndexedFile>,
    pub(super) symbols_by_name: HashMap<String, Vec<SymbolRecord>>,
    pub(super) generation: u64,
    pub(super) last_index_ms: u128,
    pub(super) last_operation: Option<String>,
    pub(super) last_hydrate_ms: u128,
    pub(super) last_reconcile_ms: u128,
    pub(super) last_persist_ms: u128,
    pub(super) last_hot_cache_ms: u128,
    pub(super) last_peer_seed_ms: u128,
    pub(super) peer_seed_file_count: usize,
    pub(super) cache_hit_count: usize,
    pub(super) cache_miss_count: usize,
    pub(super) loaded_roots: Vec<PathBuf>,
    pub(super) last_error: Option<String>,
    pub(super) cached_file_count: usize,
    pub(super) cached_symbol_count: usize,
    pub(super) cached_doc_count: usize,
    pub(super) source_root_fingerprints: Vec<SourceRootFingerprint>,
    pub(super) cached_root_fingerprint_mismatches: Vec<StaleSourceRootFingerprint>,
    pub(super) symbol_lookup_cache: SymbolLookupCache,
    pub(super) file_lookup_cache: FileLookupCache,
    pub(super) sage_method_lookup_cache: SageMethodLookupCache,
    pub(super) reference_lookup_cache: ReferenceLookupCache,
    pub(super) identifier_filter_cache: IdentifierFilterCache,
    pub(super) identifier_filter_cache_was_ready: bool,
    pub(super) pending_identifier_filter_updates: BTreeMap<PathBuf, Option<Vec<u8>>>,
    pub(super) pending_refresh_paths: PendingRefreshPaths,
    pub(super) completed_pending_refresh_versions: BTreeMap<PathBuf, u64>,
    pub(super) defer_pending_refresh_clear: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum SageOwnerType {
    MatrixConstructor,
    Matrix,
    FreeModule,
    PolynomialRing,
    UnivariatePolynomialRing,
    MultivariatePolynomialRing,
    PolynomialElement,
    Ideal,
    Field,
    FieldElement,
    Vector,
    Graph,
    EllipticCurve,
    NumberField,
    NumberFieldElement,
    Polyhedron,
}

impl SageOwnerType {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::MatrixConstructor => "MatrixConstructor",
            Self::Matrix => "Matrix",
            Self::FreeModule => "FreeModule",
            Self::PolynomialRing => "PolynomialRing",
            Self::UnivariatePolynomialRing => "UnivariatePolynomialRing",
            Self::MultivariatePolynomialRing => "MultivariatePolynomialRing",
            Self::PolynomialElement => "PolynomialElement",
            Self::Ideal => "Ideal",
            Self::Field => "Field",
            Self::FieldElement => "FieldElement",
            Self::Vector => "Vector",
            Self::Graph => "Graph",
            Self::EllipticCurve => "EllipticCurve",
            Self::NumberField => "NumberField",
            Self::NumberFieldElement => "NumberFieldElement",
            Self::Polyhedron => "Polyhedron",
        }
    }
}

pub(super) fn sage_method_cache_key(owner_type: SageOwnerType, member: &str) -> (String, String) {
    (owner_type.as_str().to_string(), member.to_ascii_lowercase())
}

#[derive(Clone, Debug)]
pub(super) struct MemberResolution {
    pub(super) record: Option<SymbolRecord>,
    pub(super) candidates: Vec<SymbolRecord>,
    pub(super) owner_type: Option<SageOwnerType>,
    pub(super) confidence: &'static str,
    pub(super) reason: String,
    pub(super) candidate_count: usize,
    pub(super) suppress_global_fallback: bool,
}

#[derive(Clone, Debug)]
pub(super) struct SageExportResolution {
    pub(super) record: SymbolRecord,
    pub(super) reason: &'static str,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct SourceDerivedMethodOwner {
    pub(super) owner_type: SageOwnerType,
    pub(super) priority: u8,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct SageMethodSpec {
    pub(super) owner_type: SageOwnerType,
    pub(super) member: &'static str,
    pub(super) module: &'static str,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct SageMethodAliasSpec {
    pub(super) owner_type: SageOwnerType,
    pub(super) member: &'static str,
    pub(super) source_name: &'static str,
    pub(super) module: &'static str,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct SageOwnerModuleSpec {
    pub(super) owner_type: SageOwnerType,
    pub(super) module: &'static str,
    pub(super) recursive: bool,
    pub(super) priority: u8,
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

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct QueryDefinitionCandidate {
    pub definition: QueryDefinition,
    pub confidence: String,
    pub reason: String,
    pub signature: Option<String>,
    pub summary: Option<String>,
}

/// Selects which physical source role a navigation query should prefer.
///
/// Role-specific queries may return different definitions and candidates for
/// the same document position, so callers must include this value in any query
/// cache key.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub enum NavigationTargetRole {
    #[default]
    Definition,
    Declaration,
    Implementation,
}

pub(super) fn query_definition_from_record(record: &SymbolRecord) -> Option<QueryDefinition> {
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
    #[serde(default, rename = "definitionCandidates")]
    pub definition_candidates: Vec<QueryDefinitionCandidate>,
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
    pub presentation: bool,
}

impl QueryFeatures {
    pub const fn full() -> Self {
        Self {
            completions: true,
            references: true,
            rename_preview: true,
            signature: true,
            diagnostics: true,
            presentation: true,
        }
    }

    pub const fn navigation() -> Self {
        Self {
            completions: false,
            references: false,
            rename_preview: false,
            signature: true,
            diagnostics: false,
            presentation: true,
        }
    }

    pub const fn hover() -> Self {
        Self {
            completions: false,
            references: false,
            rename_preview: false,
            signature: false,
            diagnostics: false,
            presentation: true,
        }
    }

    pub const fn definition_only() -> Self {
        Self {
            completions: false,
            references: false,
            rename_preview: false,
            signature: false,
            diagnostics: false,
            presentation: false,
        }
    }
}

#[derive(Clone, Debug)]
pub struct QueryExecutionOptions<'a> {
    pub rename_to: Option<&'a str>,
    pub diagnostics: Vec<DiagnosticRecord>,
    pub features: QueryFeatures,
}
