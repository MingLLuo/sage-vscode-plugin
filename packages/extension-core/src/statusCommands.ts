import * as vscode from "vscode";
import type { LanguageClient } from "vscode-languageclient/node";

import {
  formatEnvironmentDetails,
  formatIndexStatusMessage,
  type DocsStatusSummary,
  type EnvironmentPresentationInput,
  type IndexStatusSummary,
} from "./environmentPresentation";
import type { OutputLogger } from "./extensionLogger";
import { waitForIndexRebuild } from "./indexRebuild";
import {
  executeSageCommandWithTimeout,
  RUST_LSP_COMMANDS,
} from "./languageClient";
import type { SageSettings } from "./settingsModel";
import { formatDocsStatusReport, formatIndexStatusReport } from "./statusReports";
import { buildSupportBundle } from "./supportBundle";
import type { WorkspaceRuntimeState } from "./workspaceTrust";

const USER_STATUS_REQUEST_TIMEOUT_MS = 10_000;

export interface StatusCommandDependencies {
  context: vscode.ExtensionContext;
  outputChannel: vscode.OutputChannel;
  logger: Pick<OutputLogger, "info" | "warn">;
  ensureLanguageClientReady(action: string): Promise<LanguageClient | undefined>;
  refreshLanguageServerStatus(): Promise<void>;
  activeEditorSettings(): SageSettings;
  workspaceFolderPaths(): string[];
  currentWorkspaceRuntimeState(): WorkspaceRuntimeState;
  buildEnvironmentPresentationInput(languageServerStarting?: boolean): EnvironmentPresentationInput;
  languageClientLifecycleSnapshot(): Record<string, boolean | number>;
  languageClientState(): { available: boolean; starting: boolean };
  getIndexStatus(): IndexStatusSummary | undefined;
  setIndexStatus(status: IndexStatusSummary | undefined): void;
  getDocsStatus(): DocsStatusSummary | undefined;
  setDocsStatus(status: DocsStatusSummary | undefined): void;
  updateStatusBar(): void;
}

