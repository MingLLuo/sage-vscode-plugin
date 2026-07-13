import fs from "node:fs/promises";
import path from "node:path";

import * as vscode from "vscode";

import { BackingFileWatchRegistry } from "./backingFileWatchRegistry";
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

export class SageSourceTextDocumentProvider implements vscode.TextDocumentContentProvider, vscode.Disposable {
  private readonly changeEmitter = new vscode.EventEmitter<vscode.Uri>();
  private readonly backingFiles = new BackingFileWatchRegistry<string>((uri) => {
    this.changeEmitter.fire(vscode.Uri.parse(uri));
  });
  private readonly activeReads = new Map<string, number>();
  private readGeneration = 0;
  readonly onDidChange = this.changeEmitter.event;

  async provideTextDocumentContent(uri: vscode.Uri): Promise<string> {
    const sourcePath = uri.path;
    if (!path.isAbsolute(sourcePath)) {
      throw new Error(`Invalid Sage source URI path: ${uri.toString()}`);
    }
    const uriKey = uri.toString();
    const generation = ++this.readGeneration;
    this.activeReads.set(uriKey, generation);
    // Register before reading so a write that lands between the read and the
    // returned provider result cannot leave the visible snapshot stale.
    this.backingFiles.track(sourcePath, uriKey);
    let contents: string;
    try {
      contents = await fs.readFile(sourcePath, "utf8");
    } catch (error) {
      if (this.activeReads.get(uriKey) === generation) {
        this.activeReads.delete(uriKey);
        this.backingFiles.release(sourcePath, uriKey);
      }
      throw error;
    }
    return contents;
  }

  refresh(uri: vscode.Uri): void {
    if (uri.scheme === SAGE_SOURCE_SCHEME) {
      this.changeEmitter.fire(uri);
    }
  }

  release(uri: vscode.Uri): void {
    if (uri.scheme === SAGE_SOURCE_SCHEME && path.isAbsolute(uri.path)) {
      const uriKey = uri.toString();
      this.activeReads.delete(uriKey);
      this.backingFiles.release(uri.path, uriKey);
    }
  }

  dispose(): void {
    this.backingFiles.dispose();
    this.activeReads.clear();
    this.changeEmitter.dispose();
  }
}
