import * as vscode from "vscode";

import { readSettings } from "./configuration";
import { currentSageCell } from "./sageCells";
import { prepareRunFileDocument } from "./runFilePreparation";
import type { SageTerminalManager } from "./terminalManager";
import { resolveRuntimePythonPaths } from "./workspaceDiscovery";

export interface RunCurrentCellTarget {
  uri?: vscode.Uri;
  line?: number;
}

export interface ExecutionCommandDependencies {
  terminalManager: SageTerminalManager;
  ensureWorkspaceRuntimeAvailable(action: string): Promise<boolean>;
  workspaceFolderPaths(): string[];
  showExecutionStatus(message: string, fields?: Record<string, unknown>): void;
}

export function registerExecutionCommands(
  dependencies: ExecutionCommandDependencies,
): vscode.Disposable[] {
  return [
    vscode.commands.registerCommand("sage.runCurrentFile", async () => {
      if (!(await dependencies.ensureWorkspaceRuntimeAvailable("Running a Sage file"))) {
        return;
      }
      const editor = vscode.window.activeTextEditor;
      if (!editor) {
        void vscode.window.showWarningMessage("Open a Sage file before running it.");
        return;
      }
      if (editor.document.uri.scheme !== "file") {
        void vscode.window.showWarningMessage("Sage can only run local files from disk.");
        return;
      }
      const preparation = await prepareRunFileDocument(editor.document);
      if (!preparation.ready) {
        if (preparation.reason === "save-not-completed") {
          void vscode.window.showWarningMessage(
            "Sage did not run the current file because saving its unsaved changes was cancelled or did not complete.",
          );
        } else {
          const detail = preparation.error instanceof Error
            ? preparation.error.message
            : String(preparation.error);
          void vscode.window.showErrorMessage(
            `Sage could not save the current file, so it was not run: ${detail}`,
          );
        }
        return;
      }
      const settings = readSettings(vscode.workspace.getWorkspaceFolder(editor.document.uri));
      const runtimePythonPaths = resolveRuntimePythonPaths(
        workspacePathsForDocument(editor.document, dependencies.workspaceFolderPaths()),
        settings.sourceRoots,
        settings.extraPaths,
      );
      let terminal: vscode.Terminal | undefined;
      try {
        terminal = await dependencies.terminalManager.runFile(
          { ...settings, runtimePythonPaths },
          editor.document.uri.fsPath,
        );
      } catch (error) {
        const action = await vscode.window.showErrorMessage(
          `Sage could not run the current file: ${String(error)}`,
          "Select Interpreter",
        );
        if (action === "Select Interpreter") {
          await vscode.commands.executeCommand("sage.selectInterpreter");
        }
        return;
      }
      terminal?.show(true);
      dependencies.showExecutionStatus(
        settings.runTarget === "repl"
          ? "Sage: sent current file to REPL"
          : "Sage: running current file",
        {
          target: settings.runTarget,
          path: editor.document.uri.fsPath,
          runtimePythonPaths: runtimePythonPaths.length,
        },
      );
    }),
    vscode.commands.registerCommand("sage.runSelection", async () => {
      if (!(await dependencies.ensureWorkspaceRuntimeAvailable("Running Sage code in the REPL"))) {
        return;
      }
      const editor = vscode.window.activeTextEditor;
      if (!editor) {
        void vscode.window.showWarningMessage("Open a Sage file before sending code to REPL.");
        return;
      }
      const selection = editor.document.getText(editor.selection)
        || editor.document.lineAt(editor.selection.active.line).text;
      const settings = readSettings(vscode.workspace.getWorkspaceFolder(editor.document.uri));
      const runtimePythonPaths = resolveRuntimePythonPaths(
        workspacePathsForDocument(editor.document, dependencies.workspaceFolderPaths()),
        settings.sourceRoots,
        settings.extraPaths,
        editor.document.uri.fsPath,
      );
      let terminal: vscode.Terminal;
      try {
        terminal = dependencies.terminalManager.runSelection({ ...settings, runtimePythonPaths }, selection);
      } catch (error) {
        await showTerminalLaunchError("send the selection to the Sage REPL", error);
        return;
      }
      terminal.show(true);
      dependencies.showExecutionStatus("Sage: sent selection to REPL", {
        selectionLength: selection.length,
        runtimePythonPaths: runtimePythonPaths.length,
      });
    }),
    vscode.commands.registerCommand("sage.runCurrentCell", async (target?: RunCurrentCellTarget) => {
      if (!(await dependencies.ensureWorkspaceRuntimeAvailable("Running the current Sage cell in the REPL"))) {
        return;
      }
      const editor = await editorForCellExecution(target);
      if (!editor) {
        void vscode.window.showWarningMessage("Open a Sage file before sending a cell to REPL.");
        return;
      }
      const activeLine = typeof target?.line === "number" ? target.line : editor.selection.active.line;
      const cell = currentSageCell(editor.document.getText(), activeLine);
      if (!cell) {
        void vscode.window.showWarningMessage("No Sage cell content found near the cursor.");
        return;
      }
      const settings = readSettings(vscode.workspace.getWorkspaceFolder(editor.document.uri));
      const runtimePythonPaths = resolveRuntimePythonPaths(
        workspacePathsForDocument(editor.document, dependencies.workspaceFolderPaths()),
        settings.sourceRoots,
        settings.extraPaths,
        editor.document.uri.scheme === "file" ? editor.document.uri.fsPath : undefined,
      );
      let terminal: vscode.Terminal;
      try {
        terminal = dependencies.terminalManager.runSelection({ ...settings, runtimePythonPaths }, cell.text);
      } catch (error) {
        await showTerminalLaunchError("send the current cell to the Sage REPL", error);
        return;
      }
      terminal.show(true);
      dependencies.showExecutionStatus("Sage: sent current cell to REPL", {
        startLine: cell.startLine,
        endLine: cell.endLine,
        selectionLength: cell.text.length,
        runtimePythonPaths: runtimePythonPaths.length,
      });
    }),
    vscode.commands.registerCommand("sage.startRepl", async () => {
      if (!(await dependencies.ensureWorkspaceRuntimeAvailable("Starting the Sage REPL"))) {
        return;
      }
      const activeDocument = vscode.window.activeTextEditor?.document;
      const settings = readSettings(
        activeDocument
          ? vscode.workspace.getWorkspaceFolder(activeDocument.uri)
          : vscode.workspace.workspaceFolders?.[0],
      );
      let terminal: vscode.Terminal;
      try {
        terminal = dependencies.terminalManager.startRepl(settings);
      } catch (error) {
        await showTerminalLaunchError("start the Sage REPL", error);
        return;
      }
      terminal.show(true);
      dependencies.showExecutionStatus("Sage: REPL ready or starting", {
        interpreterPath: settings.interpreterPath,
      });
    }),
  ];
}

function workspacePathsForDocument(
  document: vscode.TextDocument,
  fallback: string[],
): string[] {
  const workspaceFolder = vscode.workspace.getWorkspaceFolder(document.uri);
  return workspaceFolder ? [workspaceFolder.uri.fsPath] : fallback;
}

async function showTerminalLaunchError(action: string, error: unknown): Promise<void> {
  const selectedAction = await vscode.window.showErrorMessage(
    `Sage could not ${action}: ${String(error)}`,
    "Select Interpreter",
  );
  if (selectedAction === "Select Interpreter") {
    await vscode.commands.executeCommand("sage.selectInterpreter");
  }
}

async function editorForCellExecution(target?: RunCurrentCellTarget): Promise<vscode.TextEditor | undefined> {
  if (!target?.uri) {
    return vscode.window.activeTextEditor;
  }
  const targetUri = target.uri.toString();
  const visibleEditor = vscode.window.visibleTextEditors.find(
    (editor) => editor.document.uri.toString() === targetUri,
  );
  if (visibleEditor) {
    return visibleEditor;
  }
  const document = await vscode.workspace.openTextDocument(target.uri);
  return vscode.window.showTextDocument(document, { preview: false, preserveFocus: true });
}
