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
import { executeSageCommand, RUST_LSP_COMMANDS } from "./languageClient";
import type { SageSettings } from "./settingsModel";
import { formatDocsStatusReport, formatIndexStatusReport } from "./statusReports";
import { buildSupportBundle } from "./supportBundle";
import type { WorkspaceRuntimeState } from "./workspaceTrust";

export interface StatusCommandDependencies {
  context: vscode.ExtensionContext;
  outputChannel: vscode.OutputChannel;
  logger: Pick<OutputLogger, "info">;
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
  const showStatusReport = (title: string, report: string, notification: string): void => {
    dependencies.outputChannel.appendLine(`## ${title}`);
    dependencies.outputChannel.appendLine(report);
    dependencies.outputChannel.appendLine("");
    dependencies.outputChannel.show(true);
    void vscode.window.showInformationMessage(notification);
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
      const status = await executeSageCommand<IndexStatusSummary>(activeClient, RUST_LSP_COMMANDS.indexStatus);
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
      const status = await executeSageCommand<DocsStatusSummary>(activeClient, RUST_LSP_COMMANDS.docsStatus);
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
      const baselineStatus = await executeSageCommand<IndexStatusSummary>(
        activeClient,
        RUST_LSP_COMMANDS.indexStatus,
      );
      const baselineGeneration = baselineStatus?.generation;
      if (typeof baselineGeneration !== "number" || !Number.isFinite(baselineGeneration)) {
        throw new Error("Cannot rebuild the Sage index because its current generation is unavailable.");
      }

      const scheduledStatus = await executeSageCommand<IndexStatusSummary>(
        activeClient,
        RUST_LSP_COMMANDS.rebuildIndex,
      );
      dependencies.setIndexStatus(scheduledStatus ?? baselineStatus ?? undefined);
      dependencies.updateStatusBar();

      try {
        const rebuiltStatus = await vscode.window.withProgress(
          {
            location: vscode.ProgressLocation.Notification,
            title: "Rebuilding Sage index",
            cancellable: false,
          },
          async (progress) => waitForIndexRebuild({
            baselineGeneration,
            readStatus: () => executeSageCommand<IndexStatusSummary>(
              activeClient,
              RUST_LSP_COMMANDS.indexStatus,
            ),
            reschedule: async () => {
              const retryStatus = await executeSageCommand<IndexStatusSummary>(
                activeClient,
                RUST_LSP_COMMANDS.rebuildIndex,
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
        void vscode.window.showErrorMessage(`Sage index rebuild did not complete: ${String(error)}`);
        throw error;
      }
    }),
  ];
}
