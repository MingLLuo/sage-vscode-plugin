import * as path from "node:path";

import * as vscode from "vscode";
import type { LanguageClient } from "vscode-languageclient/node";
import type {
  Definition as ProtocolDefinition,
  DefinitionLink as ProtocolDefinitionLink,
} from "vscode-languageserver-protocol";

import type { OutputLogger } from "./extensionLogger";
import { registerExternalSourceLanguageFeatureProviders } from "./externalSourceLanguageFeatures";
import { rewriteExternalDefinitionUris } from "./languageClient";
import { isLspLocationPayload, type LspLocationPayload } from "./sageNavigation";
import { locationFromLspPayload } from "./referenceQuickPick";
import { SAGE_SOURCE_SCHEME } from "./sageSourceView";
import { sourceRootContainsDocument } from "./sourceRootPaths";

type ExternalSourceNavigationMethod =
  | "textDocument/definition"
  | "textDocument/declaration"
  | "textDocument/implementation"
  | "textDocument/typeDefinition";

export interface ExternalSourceNavigationDependencies {
  ensureLanguageClientReady(action: string): Promise<LanguageClient | undefined>;
  isExternalSourceDocument(document: vscode.TextDocument): boolean;
  refreshExternalSourceDocument?(document: vscode.TextDocument): void;
  logger: Pick<OutputLogger, "info" | "warn">;
}

export function externalSourceFileUri(document: vscode.TextDocument): vscode.Uri | undefined {
  if (document.uri.scheme === "file") {
    return document.languageId === "python" ? document.uri : undefined;
  }
  if (document.uri.scheme !== SAGE_SOURCE_SCHEME || !path.isAbsolute(document.uri.path)) {
    return undefined;
  }
  return vscode.Uri.file(document.uri.path);
}

export function languageServerUriForDocument(document: vscode.TextDocument): vscode.Uri {
  return externalSourceFileUri(document) ?? document.uri;
}

export function isExternalSageSourceDocument(
  document: vscode.TextDocument,
  sourceRoots: readonly string[],
): boolean {
  const sourceUri = externalSourceFileUri(document);
  if (!sourceUri) {
    return false;
  }
  if (document.uri.scheme === "file" && vscode.workspace.getWorkspaceFolder(document.uri)) {
    return false;
  }
  return sourceRootContainsDocument(sourceRoots, sourceUri.fsPath);
}

