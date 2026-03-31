import path from "node:path";
import * as vscode from "vscode";
import {
  CloseAction,
  ErrorAction,
  LanguageClient,
  type LanguageClientOptions,
  type ServerOptions,
} from "vscode-languageclient/node";

import { readSettings } from "./configuration";
import { buildInitializationOptions } from "./settingsModel";
import {
  buildDocumentationRequestPayload,
  normalizeDocumentationResponse,
  type DocumentationResponse,
  type DocumentationResult,
} from "./documentationRequest";
import { buildWorkspaceInitializationData } from "./workspaceDiscovery";

export function createLanguageClient(
  context: vscode.ExtensionContext,
  outputChannel: vscode.OutputChannel,
): LanguageClient {
  const workspaceFolder = vscode.workspace.workspaceFolders?.[0];
  const settings = readSettings(workspaceFolder);
  const serverModuleRoot = path.resolve(context.extensionPath, "../sage-lsp/src");
  const workspaceData = buildWorkspaceInitializationData(
    vscode.workspace.workspaceFolders?.map((folder) => folder.uri.fsPath) ?? [],
    settings.sourceRoots,
  );

  const serverOptions: ServerOptions = {
    command: "python3",
    args: ["-m", "sage_lsp"],
    options: {
      cwd: context.extensionPath,
      env: {
        ...process.env,
        PYTHONPATH: appendPythonPath(process.env.PYTHONPATH, serverModuleRoot, settings.extraPaths),
      },
    },
  };

  const clientOptions: LanguageClientOptions = {
    documentSelector: [{ language: "sagemath" }],
    outputChannel,
    initializationOptions: buildInitializationOptions(settings, workspaceData),
    synchronize: {
      configurationSection: "sage",
      fileEvents: vscode.workspace.createFileSystemWatcher("**/*.sage"),
    },
    errorHandler: {
      error: (error, message, count) => {
        outputChannel.appendLine(
          `[error] language server error (message=${message}, count=${count}): ${String(error)}`,
        );
        return { action: ErrorAction.Continue };
      },
      closed: () => {
        outputChannel.appendLine("[warn] language server connection closed; restarting.");
        return { action: CloseAction.Restart };
      },
    },
  };

  return new LanguageClient("sageLanguageServer", "Sage Language Server", serverOptions, clientOptions);
}

export async function requestDocumentation(
  client: LanguageClient,
  documentUri: string,
  line: number,
  character: number,
  symbol?: string,
): Promise<DocumentationResult | null> {
  const response = await client.sendRequest<DocumentationResponse | null>(
    "sage/getDocumentation",
    buildDocumentationRequestPayload(documentUri, line, character, symbol),
  );
  return normalizeDocumentationResponse(response);
}

function appendPythonPath(
  existingPath: string | undefined,
  monorepoSourceRoot: string,
  extraPaths: string[],
): string {
  const entries = [monorepoSourceRoot, ...extraPaths];
  if (existingPath) {
    entries.push(existingPath);
  }
  return entries.join(path.delimiter);
}
