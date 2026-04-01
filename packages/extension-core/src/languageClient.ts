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
import { buildLanguageServerLaunch } from "./serverLaunch";
import { shouldAutoRestartOnLanguageServerClose } from "./serverRestart";
import { buildWorkspaceInitializationData, resolveConfiguredPaths } from "./workspaceDiscovery";

export interface LanguageClientLifecycle {
  shouldAutoRestartOnClose?: () => boolean;
  onClose?: (event: { managedShutdown: boolean; shouldRestart: boolean }) => void;
}

export function createLanguageClient(
  context: vscode.ExtensionContext,
  outputChannel: vscode.OutputChannel,
  lifecycle: LanguageClientLifecycle = {},
): LanguageClient {
  const workspaceFolders = vscode.workspace.workspaceFolders?.map((folder) => folder.uri.fsPath) ?? [];
  const workspaceFolder = vscode.workspace.workspaceFolders?.[0];
  const settings = readSettings(workspaceFolder);
  const serverModuleRoot = path.resolve(context.extensionPath, "../sage-lsp/src");
  const workspaceData = buildWorkspaceInitializationData(
    workspaceFolders,
    settings.sourceRoots,
    {
      interpreterPath: settings.interpreterPath,
      interpreterArgs: settings.interpreterArgs,
    },
  );
  const resolvedExtraPaths = resolveConfiguredPaths(workspaceFolders, settings.extraPaths);
  const languageServerSettings = {
    ...settings,
    extraPaths: resolvedExtraPaths,
  };
  const launch = buildLanguageServerLaunch({
    interpreterPath: settings.interpreterPath,
    interpreterArgs: settings.interpreterArgs,
    languageServerPythonPath: settings.languageServerPythonPath,
    languageServerPythonArgs: settings.languageServerPythonArgs,
    homeDir: process.env.HOME,
  });
  outputChannel.appendLine(
    `[info] starting language server with: ${launch.command} ${launch.args.join(" ")}`,
  );

  const serverOptions: ServerOptions = {
    command: launch.command,
    args: launch.args,
    options: {
      cwd: context.extensionPath,
      env: {
        ...process.env,
        PYTHONPATH: appendPythonPath(process.env.PYTHONPATH, serverModuleRoot, resolvedExtraPaths),
      },
    },
  };

  const clientOptions: LanguageClientOptions = {
    documentSelector: [{ language: "sagemath" }, { language: "sagemath-cython" }],
    outputChannel,
    initializationOptions: buildInitializationOptions(languageServerSettings, workspaceData, launch.command),
    synchronize: {
      fileEvents: [
        vscode.workspace.createFileSystemWatcher("**/*.sage"),
        vscode.workspace.createFileSystemWatcher("**/*.py"),
        vscode.workspace.createFileSystemWatcher("**/*.pyx"),
        vscode.workspace.createFileSystemWatcher("**/*.pxd"),
        vscode.workspace.createFileSystemWatcher("**/*.pxi"),
      ],
    },
    errorHandler: {
      error: (error, message, count) => {
        outputChannel.appendLine(
          `[error] language server error (message=${message}, count=${count}): ${String(error)}`,
        );
        return { action: ErrorAction.Continue };
      },
      closed: () => {
        const managedShutdown = !(lifecycle.shouldAutoRestartOnClose?.() ?? true);
        const shouldRestart = shouldAutoRestartOnLanguageServerClose(managedShutdown);
        lifecycle.onClose?.({ managedShutdown, shouldRestart });
        outputChannel.appendLine(
          managedShutdown
            ? "[info] language server connection closed during a managed restart."
            : "[warn] language server connection closed unexpectedly; restarting.",
        );
        return { action: shouldRestart ? CloseAction.Restart : CloseAction.DoNotRestart };
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
