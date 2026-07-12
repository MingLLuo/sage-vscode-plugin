import * as path from "node:path";

import * as vscode from "vscode";
import type { LanguageClient } from "vscode-languageclient/node";
import type {
  Definition as ProtocolDefinition,
  DefinitionLink as ProtocolDefinitionLink,
} from "vscode-languageserver-protocol";

import type { OutputLogger } from "./extensionLogger";
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
      // Let the LSP observe a normal file: document while the visible editor remains
      // the read-only sage-source view.
      await vscode.workspace.openTextDocument(sourceUri);
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
      await vscode.workspace.openTextDocument(sourceUri);
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
  ];
}
