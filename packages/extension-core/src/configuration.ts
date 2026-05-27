import * as vscode from "vscode";
import {
  type AnalysisMode,
  type DocsSource,
  type LoggingLevel,
  type RunTarget,
  type SageSettings,
} from "./settingsModel";

const SECTION = "sage";

export function readSettings(scope?: vscode.WorkspaceFolder | vscode.Uri): SageSettings {
  const configuration = vscode.workspace.getConfiguration(SECTION, configurationScope(scope));
  return {
    interpreterPath: configuration.get<string>("interpreter.path", "sage"),
    interpreterArgs: configuration.get<string[]>("interpreter.args", []),
    languageServerRustPath: configuration.get<string>("languageServer.rustPath", "auto"),
    languageServerPythonPath: configuration.get<string>("languageServer.pythonPath", "auto"),
    languageServerPythonArgs: configuration.get<string[]>("languageServer.pythonArgs", []),
    analysisMode: configuration.get<AnalysisMode>("analysis.mode", "default"),
    extraPaths: configuration.get<string[]>("analysis.extraPaths", []),
    sourceRoots: configuration.get<string[]>("analysis.sourceRoots", []),
    diagnosticsEnabled: configuration.get<boolean>("analysis.enableDiagnostics", true),
    runtimeIntrospectionEnabled: configuration.get<boolean>(
      "analysis.enableRuntimeIntrospection",
      true,
    ),
    enablePyxParsing: configuration.get<boolean>("analysis.enablePyxParsing", true),
    pythonFilesEnabled: configuration.get<boolean>("analysis.enablePythonFiles", false),
    indexingExcludeGlobs: configuration.get<string[]>("indexing.exclude", [
      "**/.git/**",
      "**/__pycache__/**",
      "**/.venv/**",
      "**/build/**",
    ]),
    docsSource: configuration.get<DocsSource>("docs.preferredSource", "auto"),
    showDocsOnHover: configuration.get<boolean>("docs.showOnHover", true),
    loggingLevel: configuration.get<LoggingLevel>("logging.level", "info"),
    runTarget: configuration.get<RunTarget>("run.target", "terminal"),
    showCellCodeLens: configuration.get<boolean>("run.showCellCodeLens", true),
    cleanupGeneratedPython: configuration.get<boolean>("run.cleanupGeneratedPython", false),
    notebookSupportEnabled: configuration.get<boolean>("experimental.notebookSupport", false),
  };
}

function configurationScope(scope?: vscode.WorkspaceFolder | vscode.Uri): vscode.Uri | undefined {
  if (scope instanceof vscode.Uri) {
    return scope;
  }
  return scope?.uri ?? vscode.workspace.workspaceFolders?.[0]?.uri;
}
