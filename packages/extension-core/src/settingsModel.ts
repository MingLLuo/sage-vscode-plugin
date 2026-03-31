export type AnalysisMode = "light" | "default" | "full";
export type DocsSource = "auto" | "workspace" | "runtime" | "reference";
export type LoggingLevel = "error" | "warn" | "info" | "debug";
export type RunTarget = "terminal" | "repl";

export interface SageSettings {
  interpreterPath: string;
  interpreterArgs: string[];
  analysisMode: AnalysisMode;
  extraPaths: string[];
  sourceRoots: string[];
  diagnosticsEnabled: boolean;
  runtimeIntrospectionEnabled: boolean;
  enablePyxParsing: boolean;
  indexingExcludeGlobs: string[];
  docsSource: DocsSource;
  showDocsOnHover: boolean;
  loggingLevel: LoggingLevel;
  runTarget: RunTarget;
  notebookSupportEnabled: boolean;
}

export interface LanguageServerInitializationOptions {
  interpreter: {
    path: string;
    args: string[];
  };
  analysis: {
    mode: AnalysisMode;
    extraPaths: string[];
    sourceRoots: string[];
    enableDiagnostics: boolean;
    enableRuntimeIntrospection: boolean;
    enablePyxParsing: boolean;
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

export function buildInitializationOptions(
  settings: SageSettings,
  workspace: WorkspaceInitializationInput,
): LanguageServerInitializationOptions {
  return {
    interpreter: {
      path: settings.interpreterPath,
      args: settings.interpreterArgs,
    },
    analysis: {
      mode: settings.analysisMode,
      extraPaths: settings.extraPaths,
      sourceRoots: settings.sourceRoots,
      enableDiagnostics: settings.diagnosticsEnabled,
      enableRuntimeIntrospection: settings.runtimeIntrospectionEnabled,
      enablePyxParsing: settings.enablePyxParsing,
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
