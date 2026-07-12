import fs from "node:fs";
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
import { buildDocumentSelector } from "./documentSelector";
import { buildLanguageServerLaunch } from "./serverLaunch";
import { buildSageSourceUri } from "./sageSourceView";
import { shouldAutoRestartOnLanguageServerClose } from "./serverRestart";
import { workspaceAliasedSourcePath } from "./sourceRootPaths";
import { buildWorkspaceInitializationData, resolveConfiguredPaths } from "./workspaceDiscovery";
import { logToChannel } from "./extensionLogger";

export const RUST_LSP_COMMANDS = {
  indexStatus: "sage.__rust.indexStatus",
  docsStatus: "sage.__rust.docsStatus",
  rebuildIndex: "sage.__rust.rebuildIndex",
  getDocumentation: "sage.__rust.getDocumentation",
  queryAtPosition: "sage.__rust.queryAtPosition",
} as const;

export const SAGE_LANGUAGE_FILE_GLOB = "**/*.{sage,py,pyx,pxd,pxi,spyx}";

export { buildDocumentSelector };

export function rustIndexCacheDir(context: vscode.ExtensionContext): string {
  return path.join(context.globalStorageUri.fsPath, "rust-index-v2");
}

export interface LanguageClientLifecycle {
  fileSystemWatcher: vscode.FileSystemWatcher;
  shouldAutoRestartOnClose?: () => boolean;
  onClose?: (event: { managedShutdown: boolean; shouldRestart: boolean }) => void;
  runtimeDiscoveredSourceRoots?: readonly string[];
}

export function createLanguageClient(
  context: vscode.ExtensionContext,
  outputChannel: vscode.OutputChannel,
  lifecycle: LanguageClientLifecycle,
): LanguageClient {
  const workspaceFolders = vscode.workspace.workspaceFolders?.map((folder) => folder.uri.fsPath) ?? [];
  const workspaceFolder = vscode.workspace.workspaceFolders?.[0];
  const settings = readSettings(workspaceFolder);
  const effectiveSourceRoots = dedupeStrings([
    ...settings.sourceRoots,
    ...(lifecycle.runtimeDiscoveredSourceRoots ?? []),
  ]);
  const workspaceData = buildWorkspaceInitializationData(
    workspaceFolders,
    effectiveSourceRoots,
    {
      interpreterPath: settings.interpreterPath,
      interpreterArgs: settings.interpreterArgs,
      runtimeProbe: false,
    },
  );
  const resolvedExtraPaths = resolveConfiguredPaths(workspaceFolders, settings.extraPaths);
  const languageServerSettings = {
    ...settings,
    sourceRoots: effectiveSourceRoots,
    extraPaths: resolvedExtraPaths,
  };
  const launch = buildLanguageServerLaunch({
    interpreterPath: settings.interpreterPath,
    interpreterArgs: settings.interpreterArgs,
    languageServerRustPath: settings.languageServerRustPath,
    languageServerPythonPath: settings.languageServerPythonPath,
    languageServerPythonArgs: settings.languageServerPythonArgs,
    extensionPath: context.extensionPath,
    repositoryRoot: path.resolve(context.extensionPath, "../.."),
    homeDir: process.env.HOME,
  });
  logToChannel(
    outputChannel,
    settings.loggingLevel,
    "info",
    "lsp-client",
    "starting language server",
    { command: launch.command, args: launch.args.join(" ") },
  );

  const serverOptions: ServerOptions = {
    command: launch.command,
    args: launch.args,
    options: {
      cwd: context.extensionPath,
      env: {
        ...process.env,
        SAGE_LS_EXTRA_PATHS: resolvedExtraPaths.join(path.delimiter),
      },
    },
  };

  const clientOptions: LanguageClientOptions = {
    documentSelector: buildDocumentSelector(settings),
    outputChannel,
    initializationOptions: buildInitializationOptions(languageServerSettings, workspaceData, {
      resolvedRustPath: launch.command,
      nodePath: process.execPath,
      pyrightServerPath: resolvePyrightServerPath(context.extensionPath),
      cacheDir: rustIndexCacheDir(context),
    }),
    synchronize: {
      fileEvents: lifecycle.fileSystemWatcher,
    },
    middleware: {
      provideDeclaration: async (document, position, token, next) => {
        const declaration = await next(document, position, token);
        return rewriteExternalDefinitionUris(declaration);
      },
      provideDefinition: async (document, position, token, next) => {
        const definition = await next(document, position, token);
        return rewriteExternalDefinitionUris(definition);
      },
      provideImplementation: async (document, position, token, next) => {
        const implementation = await next(document, position, token);
        return rewriteExternalDefinitionUris(implementation);
      },
      provideTypeDefinition: async (document, position, token, next) => {
        const typeDefinition = await next(document, position, token);
        return rewriteExternalDefinitionUris(typeDefinition);
      },
      provideReferences: async (document, position, context, token, next) => {
        const references = await next(document, position, context, token);
        return rewriteExternalDefinitionUris(references);
      },
    },
    errorHandler: {
      error: (error, message, count) => {
        logToChannel(
          outputChannel,
          settings.loggingLevel,
          "error",
          "lsp-client",
          "language server error",
          { message, count, error: String(error) },
        );
        return { action: ErrorAction.Continue };
      },
      closed: () => {
        const managedShutdown = !(lifecycle.shouldAutoRestartOnClose?.() ?? true);
        const shouldRestart = shouldAutoRestartOnLanguageServerClose(managedShutdown);
        lifecycle.onClose?.({ managedShutdown, shouldRestart });
        logToChannel(
          outputChannel,
          settings.loggingLevel,
          managedShutdown ? "info" : "warn",
          "lsp-client",
          managedShutdown
            ? "language server connection closed during managed restart"
            : "language server connection closed unexpectedly; restarting",
          { managedShutdown, shouldRestart },
        );
        return { action: shouldRestart ? CloseAction.Restart : CloseAction.DoNotRestart };
      },
    },
  };

  return new LanguageClient("sageLanguageServer", "Sage Language Server", serverOptions, clientOptions);
}

