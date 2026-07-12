import fs from "node:fs/promises";
import path from "node:path";

import * as vscode from "vscode";

import { workspaceAliasedSourcePath } from "./sourceRootPaths";

export const SAGE_SOURCE_SCHEME = "sage-source";

export function buildSageSourceUri(sourcePath: string): vscode.Uri {
  return vscode.Uri.from({
    scheme: SAGE_SOURCE_SCHEME,
    path: sourcePath,
  });
}

export function shouldUseSageSourceView(
  sourcePath: string,
  workspaceFolders: readonly string[],
): boolean {
  return workspaceAliasedSourcePath(sourcePath, workspaceFolders) === undefined;
}

export class SageSourceTextDocumentProvider implements vscode.TextDocumentContentProvider {
  provideTextDocumentContent(uri: vscode.Uri): Promise<string> {
    const sourcePath = uri.path;
    if (!path.isAbsolute(sourcePath)) {
      throw new Error(`Invalid Sage source URI path: ${uri.toString()}`);
    }
    return fs.readFile(sourcePath, "utf8");
  }
}
