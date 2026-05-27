import type {
  DocsStatusSummary,
  EnvironmentPresentationInput,
  IndexStatusSummary,
} from "./environmentPresentation";
import type { SageSettings } from "./settingsModel";

export interface SupportBundleInput {
  generatedAt: string;
  extension: {
    id: string;
    version: string;
  };
  host: {
    vscodeVersion: string;
    platform: string;
    arch: string;
    nodeVersion: string;
  };
  workspace: {
    folders: string[];
    trusted: boolean;
    hasVirtualWorkspace: boolean;
  };
  activeDocument?: {
    uri: string;
    languageId: string;
    scheme: string;
  };
  settings: SageSettings;
  environment: EnvironmentPresentationInput;
  lifecycle: Record<string, boolean | number>;
  indexStatus?: IndexStatusSummary;
  docsStatus?: DocsStatusSummary;
}

export function buildSupportBundle(input: SupportBundleInput): string {
  return JSON.stringify({
    schema_version: 1,
    generated_at: input.generatedAt,
    privacy: {
      includes_source_contents: false,
      includes_selected_text: false,
      includes_environment_variables: false,
      includes_paths_and_settings: true,
    },
    extension: input.extension,
    host: input.host,
    workspace: input.workspace,
    active_document: input.activeDocument,
    settings: {
      interpreter_path: input.settings.interpreterPath,
      interpreter_args: input.settings.interpreterArgs,
      language_server_rust_path: input.settings.languageServerRustPath,
      language_server_python_path: input.settings.languageServerPythonPath,
      language_server_python_args: input.settings.languageServerPythonArgs,
      analysis_mode: input.settings.analysisMode,
      source_roots: input.settings.sourceRoots,
      extra_paths: input.settings.extraPaths,
      diagnostics_enabled: input.settings.diagnosticsEnabled,
      runtime_introspection_enabled: input.settings.runtimeIntrospectionEnabled,
      enable_pyx_parsing: input.settings.enablePyxParsing,
      enable_python_files: input.settings.pythonFilesEnabled,
      indexing_exclude: input.settings.indexingExcludeGlobs,
      docs_source: input.settings.docsSource,
      show_docs_on_hover: input.settings.showDocsOnHover,
      logging_level: input.settings.loggingLevel,
      run_target: input.settings.runTarget,
      show_cell_code_lens: input.settings.showCellCodeLens,
      cleanup_generated_python: input.settings.cleanupGeneratedPython,
      notebook_support_enabled: input.settings.notebookSupportEnabled,
    },
    environment: {
      interpreter_path: input.environment.interpreterPath,
      language_server_path: input.environment.languageServerPath,
      language_server_engine: input.environment.languageServerEngine,
      language_server_starting: input.environment.languageServerStarting ?? false,
      analysis_mode: input.environment.analysisMode,
      docs_source: input.environment.docsSource,
      source_roots: input.environment.sourceRoots,
      extra_paths: input.environment.extraPaths ?? [],
      index_mode: input.environment.indexMode,
      runtime_introspection_enabled: input.environment.runtimeIntrospectionEnabled,
      enable_pyx_parsing: input.environment.enablePyxParsing,
      python_files_enabled: input.environment.pythonFilesEnabled,
      workspace_runtime_state: input.environment.workspaceRuntimeState,
    },
    language_client_lifecycle: input.lifecycle,
    index_status: input.indexStatus ?? input.environment.indexStatus ?? null,
    docs_status: input.docsStatus ?? input.environment.docsStatus ?? null,
  }, null, 2);
}