export function registerStatusCommands(
  dependencies: StatusCommandDependencies,
): vscode.Disposable[] {
  const requestStatus = <T>(
    activeClient: LanguageClient,
    command: string,
    label: string,
  ): Promise<T | null> => executeSageCommandWithTimeout<T>(
    activeClient,
    command,
    [],
    { timeoutMs: USER_STATUS_REQUEST_TIMEOUT_MS, label },
  );
  const showStatusReport = (title: string, report: string, notification: string): void => {
    dependencies.outputChannel.appendLine(`## ${title}`);
    dependencies.outputChannel.appendLine(report);
    dependencies.outputChannel.appendLine("");
    dependencies.outputChannel.show(true);
    void vscode.window.showInformationMessage(notification);
  };
  const reportRequestFailure = async (action: string, error: unknown): Promise<void> => {
    dependencies.logger.warn("status", `${action} failed`, { error: String(error) });
    const selectedAction = await vscode.window.showErrorMessage(
      `${action} did not complete: ${String(error)}`,
      "Restart Language Server",
    );
    if (selectedAction === "Restart Language Server") {
      await vscode.commands.executeCommand("sage.restartLanguageServer");
    }
  };

  return [
    vscode.commands.registerCommand("sage.showEnvironmentDetails", async () => {
      showStatusReport(
        "Sage Environment Details",
        formatEnvironmentDetails(dependencies.buildEnvironmentPresentationInput()),
        "Sage environment details written to the Sage output channel.",
      );
    }),
    vscode.commands.registerCommand("sage.showIndexStatus", async () => {
      const activeClient = await dependencies.ensureLanguageClientReady("Showing the Sage index status");
      if (!activeClient) {
        return;
      }
      let status: IndexStatusSummary | null;
      try {
        status = await requestStatus<IndexStatusSummary>(
          activeClient,
          RUST_LSP_COMMANDS.indexStatus,
          "Sage index status request",
        );
      } catch (error) {
        await reportRequestFailure("Showing the Sage index status", error);
        return;
      }
      dependencies.setIndexStatus(status ?? undefined);
      dependencies.updateStatusBar();
      showStatusReport(
        "Sage Index Status",
        formatIndexStatusReport(status),
        "Sage index status written to the Sage output channel.",
      );
      return status;
    }),
    vscode.commands.registerCommand("sage.showDocsStatus", async () => {
      const activeClient = await dependencies.ensureLanguageClientReady("Showing the Sage docs status");
      if (!activeClient) {
        return;
      }
      let status: DocsStatusSummary | null;
      try {
        status = await requestStatus<DocsStatusSummary>(
          activeClient,
          RUST_LSP_COMMANDS.docsStatus,
          "Sage documentation status request",
        );
      } catch (error) {
        await reportRequestFailure("Showing the Sage documentation status", error);
        return;
      }
      dependencies.setDocsStatus(status ?? undefined);
      dependencies.updateStatusBar();
      showStatusReport(
        "Sage Documentation Status",
        formatDocsStatusReport(status),
        "Sage documentation status written to the Sage output channel.",
      );
    }),
    vscode.commands.registerCommand("sage.copySupportBundle", async () => {
      if (dependencies.languageClientState().available) {
        await dependencies.refreshLanguageServerStatus();
      }
      const activeDocument = vscode.window.activeTextEditor?.document;
      const workspaceRuntimeState = dependencies.currentWorkspaceRuntimeState();
      const languageClientState = dependencies.languageClientState();
      const bundle = buildSupportBundle({
        generatedAt: new Date().toISOString(),
        extension: {
          id: dependencies.context.extension.id,
          version: String(dependencies.context.extension.packageJSON?.version ?? "unknown"),
        },
        host: {
          vscodeVersion: vscode.version,
          platform: process.platform,
          arch: process.arch,
          nodeVersion: process.version,
        },
        workspace: {
          folders: dependencies.workspaceFolderPaths(),
          trusted: workspaceRuntimeState.trusted,
          hasVirtualWorkspace: workspaceRuntimeState.hasVirtualWorkspace,
        },
        activeDocument: activeDocument
          ? {
            uri: activeDocument.uri.toString(),
            languageId: activeDocument.languageId,
            scheme: activeDocument.uri.scheme,
          }
          : undefined,
        settings: dependencies.activeEditorSettings(),
        environment: dependencies.buildEnvironmentPresentationInput(
          languageClientState.starting && !languageClientState.available,
        ),
        lifecycle: dependencies.languageClientLifecycleSnapshot(),
        indexStatus: dependencies.getIndexStatus(),
        docsStatus: dependencies.getDocsStatus(),
      });
      await vscode.env.clipboard.writeText(bundle);
      dependencies.outputChannel.clear();
      dependencies.outputChannel.appendLine(bundle);
      dependencies.outputChannel.show(true);
      void vscode.window.showInformationMessage(
        "Sage support bundle copied. It includes paths and settings, but no source contents, selected text, or environment variables.",
      );
    }),
    vscode.commands.registerCommand("sage.rebuildIndex", async () => {
      const activeClient = await dependencies.ensureLanguageClientReady("Rebuilding the Sage index");
      if (!activeClient) {
        return;
      }
      try {
        const baselineStatus = await requestStatus<IndexStatusSummary>(
          activeClient,
          RUST_LSP_COMMANDS.indexStatus,
          "Sage index rebuild baseline request",
        );
        const baselineGeneration = baselineStatus?.generation;
        if (typeof baselineGeneration !== "number" || !Number.isFinite(baselineGeneration)) {
          throw new Error("Cannot rebuild the Sage index because its current generation is unavailable.");
        }

        const scheduledStatus = await requestStatus<IndexStatusSummary>(
          activeClient,
          RUST_LSP_COMMANDS.rebuildIndex,
          "Sage index rebuild scheduling request",
        );
        dependencies.setIndexStatus(scheduledStatus ?? baselineStatus ?? undefined);
        dependencies.updateStatusBar();

        const rebuiltStatus = await vscode.window.withProgress(
          {
            location: vscode.ProgressLocation.Notification,
            title: "Rebuilding Sage index",
            cancellable: false,
          },
          async (progress) => waitForIndexRebuild({
            baselineGeneration,
            readStatus: () => requestStatus<IndexStatusSummary>(
              activeClient,
              RUST_LSP_COMMANDS.indexStatus,
              "Sage index rebuild status request",
            ),
            reschedule: async () => {
              const retryStatus = await requestStatus<IndexStatusSummary>(
                activeClient,
                RUST_LSP_COMMANDS.rebuildIndex,
                "Sage index rebuild reschedule request",
              );
              dependencies.setIndexStatus(retryStatus ?? dependencies.getIndexStatus());
              dependencies.updateStatusBar();
            },
            onReschedule: (attempt, status) => {
              dependencies.logger.info("index", "rescheduling superseded index rebuild", {
                attempt,
                generation: status.generation,
                lastOperation: status.last_operation,
              });
              progress.report({
                message: `index changed during rebuild; retrying (${attempt})`,
              });
            },
            onStatus: (status) => {
              dependencies.setIndexStatus(status);
              dependencies.updateStatusBar();
              progress.report({
                message: status.pending_jobs
                  ? `${status.pending_jobs} indexing job${status.pending_jobs === 1 ? "" : "s"} pending`
                  : `waiting for generation ${baselineGeneration + 1}`,
              });
            },
          }),
        );
        dependencies.setIndexStatus(rebuiltStatus);
        dependencies.updateStatusBar();
        void vscode.window.showInformationMessage(
          `Index rebuilt: ${formatIndexStatusMessage(rebuiltStatus)}`,
        );
        return rebuiltStatus;
      } catch (error) {
        await reportRequestFailure("Sage index rebuild", error);
        return;
      }
    }),
  ];
}