export function rewriteExternalDefinitionUris<T extends vscode.Definition | vscode.DefinitionLink[] | null | undefined>(
  definition: T,
): T {
  if (!definition) {
    return definition;
  }
  const workspaceFolders = vscode.workspace.workspaceFolders?.map((folder) => folder.uri.fsPath) ?? [];
  if (Array.isArray(definition)) {
    return dedupeDefinitionEntries(
      definition.map((entry) => rewriteDefinitionEntry(entry, workspaceFolders)),
    ) as T;
  }
  return rewriteDefinitionEntry(definition, workspaceFolders) as T;
}

function rewriteDefinitionEntry(
  definition: vscode.Location | vscode.DefinitionLink,
  workspaceFolders: readonly string[],
): vscode.Location | vscode.DefinitionLink {
  if (isDefinitionLink(definition)) {
    return {
      ...definition,
      targetUri: rewriteExternalDefinitionUri(definition.targetUri, workspaceFolders),
    };
  }
  if (isLocationLike(definition)) {
    return new vscode.Location(
      rewriteExternalDefinitionUri(definition.uri, workspaceFolders),
      definition.range,
    );
  }
  return definition;
}

function rewriteExternalDefinitionUri(
  uri: vscode.Uri,
  workspaceFolders: readonly string[],
): vscode.Uri {
  if (uri.scheme !== "file") {
    return uri;
  }
  const workspacePath = workspaceAliasedSourcePath(uri.fsPath, workspaceFolders);
  if (workspacePath) {
    return vscode.Uri.file(workspacePath);
  }
  return buildSageSourceUri(uri.fsPath);
}

function isDefinitionLink(
  definition: vscode.Location | vscode.DefinitionLink,
): definition is vscode.DefinitionLink {
  return "targetUri" in definition && isUriLike(definition.targetUri);
}

function isLocationLike(
  definition: vscode.Location | vscode.DefinitionLink,
): definition is vscode.Location {
  return "uri" in definition && isUriLike(definition.uri);
}

function isUriLike(uri: unknown): uri is vscode.Uri {
  return (
    typeof uri === "object"
    && uri !== null
    && "scheme" in uri
    && typeof (uri as { scheme?: unknown }).scheme === "string"
    && "toString" in uri
    && typeof (uri as { toString?: unknown }).toString === "function"
  );
}

function dedupeDefinitionEntries(
  definitions: Array<vscode.Location | vscode.DefinitionLink>,
): Array<vscode.Location | vscode.DefinitionLink> {
  const seen = new Set<string>();
  const deduped: Array<vscode.Location | vscode.DefinitionLink> = [];
  for (const definition of definitions) {
    const key = definitionEntryKey(definition);
    if (seen.has(key)) {
      continue;
    }
    seen.add(key);
    deduped.push(definition);
  }
  return deduped;
}

function definitionEntryKey(definition: vscode.Location | vscode.DefinitionLink): string {
  if (isDefinitionLink(definition)) {
    return [
      definition.targetUri.toString(),
      rangeKey(definition.targetSelectionRange),
      rangeKey(definition.targetRange),
    ].join("|");
  }
  if (isLocationLike(definition)) {
    return [definition.uri.toString(), rangeKey(definition.range)].join("|");
  }
  return JSON.stringify(definition);
}

function rangeKey(range: vscode.Range | undefined): string {
  if (!range) {
    return "unknown-range";
  }
  return [
    range.start.line,
    range.start.character,
    range.end.line,
    range.end.character,
  ].join(":");
}

export async function requestDocumentation(
  client: LanguageClient,
  documentUri: string,
  line: number,
  character: number,
  symbol?: string,
): Promise<DocumentationResult | null> {
  const response = await client.sendRequest<DocumentationResponse | null>(
    "workspace/executeCommand",
    {
      command: RUST_LSP_COMMANDS.getDocumentation,
      arguments: [buildDocumentationRequestPayload(documentUri, line, character, symbol)],
    },
  );
  return normalizeDocumentationResponse(response);
}

export async function executeSageCommand<T>(
  client: LanguageClient,
  command: string,
  args: unknown[] = [],
): Promise<T | null> {
  return client.sendRequest<T | null>(
    "workspace/executeCommand",
    {
      command,
      arguments: args,
    },
  );
}

function resolvePyrightServerPath(extensionPath: string): string | undefined {
  const candidates = [
    path.resolve(extensionPath, "node_modules", "pyright", "langserver.index.js"),
    path.resolve(extensionPath, "..", "..", "node_modules", "pyright", "langserver.index.js"),
  ];
  return candidates.find((candidate) => fs.existsSync(candidate));
}

function dedupeStrings(values: readonly string[]): string[] {
  return [...new Set(values)];
}
