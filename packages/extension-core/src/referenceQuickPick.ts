import * as vscode from "vscode";

import {
  referenceQuickPickLabel,
  type LspLocationPayload,
  type QuerySourceRange,
} from "./sageNavigation";

const PREVIEW_LINE_LIMIT = 200;

interface ReferenceQuickPickItem extends vscode.QuickPickItem {
  location: vscode.Location;
}

export function locationFromLspPayload(payload: LspLocationPayload): vscode.Location | undefined {
  try {
    return new vscode.Location(
      vscode.Uri.parse(payload.uri),
      new vscode.Range(
        payload.range.start.line,
        payload.range.start.character,
        payload.range.end.line,
        payload.range.end.character,
      ),
    );
  } catch {
    return undefined;
  }
}

export async function showReferencesQuickPick(locations: vscode.Location[]): Promise<void> {
  const items = await Promise.all(locations.map(referenceQuickPickItem));
  const picked = await vscode.window.showQuickPick(items, {
    matchOnDescription: true,
    matchOnDetail: true,
    placeHolder: "Select a Sage reference to open",
  });
  if (!picked) {
    return;
  }
  const document = await vscode.workspace.openTextDocument(picked.location.uri);
  const editor = await vscode.window.showTextDocument(document);
  editor.revealRange(picked.location.range, vscode.TextEditorRevealType.InCenterIfOutsideViewport);
  editor.selection = new vscode.Selection(picked.location.range.start, picked.location.range.end);
}

async function referenceQuickPickItem(location: vscode.Location, index: number): Promise<ReferenceQuickPickItem> {
  const preview = index < PREVIEW_LINE_LIMIT ? await referenceLinePreview(location) : undefined;
  return {
    ...referenceQuickPickLabel(
      location.uri.toString(),
      sourceRangeFromLocation(location),
      () => vscode.workspace.asRelativePath(location.uri, false),
      preview,
    ),
    location,
  };
}

async function referenceLinePreview(location: vscode.Location): Promise<string | undefined> {
  try {
    const document = await vscode.workspace.openTextDocument(location.uri);
    if (location.range.start.line >= document.lineCount) {
      return undefined;
    }
    return document.lineAt(location.range.start.line).text.trim();
  } catch {
    return undefined;
  }
}

function sourceRangeFromLocation(location: vscode.Location): QuerySourceRange {
  return {
    start_line: location.range.start.line,
    start_character: location.range.start.character,
    end_line: location.range.end.line,
    end_character: location.range.end.character,
  };
}
