//! Language-server initialization settings and path normalization.

use crate::analysis_mode::ConfiguredAnalysisMode;
use serde::Deserialize;
use serde_json::Value;
use std::path::PathBuf;
use tower_lsp::lsp_types::Url;

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(default)]
pub(super) struct InitializationOptions {
    pub(super) interpreter: InterpreterOptions,
    pub(super) analysis: AnalysisOptions,
    pub(super) workspace: WorkspaceOptions,
    pub(super) documentation: DocumentationOptions,
    pub(super) rust: RustOptions,
    pub(super) pyright: PyrightOptions,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(default)]
pub(super) struct InterpreterOptions {
    pub(super) path: String,
    pub(super) args: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(default)]
pub(super) struct AnalysisOptions {
    pub(super) mode: ConfiguredAnalysisMode,
    pub(super) extra_paths: Vec<String>,
    pub(super) source_roots: Vec<String>,
    pub(super) enable_diagnostics: bool,
    pub(super) enable_runtime_introspection: bool,
    pub(super) enable_pyx_parsing: bool,
}

impl Default for AnalysisOptions {
    fn default() -> Self {
        Self {
            mode: ConfiguredAnalysisMode::default(),
            extra_paths: Vec::new(),
            source_roots: Vec::new(),
            enable_diagnostics: true,
            enable_runtime_introspection: true,
            enable_pyx_parsing: true,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(default)]
pub(super) struct WorkspaceOptions {
    pub(super) folders: Vec<String>,
    pub(super) source_roots: Vec<String>,
    pub(super) exclude: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(default)]
pub(super) struct DocumentationOptions {
    pub(super) preferred_source: String,
    pub(super) show_on_hover: bool,
}

impl Default for DocumentationOptions {
    fn default() -> Self {
        Self {
            preferred_source: "auto".to_string(),
            show_on_hover: true,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum DocumentationPreferredSource {
    Auto,
    Workspace,
    Runtime,
    Reference,
}

impl DocumentationPreferredSource {
    pub(super) fn from_config(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "workspace" => Self::Workspace,
            "runtime" => Self::Runtime,
            "reference" => Self::Reference,
            _ => Self::Auto,
        }
    }

    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Workspace => "workspace",
            Self::Runtime => "runtime",
            Self::Reference => "reference",
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(default)]
pub(super) struct RustOptions {
    pub(super) cache_dir: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(default)]
pub(super) struct PyrightOptions {
    pub(super) node_path: Option<String>,
    pub(super) server_path: Option<String>,
}

pub(super) fn parse_initialization_options(value: Option<Value>) -> InitializationOptions {
    value
        .and_then(|raw| serde_json::from_value(raw).ok())
        .unwrap_or_default()
}

pub(super) fn source_roots_from_options(options: &InitializationOptions) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    let workspace_folders = workspace_folders_from_options(options);
    roots.extend(
        options
            .workspace
            .source_roots
            .iter()
            .filter_map(|entry| uri_or_path(entry)),
    );
    roots.extend(workspace_folders.clone());
    roots.extend(resolve_configured_paths(
        &options.analysis.source_roots,
        &workspace_folders,
    ));
    roots.extend(resolve_configured_paths(
        &options.analysis.extra_paths,
        &workspace_folders,
    ));
    roots.sort();
    roots.dedup();
    roots
}

pub(super) fn workspace_folders_from_options(options: &InitializationOptions) -> Vec<PathBuf> {
    let mut folders: Vec<_> = options
        .workspace
        .folders
        .iter()
        .filter_map(|entry| uri_or_path(entry))
        .collect();
    folders.sort();
    folders.dedup();
    folders
}

fn resolve_configured_paths(values: &[String], workspace_folders: &[PathBuf]) -> Vec<PathBuf> {
    values
        .iter()
        .flat_map(|value| {
            let path = PathBuf::from(value);
            if path.is_absolute() || workspace_folders.is_empty() {
                vec![path]
            } else {
                workspace_folders
                    .iter()
                    .map(|folder| folder.join(value))
                    .collect()
            }
        })
        .collect()
}

fn uri_or_path(value: &str) -> Option<PathBuf> {
    Url::parse(value)
        .ok()
        .and_then(|url| url.to_file_path().ok())
        .or_else(|| Some(PathBuf::from(value)))
}

pub(super) fn default_excludes() -> Vec<String> {
    vec![
        "**/.git/**".to_string(),
        "**/__pycache__/**".to_string(),
        "**/.venv/**".to_string(),
        "**/build/**".to_string(),
        "**/target/**".to_string(),
    ]
}
