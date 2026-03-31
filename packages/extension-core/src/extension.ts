import * as vscode from "vscode";
import { LanguageClient } from "vscode-languageclient/node";
import { createLanguageClient } from "./languageClient";
import { readInitializationOptions } from "./settingsModel";

let client: LanguageClient | undefined;

export async function activate(context: vscode.ExtensionContext): Promise<void> {
  client = createLanguageClient(context);
  context.subscriptions.push(client);
  await client.start();

  context.subscriptions.push(
    vscode.commands.registerCommand("sage.selectInterpreter", async () => {
      const nextValue = await vscode.window.showInputBox({
        title: "Sage Interpreter Path",
        prompt: "Enter the Python executable that should launch the Sage language server",
        value: readInitializationOptions().interpreterPath
      });

      if (!nextValue) {
        return;
      }

      await vscode.workspace.getConfiguration("sage").update(
        "interpreterPath",
        nextValue,
        vscode.ConfigurationTarget.Workspace
      );

      void vscode.window.showInformationMessage(`Updated Sage interpreter to ${nextValue}`);
    }),
    vscode.commands.registerCommand("sage.showEnvironmentDetails", async () => {
      const settings = readInitializationOptions();
      const message = [
        `Interpreter: ${settings.interpreterPath}`,
        `Source roots: ${settings.analysisSourceRoots.join(", ") || "(workspace default)"}`,
        `Trust mode: ${settings.workspaceTrustMode}`,
        `Log level: ${settings.logLevel}`
      ].join("\n");

      await vscode.window.showInformationMessage(message, { modal: true });
    })
  );
}

export async function deactivate(): Promise<void> {
  await client?.stop();
  client = undefined;
}
