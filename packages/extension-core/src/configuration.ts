import * as vscode from "vscode";
import {
  type AnalysisMode,
  type DocsSource,
  type LoggingLevel,
  type RunTarget,
  type SageSettings,
} from "./settingsModel";

const SECTION = "sage";

export function readSettings(workspaceFolder?: vscode.WorkspaceFolder): SageSettings {
  const configuration = vscode.workspace.getConfiguration(SECTION, workspaceFolder);
  return {
    interpreterPath: configuration.get<string>("interpreter.path", "sage"),
    interpreterArgs: configuration.get<string[]>("interpreter.args", []),
    languageServerPythonPath: configuration.get<string>("languageServer.pythonPath", "auto"),
    languageServerPythonArgs: configuration.get<string[]>("languageServer.pythonArgs", []),
    analysisMode: configuration.get<AnalysisMode>("analysis.mode", "default"),
    extraPaths: configuration.get<string[]>("analysis.extraPaths", []),
    sourceRoots: configuration.get<string[]>("analysis.sourceRoots", []),
    diagnosticsEnabled: configuration.get<boolean>("analysis.enableDiagnostics", true),
    runtimeIntrospectionEnabled: configuration.get<boolean>(
      "analysis.enableRuntimeIntrospection",
      false,
    ),
    enablePyxParsing: configuration.get<boolean>("analysis.enablePyxParsing", true),
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
    notebookSupportEnabled: configuration.get<boolean>("experimental.notebookSupport", false),
  };
}
