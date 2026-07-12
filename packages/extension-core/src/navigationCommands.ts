import * as vscode from "vscode";
import type { LanguageClient } from "vscode-languageclient/node";

import type { DocumentationPanel } from "./docsPanel";
import {
  documentationFallbackActions,
  documentationFallbackCommand,
  documentationFallbackMessage,
} from "./documentationFallback";
import { renderDocumentationMarkdown } from "./documentationRequest";
import type { OutputLogger } from "./extensionLogger";
import { languageServerUriForDocument } from "./externalSourceNavigation";
import { requestDocumentation } from "./languageClient";
import { showReferencesQuickPick } from "./referenceQuickPick";

export interface NavigationCommandDependencies {
  docsPanel: DocumentationPanel;
  logger: Pick<OutputLogger, "error" | "info" | "warn">;
  ensureLanguageClientReady(action: string): Promise<LanguageClient | undefined>;
  activeOrVisibleSageEditor(): vscode.TextEditor | undefined;
}

export function registerNavigationCommands(
  dependencies: NavigationCommandDependencies,
): vscode.Disposable[] {
  return [
    vscode.commands.registerCommand("sage.showDocumentation", async () => {
      const activeClient = await dependencies.ensureLanguageClientReady("Showing Sage documentation");
      if (!activeClient) {
        return;
      }

      const editor = dependencies.activeOrVisibleSageEditor();
      if (!editor) {
        void vscode.window.showWarningMessage("Open a Sage file to request documentation.");
        return;
      }

      const selectedText = editor.document.getText(editor.selection).trim() || undefined;
      const languageServerUri = languageServerUriForDocument(editor.document);
      const result = await requestDocumentation(
        activeClient,
        languageServerUri.toString(),
        editor.selection.active.line,
        editor.selection.active.character,
        selectedText,
      );

      if (!result) {
        const selectedAction = await vscode.window.showInformationMessage(
          documentationFallbackMessage(selectedText),
          ...documentationFallbackActions(),
        );
        if (selectedAction) {
          await vscode.commands.executeCommand(documentationFallbackCommand(selectedAction));
        }
        return;
      }

      dependencies.docsPanel.show(`Docs: ${result.symbol}`, renderDocumentationMarkdown(result));
    }),
    vscode.commands.registerCommand("sage.findReferences", async () => {
      if (!(await dependencies.ensureLanguageClientReady("Finding Sage references"))) {
        return [];
      }

      const editor = dependencies.activeOrVisibleSageEditor();
      if (!editor) {
        dependencies.logger.warn("navigation", "Sage references skipped because no Sage editor is visible", {
          activeLanguageId: vscode.window.activeTextEditor?.document.languageId ?? "none",
          activeUri: vscode.window.activeTextEditor?.document.uri.toString() ?? "none",
        });
        void vscode.window.showWarningMessage("Open a Sage file before finding references.");
        return;
      }

      let locations: vscode.Location[];
      try {
        locations = (await vscode.commands.executeCommand<vscode.Location[]>(
          "vscode.executeReferenceProvider",
          editor.document.uri,
          editor.selection.active,
        )) ?? [];
      } catch (error) {
        dependencies.logger.error("navigation", "Sage references request failed", {
          uri: editor.document.uri.toString(),
          line: editor.selection.active.line,
          character: editor.selection.active.character,
          error: String(error),
        });
        void vscode.window.showErrorMessage(`Sage references failed: ${String(error)}`);
        return [];
      }

      dependencies.logger.info("navigation", "Sage references resolved", {
        uri: editor.document.uri.toString(),
        languageId: editor.document.languageId,
        workspaceFolder: vscode.workspace.getWorkspaceFolder(editor.document.uri)?.uri.toString() ?? "external",
        line: editor.selection.active.line,
        character: editor.selection.active.character,
        count: locations.length,
      });
      if (locations.length === 0) {
        void vscode.window.showInformationMessage("No Sage references found for the current symbol.");
        return [];
      }

      try {
        await vscode.commands.executeCommand(
          "editor.action.peekLocations",
          editor.document.uri,
          editor.selection.active,
          locations,
          "peek",
        );
      } catch (error) {
        dependencies.logger.warn("navigation", "Peek references failed; falling back to quick pick", {
          uri: editor.document.uri.toString(),
          count: locations.length,
          error: String(error),
        });
        await showReferencesQuickPick(locations);
      }
      return locations;
    }),
  ];
}
