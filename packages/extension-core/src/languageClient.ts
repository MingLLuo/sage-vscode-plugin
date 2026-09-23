import fs from "node:fs";
import path from "node:path";
import { createHash } from "node:crypto";
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
import { shouldStopOnTransportErrors } from "./serverRestart";
import { workspaceAliasedSourcePath } from "./sourceRootPaths";
import { buildWorkspaceInitializationData, resolveConfiguredPaths } from "./workspaceDiscovery";
import { logToChannel } from "./extensionLogger";
import {
  executeSageCommand,
  executeSageCommandWithTimeout,
} from "./sageCommandClient";

export {
  executeSageCommand,
  executeSageCommandWithTimeout,
};

const DOCUMENTATION_REQUEST_TIMEOUT_MS = 15_000;

export const RUST_LSP_COMMANDS = {
  indexStatus: "sage.__rust.indexStatus",
  docsStatus: "sage.__rust.docsStatus",
  rebuildIndex: "sage.__rust.rebuildIndex",
  getDocumentation: "sage.__rust.getDocumentation",
  queryAtPosition: "sage.__rust.queryAtPosition",
} as const;

export const SAGE_LANGUAGE_FILE_GLOB = "**/*.{sage,py,pyx,pxd,pxi,spyx}";

export { buildDocumentSelector };

export function rustIndexCacheDir(context: vscode.ExtensionContext, folder?: vscode.WorkspaceFolder): string {
  const root = path.join(context.globalStorageUri.fsPath, "rust-index-v2");
  return folder ? path.join(root, createHash("sha256").update(folder.uri.toString()).digest("hex").slice(0, 16)) : root;
}

export interface LanguageClientLifecycle {
  fileSystemWatcher: vscode.FileSystemWatcher;
  shouldAutoRestartOnClose?: () => boolean;
  onClose?: (event: { managedShutdown: boolean; shouldRestart: boolean }) => void;
  runtimeDiscoveredSourceRoots?: readonly string[];
  workspaceFolder?: vscode.WorkspaceFolder;
}

export function createLanguageClient(
  context: vscode.ExtensionContext,
  outputChannel: vscode.OutputChannel,
  lifecycle: LanguageClientLifecycle,
): LanguageClient {
  const workspaceFolder = lifecycle.workspaceFolder ?? vscode.workspace.workspaceFolders?.[0];
  const workspaceFolders = workspaceFolder ? [workspaceFolder.uri.fsPath] : [];
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
    documentSelector: lifecycle.workspaceFolder
      ? [
        { language: "sagemath", scheme: "file", pattern: `${lifecycle.workspaceFolder.uri.fsPath.replaceAll("\\", "/")}/**/*` },
        { language: "sagemath-cython", scheme: "file", pattern: `${lifecycle.workspaceFolder.uri.fsPath.replaceAll("\\", "/")}/**/*` },
        ...(lifecycle.workspaceFolder.index === 0
          ? [
            { language: "sagemath", scheme: "untitled" },
            { language: "sagemath-cython", scheme: "untitled" },
          ]
          : []),
        ...(settings.pythonFilesEnabled
          ? [{ language: "python", scheme: "file", pattern: `${lifecycle.workspaceFolder.uri.fsPath.replaceAll("\\", "/")}/**/*` }]
          : []),
      ]
      : buildDocumentSelector(settings),
    ...(lifecycle.workspaceFolder ? { workspaceFolder: lifecycle.workspaceFolder } : {}),
    outputChannel,
    initializationOptions: buildInitializationOptions(languageServerSettings, workspaceData, {
      resolvedRustPath: launch.command,
      nodePath: process.execPath,
      pyrightServerPath: resolvePyrightServerPath(context.extensionPath),
      cacheDir: rustIndexCacheDir(context, lifecycle.workspaceFolder),
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
        return { action: shouldStopOnTransportErrors(count) ? ErrorAction.Shutdown : ErrorAction.Continue };
      },
      closed: () => {
        const managedShutdown = !(lifecycle.shouldAutoRestartOnClose?.() ?? true);
        const shouldRestart = false;
        lifecycle.onClose?.({ managedShutdown, shouldRestart });
        logToChannel(
          outputChannel,
          settings.loggingLevel,
          managedShutdown ? "info" : "warn",
          "lsp-client",
          managedShutdown
            ? "language server connection closed during managed restart"
            : "language server connection closed unexpectedly; recovery scheduled by extension",
          { managedShutdown, shouldRestart },
        );
        return { action: CloseAction.DoNotRestart, handled: true };
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
      targetUri: rewriteExternalSourceUri(definition.targetUri, workspaceFolders),
    };
  }
  if (isLocationLike(definition)) {
    return new vscode.Location(
      rewriteExternalSourceUri(definition.uri, workspaceFolders),
      definition.range,
    );
  }
  return definition;
}

export function rewriteExternalSourceUri(
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
  const response = await executeSageCommandWithTimeout<DocumentationResponse>(
    client,
    RUST_LSP_COMMANDS.getDocumentation,
    [buildDocumentationRequestPayload(documentUri, line, character, symbol)],
    {
      timeoutMs: DOCUMENTATION_REQUEST_TIMEOUT_MS,
      label: "Sage documentation request",
    },
  );
  return normalizeDocumentationResponse(response);
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
