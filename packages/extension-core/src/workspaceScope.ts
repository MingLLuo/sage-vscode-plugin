import * as vscode from "vscode";

import { updateWorkspaceSettingJson } from "./workspaceSettingsJson";

export function workspaceFolderPaths(): string[] {
  return vscode.workspace.workspaceFolders?.map((folder) => folder.uri.fsPath) ?? [];
}

export function activeWorkspaceFolder(): vscode.WorkspaceFolder | undefined {
  const activeDocument = vscode.window.activeTextEditor?.document;
  return (activeDocument && vscode.workspace.getWorkspaceFolder(activeDocument.uri))
    || vscode.workspace.workspaceFolders?.[0];
}

export function workspaceConfigurationTarget(): vscode.ConfigurationTarget {
  return (vscode.workspace.workspaceFolders?.length ?? 0) > 1
    ? vscode.ConfigurationTarget.WorkspaceFolder
    : vscode.ConfigurationTarget.Workspace;
}

export function isUnregisteredConfigurationError(error: unknown): boolean {
  return error instanceof Error && error.message.includes("not a registered configuration");
}

export function formatLoggedConfigurationValue(value: unknown): string {
  if (Array.isArray(value)) {
    return value.join(",");
  }
  if (value && typeof value === "object") {
    return JSON.stringify(value);
  }
  return String(value);
}

export async function updateWorkspaceSettingsJson(
  workspaceFolderUri: vscode.Uri,
  setting: string,
  value: unknown,
): Promise<void> {
  const vscodeDirectoryUri = vscode.Uri.joinPath(workspaceFolderUri, ".vscode");
  const settingsUri = vscode.Uri.joinPath(vscodeDirectoryUri, "settings.json");
  let source = "";

  try {
    const existing = await vscode.workspace.fs.readFile(settingsUri);
    source = Buffer.from(existing).toString("utf8");
  } catch (error) {
    if (!(error instanceof vscode.FileSystemError && error.code === "FileNotFound")) {
      throw error;
    }
  }

  const updated = updateWorkspaceSettingJson(source, setting, value);
  await vscode.workspace.fs.createDirectory(vscodeDirectoryUri);
  await vscode.workspace.fs.writeFile(settingsUri, Buffer.from(updated, "utf8"));
}