export function registerExternalSourceNavigationProviders(
  dependencies: ExternalSourceNavigationDependencies,
): vscode.Disposable[] {
  const requestNavigation = async (
    document: vscode.TextDocument,
    position: vscode.Position,
    token: vscode.CancellationToken,
    method: ExternalSourceNavigationMethod,
  ): Promise<vscode.Definition | vscode.DefinitionLink[] | null> => {
    if (!dependencies.isExternalSourceDocument(document)) {
      return null;
    }
    const sourceUri = externalSourceFileUri(document);
    if (!sourceUri) {
      return null;
    }
    const activeClient = await dependencies.ensureLanguageClientReady("Navigating Sage source");
    if (!activeClient || token.isCancellationRequested) {
      return null;
    }

    try {
      if (!(await externalSourceTextIsCurrent(document, sourceUri, dependencies))) {
        return null;
      }
      // Address the backing file directly while the visible editor remains the
      // read-only sage-source view. Do not open a hidden file document here.
      const response = await activeClient.sendRequest<ProtocolDefinition | ProtocolDefinitionLink[] | null>(
        method,
        {
          textDocument: { uri: sourceUri.toString() },
          position: { line: position.line, character: position.character },
        },
        token,
      );
      const converted = await activeClient.protocol2CodeConverter.asDefinitionResult(response, token);
      return rewriteExternalDefinitionUris(converted) ?? null;
    } catch (error) {
      dependencies.logger.warn("navigation", "external Sage source navigation failed", {
        method,
        uri: document.uri.toString(),
        sourceUri: sourceUri.toString(),
        line: position.line,
        character: position.character,
        error: String(error),
      });
      return null;
    }
  };

  const requestReferences = async (
    document: vscode.TextDocument,
    position: vscode.Position,
    referenceContext: vscode.ReferenceContext,
    token: vscode.CancellationToken,
  ): Promise<vscode.Location[]> => {
    if (!dependencies.isExternalSourceDocument(document)) {
      return [];
    }
    const sourceUri = externalSourceFileUri(document);
    if (!sourceUri) {
      return [];
    }
    const activeClient = await dependencies.ensureLanguageClientReady("Finding Sage references");
    if (!activeClient || token.isCancellationRequested) {
      return [];
    }
    const payload = {
      textDocument: { uri: sourceUri.toString() },
      position: { line: position.line, character: position.character },
      context: { includeDeclaration: referenceContext.includeDeclaration },
    };
    try {
      if (!(await externalSourceTextIsCurrent(document, sourceUri, dependencies))) {
        return [];
      }
      const references = await activeClient.sendRequest<LspLocationPayload[]>(
        "textDocument/references",
        payload,
        token,
      );
      const locations = (references ?? [])
        .filter(isLspLocationPayload)
        .map(locationFromLspPayload)
        .filter((location): location is vscode.Location => Boolean(location));
      const rewrittenLocations = rewriteExternalDefinitionUris(locations);
      dependencies.logger.info("navigation", "external Sage source references resolved", {
        uri: document.uri.toString(),
        sourceUri: sourceUri.toString(),
        line: position.line,
        character: position.character,
        includeDeclaration: referenceContext.includeDeclaration,
        count: rewrittenLocations.length,
      });
      return rewrittenLocations;
    } catch (error) {
      dependencies.logger.warn("navigation", "external Sage source references failed", {
        uri: document.uri.toString(),
        sourceUri: sourceUri.toString(),
        line: position.line,
        character: position.character,
        error: String(error),
      });
      return [];
    }
  };

  const selector: vscode.DocumentSelector = [{ scheme: SAGE_SOURCE_SCHEME }];
  return [
    vscode.languages.registerDefinitionProvider(selector, {
      provideDefinition: (document, position, token) =>
        requestNavigation(document, position, token, "textDocument/definition"),
    }),
    vscode.languages.registerDeclarationProvider(selector, {
      provideDeclaration: (document, position, token) =>
        requestNavigation(document, position, token, "textDocument/declaration"),
    }),
    vscode.languages.registerImplementationProvider(selector, {
      provideImplementation: (document, position, token) =>
        requestNavigation(document, position, token, "textDocument/implementation"),
    }),
    vscode.languages.registerTypeDefinitionProvider(selector, {
      provideTypeDefinition: (document, position, token) =>
        requestNavigation(document, position, token, "textDocument/typeDefinition"),
    }),
    // Normal file references stay owned by the LanguageClient. This bridge only
    // handles the read-only view, preventing duplicate providers for Python files.
    vscode.languages.registerReferenceProvider(selector, {
      provideReferences: requestReferences,
    }),
    ...registerExternalSourceLanguageFeatureProviders({
      ...dependencies,
      sourceFileUri: externalSourceFileUri,
      textIsCurrent: (document, sourceUri) =>
        externalSourceTextIsCurrent(document, sourceUri, dependencies),
    }),
  ];
}

export async function externalSourceTextIsCurrent(
  document: vscode.TextDocument,
  sourceUri: vscode.Uri,
  dependencies: ExternalSourceNavigationDependencies,
): Promise<boolean> {
  if (document.uri.scheme !== SAGE_SOURCE_SCHEME) {
    return true;
  }
  const backingText = Buffer.from(await vscode.workspace.fs.readFile(sourceUri)).toString("utf8");
  if (document.getText() === backingText) {
    return true;
  }
  // Never forward a position from stale visible text to the current file. The
  // provider refresh is asynchronous, so the user can retry after the view catches up.
  dependencies.refreshExternalSourceDocument?.(document);
  dependencies.logger.warn("navigation", "external Sage source changed before navigation; refreshing view", {
    uri: document.uri.toString(),
    sourceUri: sourceUri.toString(),
  });
  return false;
}
