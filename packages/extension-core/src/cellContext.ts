import * as vscode from "vscode";
import { hasSageCells } from "./sageCells";

export const EDITOR_HAS_CELLS_CONTEXT = "sage.editorHasCells";

/**
 * Keeps `sage.editorHasCells` in sync with the active editor so the Run Current Cell shortcut only
 * claims Ctrl/Cmd+Enter in files that actually contain cells; elsewhere it stays "insert line below".
 */
export function registerCellContext(): vscode.Disposable {
  let last: boolean | undefined;
  let pending: ReturnType<typeof setTimeout> | undefined;
  const update = () => {
    pending = undefined;
    const text = vscode.window.activeTextEditor?.document.getText() ?? "";
    const next = hasSageCells(text);
    if (next !== last) {
      last = next;
      void vscode.commands.executeCommand("setContext", EDITOR_HAS_CELLS_CONTEXT, next);
    }
  };
  const schedule = () => {
    if (pending === undefined) {
      pending = setTimeout(update, 250);
    }
  };
  update();
  return vscode.Disposable.from(
    vscode.window.onDidChangeActiveTextEditor(update),
    vscode.workspace.onDidChangeTextDocument((event) => {
      if (event.document === vscode.window.activeTextEditor?.document) {
        schedule();
      }
    }),
    { dispose: () => pending !== undefined && clearTimeout(pending) },
  );
}
