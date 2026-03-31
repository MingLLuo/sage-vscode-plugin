import * as vscode from "vscode";
import type { LanguageClient } from "vscode-languageclient/node";

import { readSettings } from "./configuration";
import { DocumentationPanel } from "./docsPanel";
import { renderDocumentationMarkdown } from "./documentationRequest";
import {
  buildInterpreterCommand,
  buildReplLoadCommand,
  buildRunFileCommand,
  shouldRunInRepl,
} from "./executionPlan";
import {
  formatEnvironmentDetails,
  formatStatusBarText,
  formatStatusBarTooltip,
} from "./environmentPresentation";
import { createLanguageClient, requestDocumentation } from "./languageClient";
import { shouldRestartLanguageServer } from "./serverRestart";
import { buildWorkspaceInitializationData } from "./workspaceDiscovery";

let client: LanguageClient | undefined;
let replTerminal: vscode.Terminal | undefined;
let runTerminal: vscode.Terminal | undefined;
let replBootstrapped = false;

export async function activate(context: vscode.ExtensionContext): Promise<void> {
  const outputChannel = vscode.window.createOutputChannel("Sage");
  const languageOutputChannel = vscode.window.createOutputChannel("Sage Language Server");
  const docsPanel = new DocumentationPanel();
  const statusBarItem = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Left, 100);

  context.subscriptions.push(outputChannel, languageOutputChannel, statusBarItem);

  const updateStatusBar = (): void => {
    const settings = readSettings(vscode.workspace.workspaceFolders?.[0]);
    const workspaceData = buildWorkspaceInitializationData(
      vscode.workspace.workspaceFolders?.map((folder) => folder.uri.fsPath) ?? [],
      settings.sourceRoots,
    );
    statusBarItem.text = formatStatusBarText({
      interpreterPath: settings.interpreterPath,
      analysisMode: settings.analysisMode,
      docsSource: settings.docsSource,
      sourceRoots: workspaceData.sourceRoots,
      enablePyxParsing: settings.enablePyxParsing,
    });
    statusBarItem.tooltip = formatStatusBarTooltip({
      interpreterPath: settings.interpreterPath,
      analysisMode: settings.analysisMode,
      docsSource: settings.docsSource,
      sourceRoots: workspaceData.sourceRoots,
      enablePyxParsing: settings.enablePyxParsing,
    });
    statusBarItem.command = "sage.showEnvironmentDetails";
    statusBarItem.show();
  };

  const startLanguageClient = async (): Promise<void> => {
    if (client) {
      await client.stop();
    }

    try {
      client = createLanguageClient(context, languageOutputChannel);
      await client.start();
      outputChannel.appendLine("Sage language client started.");
    } catch (error) {
      client = undefined;
      const message = `Sage language server failed to start: ${String(error)}`;
      outputChannel.appendLine(message);
      void vscode.window.showErrorMessage(
        `${message}. Check 'sage.languageServer.pythonPath' and the Sage output channels.`,
      );
    }
  };

  context.subscriptions.push(
    vscode.window.onDidCloseTerminal((terminal) => {
      if (replTerminal === terminal) {
        replTerminal = undefined;
        replBootstrapped = false;
      }
      if (runTerminal === terminal) {
        runTerminal = undefined;
      }
    }),
    vscode.commands.registerCommand("sage.selectInterpreter", async () => {
      const settings = readSettings(vscode.workspace.workspaceFolders?.[0]);
      const selection = await vscode.window.showInputBox({
        title: "Sage interpreter path",
        value: settings.interpreterPath,
        prompt: "Enter the Sage executable used for execution and Sage-aware runtime context.",
      });

      if (!selection) {
        return;
      }

      await vscode.workspace
        .getConfiguration("sage")
        .update("interpreter.path", selection, vscode.ConfigurationTarget.Workspace);
      resetReplTerminal();
      outputChannel.appendLine(`Updated Sage interpreter path to: ${selection}`);
      updateStatusBar();
      await startLanguageClient();
    }),
    vscode.commands.registerCommand("sage.restartLanguageServer", async () => {
      outputChannel.appendLine("Restarting Sage language server.");
      await startLanguageClient();
    }),
    vscode.commands.registerCommand("sage.runCurrentFile", async () => {
      const editor = vscode.window.activeTextEditor;
      if (!editor) {
        void vscode.window.showWarningMessage("Open a Sage file before running it.");
        return;
      }
      const settings = readSettings(vscode.workspace.getWorkspaceFolder(editor.document.uri));
      const terminal = shouldRunInRepl(settings.runTarget)
        ? ensureReplTerminal(settings)
        : getOrCreateRunTerminal();
      const command = shouldRunInRepl(settings.runTarget)
        ? buildReplLoadCommand(editor.document.uri.fsPath)
        : buildRunFileCommand(settings, editor.document.uri.fsPath);
      terminal.sendText(command, true);
      terminal.show(true);
    }),
    vscode.commands.registerCommand("sage.runSelection", async () => {
      const editor = vscode.window.activeTextEditor;
      if (!editor) {
        void vscode.window.showWarningMessage("Open a Sage file before sending code to REPL.");
        return;
      }
      const selection = editor.document.getText(editor.selection) || editor.document.lineAt(editor.selection.active.line).text;
      const settings = readSettings(vscode.workspace.getWorkspaceFolder(editor.document.uri));
      const terminal = ensureReplTerminal(settings);
      terminal.sendText(selection, true);
      terminal.show(true);
    }),
    vscode.commands.registerCommand("sage.startRepl", async () => {
      const settings = readSettings(vscode.workspace.workspaceFolders?.[0]);
      const terminal = ensureReplTerminal(settings);
      terminal.show(true);
    }),
    vscode.commands.registerCommand("sage.showDocumentation", async () => {
      if (!client) {
        void vscode.window.showWarningMessage("Sage language server is not available yet.");
        return;
      }

      const editor = vscode.window.activeTextEditor;
      if (!editor) {
        void vscode.window.showWarningMessage("Open a Sage file to request documentation.");
        return;
      }

      const selectedText = editor.document.getText(editor.selection).trim() || undefined;
      const result = await requestDocumentation(
        client,
        editor.document.uri.toString(),
        editor.selection.active.line,
        editor.selection.active.character,
        selectedText,
      );

      if (!result) {
        void vscode.window.showInformationMessage("No documentation available for the current symbol.");
        return;
      }

      docsPanel.show(`Docs: ${result.symbol}`, renderDocumentationMarkdown(result));
    }),
    vscode.commands.registerCommand("sage.showEnvironmentDetails", async () => {
      const settings = readSettings(vscode.workspace.workspaceFolders?.[0]);
      const workspaceData = buildWorkspaceInitializationData(
        vscode.workspace.workspaceFolders?.map((folder) => folder.uri.fsPath) ?? [],
        settings.sourceRoots,
      );
      void vscode.window.showInformationMessage(
        formatEnvironmentDetails({
          interpreterPath: settings.interpreterPath,
          analysisMode: settings.analysisMode,
          docsSource: settings.docsSource,
          sourceRoots: workspaceData.sourceRoots,
          enablePyxParsing: settings.enablePyxParsing,
        }),
      );
    }),
    vscode.workspace.onDidChangeConfiguration(async (event) => {
      if (!event.affectsConfiguration("sage")) {
        return;
      }
      if (event.affectsConfiguration("sage.interpreter.path") || event.affectsConfiguration("sage.interpreter.args")) {
        resetReplTerminal();
      }
      updateStatusBar();
      if (shouldRestartLanguageServer((section) => event.affectsConfiguration(section))) {
        await startLanguageClient();
      }
    }),
    vscode.workspace.onDidChangeWorkspaceFolders(async () => {
      updateStatusBar();
      await startLanguageClient();
    }),
  );

  updateStatusBar();
  await startLanguageClient();
}

export async function deactivate(): Promise<void> {
  if (client) {
    await client.stop();
    client = undefined;
  }
}

function ensureReplTerminal(settings: {
  interpreterPath: string;
  interpreterArgs: readonly string[];
}): vscode.Terminal {
  if (!replTerminal) {
    replTerminal = vscode.window.createTerminal("Sage REPL");
    replBootstrapped = false;
  }

  if (!replBootstrapped) {
    replTerminal.sendText(buildInterpreterCommand(settings), true);
    replBootstrapped = true;
  }

  return replTerminal;
}

function getOrCreateRunTerminal(): vscode.Terminal {
  if (!runTerminal) {
    runTerminal = vscode.window.createTerminal("Sage Run");
  }
  return runTerminal;
}

function resetReplTerminal(): void {
  if (replTerminal) {
    replTerminal.dispose();
    replTerminal = undefined;
  }
  replBootstrapped = false;
}
