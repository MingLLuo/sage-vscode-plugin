import * as path from "node:path";

import * as vscode from "vscode";
import type { LanguageClient } from "vscode-languageclient/node";
import {
  CallHierarchyIncomingCallsRequest,
  CallHierarchyOutgoingCallsRequest,
  CallHierarchyPrepareRequest,
  DocumentLinkRequest,
  HoverRequest,
  SignatureHelpRequest,
  type CallHierarchyIncomingCall as ProtocolCallHierarchyIncomingCall,
  type CallHierarchyItem as ProtocolCallHierarchyItem,
  type CallHierarchyOutgoingCall as ProtocolCallHierarchyOutgoingCall,
  type DocumentLink as ProtocolDocumentLink,
  type Hover as ProtocolHover,
  type SignatureHelp as ProtocolSignatureHelp,
  type SignatureHelpTriggerKind as ProtocolSignatureHelpTriggerKind,
} from "vscode-languageserver-protocol";

import type { ExternalSourceNavigationDependencies } from "./externalSourceNavigation";
import { rewriteExternalSourceUri } from "./languageClient";
import { protocolItemWithUri } from "./externalSourceProtocol";
import { SAGE_SOURCE_SCHEME } from "./sageSourceView";

interface ExternalSourceLanguageFeatureDependencies extends ExternalSourceNavigationDependencies {
  sourceFileUri(document: vscode.TextDocument): vscode.Uri | undefined;
  textIsCurrent(document: vscode.TextDocument, sourceUri: vscode.Uri): Promise<boolean>;
}

