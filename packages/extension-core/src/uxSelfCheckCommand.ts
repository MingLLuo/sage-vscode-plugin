import * as vscode from "vscode";
import type { LanguageClient } from "vscode-languageclient/node";

import type {
  DocsStatusSummary,
  IndexStatusSummary,
} from "./environmentPresentation";
import { languageServerUriForDocument } from "./externalSourceNavigation";
import {
  executeSageCommandWithTimeout,
  RUST_LSP_COMMANDS,
} from "./languageClient";
import {
  buildQueryRequestPayload,
  diagnosticCodeLabel,
  diagnosticRangeLabel,
  formatUxSelfCheckReport,
  measureAsync,
  shouldRunFullUxSelfCheckQuery,
  type QueryResponse,
} from "./uxSelfCheck";

const UX_SELF_CHECK_REQUEST_TIMEOUT_MS = 10_000;

export interface UxSelfCheckCommandOptions {
  ensureLanguageClientReady: (action: string) => Promise<LanguageClient | undefined>;
  activeOrVisibleSageEditor: () => vscode.TextEditor | undefined;
  workspaceFolderPaths: () => string[];
  outputChannel: vscode.OutputChannel;
  updateLanguageServerStatus: (
    indexStatus: IndexStatusSummary | undefined,
    docsStatus: DocsStatusSummary | undefined,
  ) => void;
  updateStatusBar: () => void;
}

export function registerUxSelfCheckCommand(
  options: UxSelfCheckCommandOptions,
): vscode.Disposable {
  return vscode.commands.registerCommand("sage.runUxSelfCheck", async () => {
    const activeClient = await options.ensureLanguageClientReady("Running the Sage UX self check");
    if (!activeClient) {
      return;
    }

    const editor = options.activeOrVisibleSageEditor();
    if (!editor) {
      void vscode.window.showWarningMessage("Open a Sage file before running the Sage UX self check.");
      return;
    }

    const selectedText = editor.document.getText(editor.selection).trim() || undefined;
    const languageServerUri = languageServerUriForDocument(editor.document);
    const requestSelfCheck = <T>(command: string, args: unknown[] = []): Promise<T | null> =>
      executeSageCommandWithTimeout<T>(activeClient, command, args, {
        timeoutMs: UX_SELF_CHECK_REQUEST_TIMEOUT_MS,
        label: `Sage UX self-check request (${command})`,
      });
    const selfCheckStarted = Date.now();
    const queryStarted = Date.now();
    let query = await requestSelfCheck<QueryResponse>(
      RUST_LSP_COMMANDS.queryAtPosition,
      [
        buildQueryRequestPayload(
          languageServerUri.toString(),
          editor.selection.active.line,
          editor.selection.active.character,
          selectedText,
          { mode: "navigation" },
        ),
      ],
    );
    const queryMs = Date.now() - queryStarted;
    let fullQueryMs: number | undefined;
    if (shouldRunFullUxSelfCheckQuery(query, options.workspaceFolderPaths())) {
      const fullQueryStarted = Date.now();
      query = await requestSelfCheck<QueryResponse>(
        RUST_LSP_COMMANDS.queryAtPosition,
        [
          buildQueryRequestPayload(
            languageServerUri.toString(),
            editor.selection.active.line,
            editor.selection.active.character,
            selectedText,
          ),
        ],
      );
      fullQueryMs = Date.now() - fullQueryStarted;
    }
    const [indexStatusResult, docsStatusResult] = await Promise.all([
      measureAsync(() => requestSelfCheck<IndexStatusSummary>(RUST_LSP_COMMANDS.indexStatus)),
      measureAsync(() => requestSelfCheck<DocsStatusSummary>(RUST_LSP_COMMANDS.docsStatus)),
    ]);
    const indexStatus = indexStatusResult.value ?? undefined;
    const docsStatus = docsStatusResult.value ?? undefined;
    options.updateLanguageServerStatus(indexStatus, docsStatus);
    options.updateStatusBar();

    const result = formatUxSelfCheckReport({
      documentUri: editor.document.uri.toString(),
      symbol: selectedText,
      query,
      indexStatus,
      docsStatus,
      timings: {
        queryMs,
        fullQueryMs,
        indexStatusMs: indexStatusResult.elapsedMs,
        docsStatusMs: docsStatusResult.elapsedMs,
        totalMs: Date.now() - selfCheckStarted,
      },
      editorDiagnostics: vscode.languages.getDiagnostics(editor.document.uri).map((diagnostic) => ({
        source: diagnostic.source,
        code: diagnosticCodeLabel(diagnostic.code),
        severity: vscode.DiagnosticSeverity[diagnostic.severity] ?? diagnostic.severity,
        range: diagnosticRangeLabel(diagnostic.range),
        message: diagnostic.message,
      })),
    });
    options.outputChannel.clear();
    options.outputChannel.appendLine(result.report);
    options.outputChannel.show(true);
    void vscode.window.showInformationMessage(
      `Sage UX Self Check: ${result.passed}/${result.total} checks passing`,
    );
  });
}
