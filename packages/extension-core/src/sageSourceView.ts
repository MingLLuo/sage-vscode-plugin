import fs from "node:fs/promises";
import path from "node:path";

import * as vscode from "vscode";

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
  const resolved = path.resolve(sourcePath);
  return !workspaceFolders.some((folder) => isPathInsideOrEqual(resolved, path.resolve(folder)));
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

function isPathInsideOrEqual(targetPath: string, folder: string): boolean {
  const relative = path.relative(folder, targetPath);
  return relative === "" || (!relative.startsWith("..") && !path.isAbsolute(relative));
}
