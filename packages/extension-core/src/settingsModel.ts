export type AnalysisMode = "light" | "default" | "full";
export type DocsSource = "auto" | "workspace" | "runtime" | "reference";
export type LoggingLevel = "error" | "warn" | "info" | "debug";
export type RunTarget = "terminal" | "repl";

export interface SageSettings {
  interpreterPath: string;
  interpreterArgs: string[];
  languageServerRustPath: string;
  languageServerPythonPath: string;
  languageServerPythonArgs: string[];
  analysisMode: AnalysisMode;
  extraPaths: string[];
  sourceRoots: string[];
  diagnosticsEnabled: boolean;
  runtimeIntrospectionEnabled: boolean;
  enablePyxParsing: boolean;
  pythonFilesEnabled: boolean;
  indexingExcludeGlobs: string[];
  docsSource: DocsSource;
  showDocsOnHover: boolean;
  loggingLevel: LoggingLevel;
  runTarget: RunTarget;
  showCellCodeLens: boolean;
  cleanupGeneratedPython: boolean;
  notebookSupportEnabled: boolean;
}

export interface LanguageServerInitializationOptions {
  interpreter: {
    path: string;
    args: string[];
    pythonPath?: string;
  };
  rust: {
    binaryPath?: string;
    cacheDir?: string;
  };
  pyright: {
    nodePath?: string;
    serverPath?: string;
  };
  analysis: {
    mode: AnalysisMode;
    extraPaths: string[];
    sourceRoots: string[];
    enableDiagnostics: boolean;
    enableRuntimeIntrospection: boolean;
    enablePyxParsing: boolean;
    enablePythonFiles: boolean;
  };
  workspace: {
    rootUri: string | null;
    folders: string[];
    sourceRoots: string[];
    exclude: string[];
  };
  documentation: {
    preferredSource: DocsSource;
    showOnHover: boolean;
  };
  logging: {
    level: LoggingLevel;
  };
  experimental: {
    notebookSupport: boolean;
  };
}

export interface WorkspaceInitializationInput {
  rootUri: string | null;
  folders: string[];
  sourceRoots: string[];
}

export interface RuntimeInitializationInput {
  resolvedRustPath?: string;
  cacheDir?: string;
  nodePath?: string;
  pyrightServerPath?: string;
  resolvedLanguageServerPythonPath?: string;
}

export function buildInitializationOptions(
  settings: SageSettings,
  workspace: WorkspaceInitializationInput,
  runtimeInput: RuntimeInitializationInput | string = {},
): LanguageServerInitializationOptions {
  const runtime = typeof runtimeInput === "string"
    ? { resolvedLanguageServerPythonPath: runtimeInput }
    : runtimeInput;
  return {
    interpreter: {
      path: settings.interpreterPath,
      args: settings.interpreterArgs,
      ...(runtime.resolvedLanguageServerPythonPath ? { pythonPath: runtime.resolvedLanguageServerPythonPath } : {}),
    },
    rust: {
      ...(runtime.resolvedRustPath ? { binaryPath: runtime.resolvedRustPath } : {}),
      ...(runtime.cacheDir ? { cacheDir: runtime.cacheDir } : {}),
    },
    pyright: {
      ...(runtime.nodePath ? { nodePath: runtime.nodePath } : {}),
      ...(runtime.pyrightServerPath ? { serverPath: runtime.pyrightServerPath } : {}),
    },
    analysis: {
      mode: settings.analysisMode,
      extraPaths: settings.extraPaths,
      sourceRoots: settings.sourceRoots,
      enableDiagnostics: settings.diagnosticsEnabled,
      enableRuntimeIntrospection: settings.runtimeIntrospectionEnabled,
      enablePyxParsing: settings.enablePyxParsing,
      enablePythonFiles: settings.pythonFilesEnabled,
    },
    workspace: {
      rootUri: workspace.rootUri,
      folders: workspace.folders,
      sourceRoots: workspace.sourceRoots,
      exclude: settings.indexingExcludeGlobs,
    },
    documentation: {
      preferredSource: settings.docsSource,
      showOnHover: settings.showDocsOnHover,
    },
    logging: {
      level: settings.loggingLevel,
    },
    experimental: {
      notebookSupport: settings.notebookSupportEnabled,
    },
  };
}