export function registerExternalSourceLanguageFeatureProviders(
  dependencies: ExternalSourceLanguageFeatureDependencies,
): vscode.Disposable[] {
  const workspaceFolders = (): string[] =>
    vscode.workspace.workspaceFolders?.map((folder) => folder.uri.fsPath) ?? [];

  const requestHover = async (
    document: vscode.TextDocument,
    position: vscode.Position,
    token: vscode.CancellationToken,
  ): Promise<vscode.Hover | null> => {
    const sourceUri = externalDocumentSourceUri(document, dependencies);
    if (!sourceUri) {
      return null;
    }
    try {
      const activeClient = await dependencies.ensureLanguageClientReady("Inspecting Sage source");
      if (!activeClient || token.isCancellationRequested || !(await dependencies.textIsCurrent(document, sourceUri))) {
        return null;
      }
      const response = await activeClient.sendRequest<ProtocolHover | null>(
        HoverRequest.method,
        positionRequest(sourceUri, position),
        token,
      );
      return activeClient.protocol2CodeConverter.asHover(response) ?? null;
    } catch (error) {
      logPositionFailure(dependencies, "hover", document, sourceUri, position, error);
      return null;
    }
  };

  const requestSignatureHelp = async (
    document: vscode.TextDocument,
    position: vscode.Position,
    token: vscode.CancellationToken,
    context: vscode.SignatureHelpContext,
  ): Promise<vscode.SignatureHelp | null> => {
    const sourceUri = externalDocumentSourceUri(document, dependencies);
    if (!sourceUri) {
      return null;
    }
    try {
      const activeClient = await dependencies.ensureLanguageClientReady("Inspecting a Sage call signature");
      if (!activeClient || token.isCancellationRequested || !(await dependencies.textIsCurrent(document, sourceUri))) {
        return null;
      }
      const response = await activeClient.sendRequest<ProtocolSignatureHelp | null>(
        SignatureHelpRequest.method,
        {
          ...positionRequest(sourceUri, position),
          context: {
            triggerKind: context.triggerKind as ProtocolSignatureHelpTriggerKind,
            triggerCharacter: context.triggerCharacter,
            isRetrigger: context.isRetrigger,
          },
        },
        token,
      );
      return (await activeClient.protocol2CodeConverter.asSignatureHelp(response, token)) ?? null;
    } catch (error) {
      logPositionFailure(dependencies, "signature help", document, sourceUri, position, error);
      return null;
    }
  };

  const requestDocumentLinks = async (
    document: vscode.TextDocument,
    token: vscode.CancellationToken,
  ): Promise<vscode.DocumentLink[]> => {
    const sourceUri = externalDocumentSourceUri(document, dependencies);
    if (!sourceUri) {
      return [];
    }
    try {
      const activeClient = await dependencies.ensureLanguageClientReady("Finding linked Sage source files");
      if (!activeClient || token.isCancellationRequested || !(await dependencies.textIsCurrent(document, sourceUri))) {
        return [];
      }
      const response = await activeClient.sendRequest<ProtocolDocumentLink[] | null>(
        DocumentLinkRequest.method,
        { textDocument: { uri: sourceUri.toString() } },
        token,
      );
      const links = (await activeClient.protocol2CodeConverter.asDocumentLinks(response, token)) ?? [];
      const roots = workspaceFolders();
      for (const link of links) {
        if (link.target) {
          link.target = rewriteExternalSourceUri(link.target, roots);
        }
      }
      return links;
    } catch (error) {
      dependencies.logger.warn("navigation", "external Sage source document links failed", {
        uri: document.uri.toString(),
        sourceUri: sourceUri.toString(),
        error: String(error),
      });
      return [];
    }
  };

  const prepareCallHierarchy = async (
    document: vscode.TextDocument,
    position: vscode.Position,
    token: vscode.CancellationToken,
  ): Promise<vscode.CallHierarchyItem[]> => {
    const sourceUri = externalDocumentSourceUri(document, dependencies);
    if (!sourceUri) {
      return [];
    }
    try {
      const activeClient = await dependencies.ensureLanguageClientReady("Preparing Sage call hierarchy");
      if (!activeClient || token.isCancellationRequested || !(await dependencies.textIsCurrent(document, sourceUri))) {
        return [];
      }
      const response = await activeClient.sendRequest<ProtocolCallHierarchyItem[] | null>(
        CallHierarchyPrepareRequest.method,
        positionRequest(sourceUri, position),
        token,
      );
      const items = (await activeClient.protocol2CodeConverter.asCallHierarchyItems(response, token)) ?? [];
      rewriteCallHierarchyItems(items, workspaceFolders());
      return items;
    } catch (error) {
      logPositionFailure(dependencies, "call hierarchy preparation", document, sourceUri, position, error);
      return [];
    }
  };

  const requestIncomingCalls = async (
    item: vscode.CallHierarchyItem,
    token: vscode.CancellationToken,
  ): Promise<vscode.CallHierarchyIncomingCall[]> => {
    try {
      const activeClient = await dependencies.ensureLanguageClientReady("Finding incoming Sage calls");
      if (!activeClient || token.isCancellationRequested || !(await callHierarchyItemTextIsCurrent(item, dependencies))) {
        return [];
      }
      const response = await activeClient.sendRequest<ProtocolCallHierarchyIncomingCall[] | null>(
        CallHierarchyIncomingCallsRequest.method,
        { item: callHierarchyItemForServer(activeClient, item) },
        token,
      );
      const calls = (await activeClient.protocol2CodeConverter.asCallHierarchyIncomingCalls(response, token)) ?? [];
      rewriteCallHierarchyItems(calls.map((call) => call.from), workspaceFolders());
      return calls;
    } catch (error) {
      logHierarchyExpansionFailure(dependencies, "incoming calls", item, error);
      return [];
    }
  };

  const requestOutgoingCalls = async (
    item: vscode.CallHierarchyItem,
    token: vscode.CancellationToken,
  ): Promise<vscode.CallHierarchyOutgoingCall[]> => {
    try {
      const activeClient = await dependencies.ensureLanguageClientReady("Finding outgoing Sage calls");
      if (!activeClient || token.isCancellationRequested || !(await callHierarchyItemTextIsCurrent(item, dependencies))) {
        return [];
      }
      const response = await activeClient.sendRequest<ProtocolCallHierarchyOutgoingCall[] | null>(
        CallHierarchyOutgoingCallsRequest.method,
        { item: callHierarchyItemForServer(activeClient, item) },
        token,
      );
      const calls = (await activeClient.protocol2CodeConverter.asCallHierarchyOutgoingCalls(response, token)) ?? [];
      rewriteCallHierarchyItems(calls.map((call) => call.to), workspaceFolders());
      return calls;
    } catch (error) {
      logHierarchyExpansionFailure(dependencies, "outgoing calls", item, error);
      return [];
    }
  };

  const selector: vscode.DocumentSelector = [{ scheme: SAGE_SOURCE_SCHEME }];
  return [
    vscode.languages.registerHoverProvider(selector, { provideHover: requestHover }),
    vscode.languages.registerSignatureHelpProvider(
      selector,
      { provideSignatureHelp: requestSignatureHelp },
      { triggerCharacters: ["(", ","], retriggerCharacters: [","] },
    ),
    vscode.languages.registerDocumentLinkProvider(selector, {
      provideDocumentLinks: requestDocumentLinks,
    }),
    vscode.languages.registerCallHierarchyProvider(selector, {
      prepareCallHierarchy,
      provideCallHierarchyIncomingCalls: requestIncomingCalls,
      provideCallHierarchyOutgoingCalls: requestOutgoingCalls,
    }),
  ];
}

