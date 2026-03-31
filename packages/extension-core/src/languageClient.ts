import * as vscode from "vscode";
import { LanguageClient, ServerOptions, TransportKind } from "vscode-languageclient/node";
import { SageInitializationOptions, readInitializationOptions } from "./settingsModel";

export function createLanguageClient(context: vscode.ExtensionContext): LanguageClient {
  const initializationOptions: SageInitializationOptions = readInitializationOptions();
  void context;

  const serverOptions: ServerOptions = {
    command: initializationOptions.interpreterPath,
    args: ["-m", "sage_lsp"],
    transport: TransportKind.stdio
  };

  return new LanguageClient(
    "sageLanguageServer",
    "Sage Language Server",
    serverOptions,
    {
      documentSelector: [{ language: "sagemath" }],
      initializationOptions,
      outputChannel: vscode.window.createOutputChannel("Sage Language Server")
    }
  );
}
