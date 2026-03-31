import * as vscode from "vscode";

export interface SageInitializationOptions {
  interpreterPath: string;
  analysisSourceRoots: string[];
  logLevel: string;
  workspaceTrustMode: "trusted" | "restricted";
}

export function readInitializationOptions(): SageInitializationOptions {
  const configuration = vscode.workspace.getConfiguration("sage");
  const sourceRoots = configuration.get<string[]>("analysis.sourceRoots", []);

  return {
    interpreterPath: configuration.get<string>("interpreterPath", "python"),
    analysisSourceRoots: Array.isArray(sourceRoots) ? sourceRoots : [],
    logLevel: configuration.get<string>("logLevel", "info"),
    workspaceTrustMode: vscode.workspace.isTrusted ? "trusted" : "restricted"
  };
}