function externalDocumentSourceUri(
  document: vscode.TextDocument,
  dependencies: ExternalSourceLanguageFeatureDependencies,
): vscode.Uri | undefined {
  return dependencies.isExternalSourceDocument(document)
    ? dependencies.sourceFileUri(document)
    : undefined;
}

function positionRequest(sourceUri: vscode.Uri, position: vscode.Position) {
  return {
    textDocument: { uri: sourceUri.toString() },
    position: { line: position.line, character: position.character },
  };
}

function callHierarchyItemForServer(
  client: LanguageClient,
  item: vscode.CallHierarchyItem,
): ProtocolCallHierarchyItem {
  const converted = client.code2ProtocolConverter.asCallHierarchyItem(item);
  const backingUri = sageSourceBackingFileUri(item.uri);
  return backingUri ? protocolItemWithUri(converted, backingUri.toString()) : converted;
}

function rewriteCallHierarchyItems(
  items: readonly vscode.CallHierarchyItem[],
  workspaceFolders: readonly string[],
): void {
  for (const item of items) {
    item.uri = rewriteExternalSourceUri(item.uri, workspaceFolders);
  }
}

function sageSourceBackingFileUri(uri: vscode.Uri): vscode.Uri | undefined {
  if (uri.scheme !== SAGE_SOURCE_SCHEME || !path.isAbsolute(uri.path)) {
    return undefined;
  }
  return vscode.Uri.file(uri.path);
}

async function callHierarchyItemTextIsCurrent(
  item: vscode.CallHierarchyItem,
  dependencies: ExternalSourceLanguageFeatureDependencies,
): Promise<boolean> {
  const sourceUri = sageSourceBackingFileUri(item.uri);
  if (!sourceUri) {
    return true;
  }
  const document = vscode.workspace.textDocuments.find(
    (candidate) => candidate.uri.toString() === item.uri.toString(),
  );
  return document ? dependencies.textIsCurrent(document, sourceUri) : true;
}

function logPositionFailure(
  dependencies: ExternalSourceLanguageFeatureDependencies,
  feature: string,
  document: vscode.TextDocument,
  sourceUri: vscode.Uri,
  position: vscode.Position,
  error: unknown,
): void {
  dependencies.logger.warn("navigation", `external Sage source ${feature} failed`, {
    uri: document.uri.toString(),
    sourceUri: sourceUri.toString(),
    line: position.line,
    character: position.character,
    error: String(error),
  });
}

function logHierarchyExpansionFailure(
  dependencies: ExternalSourceLanguageFeatureDependencies,
  feature: string,
  item: vscode.CallHierarchyItem,
  error: unknown,
): void {
  dependencies.logger.warn("navigation", `external Sage source ${feature} failed`, {
    uri: item.uri.toString(),
    symbol: item.name,
    error: String(error),
  });
}
