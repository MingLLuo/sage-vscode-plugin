import * as path from "node:path";

import * as vscode from "vscode";
import type { LanguageClient } from "vscode-languageclient/node";

import {
  isSageDocumentLanguage,
  shouldAutoStartLanguageClient,
  shouldExposeSageExperience,
} from "./activationPolicy";
import { readSettings } from "./configuration";
import { SageCellCodeLensProvider } from "./cellCodeLens";
import { DocumentationPanel } from "./docsPanel";
import { renderDocumentationMarkdown } from "./documentationRequest";
import {
  formatEnvironmentDetails,
  formatIndexStatusMessage,
  formatStatusBarText,
  formatStatusBarTooltip,
  buildIndexMaintenanceNotice,
  type DocsStatusSummary,
  type EnvironmentPresentationInput,
  type IndexStatusSummary,
} from "./environmentPresentation";
import { createOutputLogger } from "./extensionLogger";
import { createLanguageClient, executeSageCommand, requestDocumentation, RUST_LSP_COMMANDS, rustIndexCacheDir } from "./languageClient";
import { formatDocsStatusReport, formatIndexStatusReport } from "./statusReports";
import {
  DEFAULT_INDEX_CACHE_KEEP_LATEST_DATABASES,
  DEFAULT_INDEX_CACHE_MAX_AGE_DAYS,
  DEFAULT_INDEX_CACHE_MAX_TOTAL_BYTES,
  DEFAULT_INDEX_CACHE_ORPHAN_MAX_AGE_DAYS,
  DEFAULT_INDEX_CACHE_SIZE_PRUNE_MIN_AGE_DAYS,
  maintainIndexCache,
} from "./indexCacheMaintenance";
import { currentSageCell } from "./sageCells";
import {
  isLspLocationPayload,
  type LspLocationPayload,
} from "./sageNavigation";
import {
  locationFromLspPayload,
  showReferencesQuickPick,
} from "./referenceQuickPick";
import {
  SageSourceTextDocumentProvider,
  SAGE_SOURCE_SCHEME,
} from "./sageSourceView";
import {
  discoverInterpreterCandidates,
  type InterpreterCandidate,
} from "./interpreterDiscovery";
import { shouldRestartLanguageServer } from "./serverRestart";
import { SageTerminalManager } from "./terminalManager";
import {
  buildQueryRequestPayload,
  formatUxSelfCheckReport,
  type QueryResponse,
} from "./uxSelfCheck";
import { buildSupportBundle } from "./supportBundle";
import {
  formatWorkspaceRuntimeUnavailableMessage,
  isWorkspaceRuntimeAvailable,
  type WorkspaceRuntimeState,
} from "./workspaceTrust";
import {
  buildWorkspaceInitializationData,
  discoverSourceRoots,
  discoverSourceRootsAsync,
  resolveRuntimePythonPaths,
} from "./workspaceDiscovery";
import {
  buildWorkspaceConfigurationUpdates,
  recommendedWorkspaceProfile,
  WORKSPACE_CONFIGURATION_PROFILES,
  type WorkspaceConfigurationProfile,
  type WorkspaceConfigurationProfileId,
} from "./workspaceConfigurator";

let client: LanguageClient | undefined;
let languageClientOperation: Promise<void> | undefined;
let languageClientRestartQueued = false;
let languageClientManagedShutdown = false;
let languageClientLaunchCount = 0;
let languageClientManagedCloseCount = 0;
let languageClientUnexpectedCloseCount = 0;
let configurationProfileUpdateDepth = 0;
let suppressedConfigurationRestartCount = 0;
let lastIndexStatus: IndexStatusSummary | undefined;
let lastDocsStatus: DocsStatusSummary | undefined;
let languageServerStatusRefreshTimer: ReturnType<typeof setInterval> | undefined;
let languageServerStatusRefreshAttempts = 0;
let languageServerStatusRefreshInFlight = false;
let slowLanguageServerNoticeTimer: ReturnType<typeof setTimeout> | undefined;
let slowLanguageServerNoticeShown = false;
let runtimeDiscoveredSourceRoots: string[] = [];
let runtimeSourceRootDiscoveryOperation: Promise<void> | undefined;
let runtimeSourceRootDiscoveryGeneration = 0;
const shownIndexMaintenanceNotices = new Set<string>();

const LANGUAGE_SERVER_STATUS_REFRESH_INTERVAL_MS = 1500;
const LANGUAGE_SERVER_STATUS_REFRESH_MAX_ATTEMPTS = 12;
const SLOW_LANGUAGE_SERVER_NOTICE_MS = 8000;
const GETTING_STARTED_WALKTHROUGH_ID = "gettingStarted";

interface RunCurrentCellTarget {
  uri?: vscode.Uri;
  line?: number;
}

interface TestConfigureWorkspaceProfileResult {
  profileId: WorkspaceConfigurationProfileId;
  updates: Array<{ setting: string; value: unknown }>;
}

function isUnregisteredConfigurationError(error: unknown): boolean {
  return error instanceof Error && error.message.includes("not a registered configuration");
}

function formatLoggedConfigurationValue(value: unknown): string {
  if (Array.isArray(value)) {
    return value.join(",");
  }
  if (value && typeof value === "object") {
    return JSON.stringify(value);
  }
  return String(value);
}

async function updateWorkspaceSettingsJson(
  workspaceFolderUri: vscode.Uri,
  setting: string,
  value: unknown,
): Promise<void> {
  const vscodeDirectoryUri = vscode.Uri.joinPath(workspaceFolderUri, ".vscode");
  const settingsUri = vscode.Uri.joinPath(vscodeDirectoryUri, "settings.json");
  let settings: Record<string, unknown> = {};

  try {
    const existing = await vscode.workspace.fs.readFile(settingsUri);
    const text = Buffer.from(existing).toString("utf8").trim();
    if (text.length > 0) {
      const parsed = JSON.parse(text);
      if (parsed && typeof parsed === "object" && !Array.isArray(parsed)) {
        settings = parsed as Record<string, unknown>;
      }
    }
  } catch (error) {
    if (!(error instanceof vscode.FileSystemError && error.code === "FileNotFound")) {
      throw error;
    }
  }

  settings[setting] = value;
  await vscode.workspace.fs.createDirectory(vscodeDirectoryUri);
  await vscode.workspace.fs.writeFile(
    settingsUri,
    Buffer.from(`${JSON.stringify(settings, null, 2)}\n`, "utf8"),
  );
}

function currentWorkspaceRuntimeState(): WorkspaceRuntimeState {
  return {
    trusted: vscode.workspace.isTrusted,
    hasVirtualWorkspace: (vscode.workspace.workspaceFolders ?? []).some((folder) => folder.uri.scheme !== "file"),
  };
}

function isSageDocument(document: vscode.TextDocument | undefined): boolean {
  if (!document) {
    return false;
  }
  const workspaceFolder = vscode.workspace.getWorkspaceFolder(document.uri);
  const settings = readSettings(workspaceFolder);
  if (isSageDocumentLanguage(document.languageId, settings.pythonFilesEnabled)) {
    return true;
  }
  if (document.uri.scheme !== "file" || document.languageId !== "python") {
    return false;
  }
  return sourceRootContainsDocument(effectiveSourceRootPaths(settings), document.uri.fsPath);
}

function isExternalSageSourceDocument(document: vscode.TextDocument): boolean {
  if (document.uri.scheme !== "file" || document.languageId !== "python") {
    return false;
  }
  if (vscode.workspace.getWorkspaceFolder(document.uri)) {
    return false;
  }
  const settings = readSettings(vscode.workspace.workspaceFolders?.[0]);
  return sourceRootContainsDocument(effectiveSourceRootPaths(settings), document.uri.fsPath);
}

function effectiveSourceRootPaths(settings: ReturnType<typeof readSettings>): string[] {
  const workspaceFolders = workspaceFolderPaths();
  const configuredRoots = effectiveInitializationSourceRoots(settings).flatMap((root) => {
    if (path.isAbsolute(root)) {
      return [root];
    }
    if (workspaceFolders.length === 0) {
      return [root];
    }
    return workspaceFolders.map((folder) => path.resolve(folder, root));
  });
  const indexedRoots = (lastIndexStatus?.source_root_fingerprints ?? [])
    .map((fingerprint) => fingerprint.root)
    .filter((root): root is string => Boolean(root));

  const normalized = [...configuredRoots, ...indexedRoots]
    .map(normalizeSourceRootPath)
    .filter((root): root is string => Boolean(root));
  return [...new Set(normalized)];
}

function effectiveInitializationSourceRoots(settings: ReturnType<typeof readSettings>): string[] {
  return dedupeStrings([...settings.sourceRoots, ...runtimeDiscoveredSourceRoots]);
}

function normalizeSourceRootPath(root: string): string | undefined {
  let candidate = root;
  if (candidate.startsWith("file://")) {
    try {
      candidate = vscode.Uri.parse(candidate).fsPath;
    } catch {
      return undefined;
    }
  }
  const resolved = path.resolve(candidate);
  const trimmed = resolved.replace(/[\\/]+$/, "");
  return trimmed || resolved;
}

function dedupeStrings(values: readonly string[]): string[] {
  return [...new Set(values)];
}

function sourceRootContainsDocument(sourceRoots: readonly string[], documentPath: string): boolean {
  const normalizedDocumentPath = normalizeSourceRootPath(documentPath);
  if (!normalizedDocumentPath) {
    return false;
  }
  return sourceRoots.some((root) => {
    const normalizedRoot = normalizeSourceRootPath(root);
    return Boolean(
      normalizedRoot
      && (normalizedDocumentPath === normalizedRoot
        || normalizedDocumentPath.startsWith(`${normalizedRoot}${path.sep}`)),
    );
  });
}

function diagnosticCodeLabel(code: vscode.Diagnostic["code"]): string | number | undefined {
  if (typeof code === "string" || typeof code === "number") {
    return code;
  }
  if (code && typeof code === "object" && "value" in code) {
    const value = code.value;
    return typeof value === "string" || typeof value === "number" ? value : String(value);
  }
  return undefined;
}

function diagnosticRangeLabel(range: vscode.Range): string {
  return `${range.start.line}:${range.start.character}-${range.end.line}:${range.end.character}`;
}

function clearLanguageServerStatusRefresh(): void {
  if (languageServerStatusRefreshTimer) {
    clearInterval(languageServerStatusRefreshTimer);
    languageServerStatusRefreshTimer = undefined;
  }
  languageServerStatusRefreshAttempts = 0;
  languageServerStatusRefreshInFlight = false;
}

function clearSlowLanguageServerNotice(): void {
  if (slowLanguageServerNoticeTimer) {
    clearTimeout(slowLanguageServerNoticeTimer);
    slowLanguageServerNoticeTimer = undefined;
  }
}

function languageClientLifecycleSnapshot(): Record<string, boolean | number> {
  return {
    launchCount: languageClientLaunchCount,
    managedCloseCount: languageClientManagedCloseCount,
    unexpectedCloseCount: languageClientUnexpectedCloseCount,
    managedShutdownActive: languageClientManagedShutdown,
    restartQueued: languageClientRestartQueued,
    operationInFlight: Boolean(languageClientOperation),
    hasClient: Boolean(client),
    configurationProfileUpdateDepth,
    suppressedConfigurationRestartCount,
  };
}

function workspaceFolderPaths(): string[] {
  return vscode.workspace.workspaceFolders?.map((folder) => folder.uri.fsPath) ?? [];
}

function shouldRunFullUxSelfCheckQuery(query: QueryResponse | null | undefined): boolean {
  const definitionPath = query?.definition?.path;
  if (!definitionPath) {
    return false;
  }
  const normalizedDefinition = path.resolve(definitionPath);
  return workspaceFolderPaths()
    .map((folder) => path.resolve(folder))
    .some((folder) => normalizedDefinition === folder || normalizedDefinition.startsWith(`${folder}${path.sep}`));
}

export async function activate(context: vscode.ExtensionContext): Promise<void> {
  const outputChannel = vscode.window.createOutputChannel("Sage");
  const languageOutputChannel = vscode.window.createOutputChannel("Sage Language Server");
  const logger = createOutputLogger(outputChannel);
  const docsPanel = new DocumentationPanel();
  const terminalManager = new SageTerminalManager();
  const statusBarItem = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Left, 100);
  const cellCodeLensProvider = new SageCellCodeLensProvider({
    isEnabled: (document) => {
      const settings = readSettings(vscode.workspace.getWorkspaceFolder(document.uri));
      return settings.showCellCodeLens
        && isSageDocument(document)
        && isWorkspaceRuntimeAvailable(currentWorkspaceRuntimeState());
    },
  });
  const showExecutionStatus = (message: string, fields: Record<string, unknown> = {}): void => {
    void vscode.window.setStatusBarMessage(message, 3000);
    logger.info("execution", message, fields);
  };
  const showStatusReport = (title: string, report: string, notification: string): void => {
    outputChannel.appendLine(`## ${title}`);
    outputChannel.appendLine(report);
    outputChannel.appendLine("");
    outputChannel.show(true);
    void vscode.window.showInformationMessage(notification);
  };
  const buildEnvironmentPresentationInput = (languageServerStarting = false): EnvironmentPresentationInput => {
    const settings = readSettings(vscode.workspace.workspaceFolders?.[0]);
    const workspaceData = buildWorkspaceInitializationData(
      workspaceFolderPaths(),
      effectiveInitializationSourceRoots(settings),
      {
        interpreterPath: settings.interpreterPath,
        interpreterArgs: settings.interpreterArgs,
        runtimeProbe: false,
      },
    );
    return {
      interpreterPath: settings.interpreterPath,
      languageServerPath: settings.languageServerRustPath,
      languageServerEngine: "rust-v2",
      analysisMode: settings.analysisMode,
      docsSource: settings.docsSource,
      sourceRoots: workspaceData.sourceRoots,
      extraPaths: settings.extraPaths,
      indexMode: workspaceData.sourceRoots.some((root) => root.endsWith("/sage/src") || root.endsWith("\\sage\\src"))
        ? "deferred Sage roots with eager workspace roots"
        : "eager workspace roots",
      indexStatus: lastIndexStatus,
      docsStatus: lastDocsStatus,
      runtimeIntrospectionEnabled: settings.runtimeIntrospectionEnabled,
      enablePyxParsing: settings.enablePyxParsing,
      pythonFilesEnabled: settings.pythonFilesEnabled,
      workspaceRuntimeState: currentWorkspaceRuntimeState(),
      languageServerStarting,
    };
  };

  context.subscriptions.push(
    outputChannel,
    languageOutputChannel,
    statusBarItem,
    cellCodeLensProvider,
    vscode.languages.registerCodeLensProvider(
      [
        { scheme: "file", language: "sagemath" },
        { scheme: "file", language: "python" },
      ],
      cellCodeLensProvider,
    ),
  );

  const activeEditorSettings = (): ReturnType<typeof readSettings> => {
    const activeDocument = vscode.window.activeTextEditor?.document;
    return readSettings(activeDocument ? vscode.workspace.getWorkspaceFolder(activeDocument.uri) : vscode.workspace.workspaceFolders?.[0]);
  };

  const activeOrVisibleSageEditor = (): vscode.TextEditor | undefined => {
    const activeEditor = vscode.window.activeTextEditor;
    if (isSageDocument(activeEditor?.document)) {
      return activeEditor;
    }
    return vscode.window.visibleTextEditors.find((editor) => isSageDocument(editor.document));
  };

  const activationPolicyInput = (): Parameters<typeof shouldAutoStartLanguageClient>[0] => {
    const settings = activeEditorSettings();
    return {
      activeLanguageId: vscode.window.activeTextEditor?.document.languageId,
      pythonFilesEnabled: settings.pythonFilesEnabled,
      sourceRoots: settings.sourceRoots,
      extraPaths: settings.extraPaths,
    };
  };

  const setWorkspaceContexts = async (): Promise<void> => {
    const workspaceRuntimeState = currentWorkspaceRuntimeState();
    await vscode.commands.executeCommand(
      "setContext",
      "sage.workspaceRuntimeAvailable",
      isWorkspaceRuntimeAvailable(workspaceRuntimeState),
    );
    await vscode.commands.executeCommand("setContext", "sage.hasSageEditor", Boolean(activeOrVisibleSageEditor()));
  };

  const ensureWorkspaceRuntimeAvailable = async (action: string): Promise<boolean> => {
    const workspaceRuntimeState = currentWorkspaceRuntimeState();
    if (isWorkspaceRuntimeAvailable(workspaceRuntimeState)) {
      return true;
    }

    const message = formatWorkspaceRuntimeUnavailableMessage(workspaceRuntimeState, action);
    if (!workspaceRuntimeState.trusted) {
      const selected = await vscode.window.showWarningMessage(message, "Manage Workspace Trust");
      if (selected === "Manage Workspace Trust") {
        await vscode.commands.executeCommand("workbench.trust.manage");
      }
    } else {
      void vscode.window.showWarningMessage(message);
    }
    return false;
  };

  const editorForCellExecution = async (target?: RunCurrentCellTarget): Promise<vscode.TextEditor | undefined> => {
    if (!target?.uri) {
      return vscode.window.activeTextEditor;
    }

    const targetUri = target.uri.toString();
    const visibleEditor = vscode.window.visibleTextEditors.find((editor) => editor.document.uri.toString() === targetUri);
    if (visibleEditor) {
      return visibleEditor;
    }

    const document = await vscode.workspace.openTextDocument(target.uri);
    return vscode.window.showTextDocument(document, { preview: false, preserveFocus: true });
  };

  const applyWorkspaceConfigurationProfile = async (
    profile: WorkspaceConfigurationProfile,
    reason: string,
    showCompletionMessage: boolean,
  ): Promise<TestConfigureWorkspaceProfileResult | undefined> => {
    const workspaceFolder = vscode.workspace.workspaceFolders?.[0];
    if (!workspaceFolder) {
      void vscode.window.showWarningMessage("Open a workspace folder before configuring Sage.");
      return undefined;
    }

    const settings = readSettings(workspaceFolder);
    const discoveredSourceRoots = discoverSourceRoots(
      workspaceFolderPaths(),
      settings.sourceRoots,
      {
        interpreterPath: settings.interpreterPath,
        interpreterArgs: settings.interpreterArgs,
      },
    );
    const updates = buildWorkspaceConfigurationUpdates({
      workspaceFolders: workspaceFolderPaths(),
      discoveredSourceRoots,
      configuredExtraPaths: settings.extraPaths,
      configuredRuffConfiguration: vscode.workspace
        .getConfiguration("ruff", workspaceFolder.uri)
        .get("configuration"),
      profile,
    });
    const applied: TestConfigureWorkspaceProfileResult["updates"] = [];
    configurationProfileUpdateDepth += 1;
    try {
      for (const update of updates) {
        const namespace = update.namespace ?? "sage";
        const configuration = vscode.workspace.getConfiguration(namespace, workspaceFolder.uri);
        try {
          await configuration.update(update.section, update.value, vscode.ConfigurationTarget.Workspace);
        } catch (error) {
          if (!isUnregisteredConfigurationError(error)) {
            throw error;
          }
          await updateWorkspaceSettingsJson(
            workspaceFolder.uri,
            `${namespace}.${update.section}`,
            update.value,
          );
        }
        applied.push({ setting: `${namespace}.${update.section}`, value: update.value });
        logger.info("configuration", "updated setting", {
          setting: `${namespace}.${update.section}`,
          value: formatLoggedConfigurationValue(update.value),
        });
      }
    } finally {
      configurationProfileUpdateDepth = Math.max(0, configurationProfileUpdateDepth - 1);
    }
    await setWorkspaceContexts();
    updateStatusBar();
    const suppressedRestarts = suppressedConfigurationRestartCount;
    suppressedConfigurationRestartCount = 0;
    if (suppressedRestarts > 0) {
      logger.info("configuration", "coalesced profile language-server restarts", {
        suppressedRestarts,
      });
    }
    if (showCompletionMessage) {
      startLanguageClientInBackground(reason, true);
    } else {
      await startLanguageClient();
    }

    if (showCompletionMessage) {
      void vscode.window
        .showInformationMessage(
          `Configured Sage workspace profile: ${profile.label}`,
          "Show Sage Status",
          "Rebuild Index",
        )
        .then((selection) => {
          if (selection === "Show Sage Status") {
            void vscode.commands.executeCommand("sage.showEnvironmentDetails");
          }
          if (selection === "Rebuild Index") {
            void vscode.commands.executeCommand("sage.rebuildIndex");
          }
        });
    }

    return { profileId: profile.id, updates: applied };
  };

  const scheduleSlowLanguageServerNotice = (reason: string): void => {
    if (slowLanguageServerNoticeShown || slowLanguageServerNoticeTimer) {
      return;
    }
    slowLanguageServerNoticeTimer = setTimeout(() => {
      slowLanguageServerNoticeTimer = undefined;
      if (client || !languageClientOperation) {
        return;
      }
      slowLanguageServerNoticeShown = true;
      logger.info("extension", "showing slow language-server startup notice", { reason });
      void vscode.window
        .showInformationMessage(
          "Sage language features are starting in the background. You can keep editing; hover, completion, navigation, and indexing will appear when ready.",
          "Show Sage Status",
        )
        .then((selection) => {
          if (selection === "Show Sage Status") {
            void vscode.commands.executeCommand("sage.showEnvironmentDetails");
          }
        });
    }, SLOW_LANGUAGE_SERVER_NOTICE_MS);
  };

  const updateStatusBar = (): void => {
    const policyInput = activationPolicyInput();
    if (!shouldExposeSageExperience(policyInput)) {
      statusBarItem.hide();
      return;
    }
    const settings = activeEditorSettings();
    logger.setLevel(settings.loggingLevel);
    const presentationInput = buildEnvironmentPresentationInput(Boolean(languageClientOperation && !client));
    statusBarItem.text = formatStatusBarText(presentationInput);
    statusBarItem.tooltip = formatStatusBarTooltip(presentationInput);
    statusBarItem.command = "sage.showEnvironmentDetails";
    statusBarItem.show();
  };

  const runIndexCacheMaintenance = async (reason: string): Promise<void> => {
    const report = await maintainIndexCache({
      cacheDir: rustIndexCacheDir(context),
      maxAgeDays: DEFAULT_INDEX_CACHE_MAX_AGE_DAYS,
      maxTotalBytes: DEFAULT_INDEX_CACHE_MAX_TOTAL_BYTES,
      keepLatestDatabases: DEFAULT_INDEX_CACHE_KEEP_LATEST_DATABASES,
      orphanMaxAgeDays: DEFAULT_INDEX_CACHE_ORPHAN_MAX_AGE_DAYS,
      sizePruneMinAgeDays: DEFAULT_INDEX_CACHE_SIZE_PRUNE_MIN_AGE_DAYS,
    });
    const fields = {
      reason,
      cacheDir: report.cacheDir,
      databases: report.totals.databaseCount,
      totalBytes: report.totals.totalBytes,
      deletedFiles: report.totals.deletedFileCount,
      deletedBytes: report.totals.deletedBytes,
      failures: report.failures.length,
    };
    if (report.failures.length > 0) {
      logger.warn("index", "cache maintenance completed with errors", {
        ...fields,
        firstError: report.failures[0],
      });
      return;
    }
    if (report.totals.deletedFileCount > 0) {
      logger.info("index", "cache maintenance pruned stale index files", fields);
      return;
    }
    logger.debug("index", "cache maintenance completed without pruning", fields);
  };

  const startLanguageClient = async (): Promise<void> => {
    languageClientRestartQueued = true;
    if (languageClientOperation) {
      await languageClientOperation;
      return;
    }

    languageClientOperation = (async () => {
      while (languageClientRestartQueued) {
        languageClientRestartQueued = false;
        clearLanguageServerStatusRefresh();

        if (client) {
          const previousClient = client;
          client = undefined;
          languageClientManagedShutdown = true;
          languageClientManagedCloseCount += 1;
          try {
            await previousClient.stop();
          } finally {
            languageClientManagedShutdown = false;
          }
        }

        const workspaceRuntimeState = currentWorkspaceRuntimeState();
        if (!isWorkspaceRuntimeAvailable(workspaceRuntimeState)) {
          lastIndexStatus = undefined;
          lastDocsStatus = undefined;
          updateStatusBar();
          logger.info("extension", "language client disabled by workspace runtime state", {
            trusted: workspaceRuntimeState.trusted,
            hasVirtualWorkspace: workspaceRuntimeState.hasVirtualWorkspace,
          });
          continue;
        }

        try {
          await runIndexCacheMaintenance("language-client-start");
          const nextClient = createLanguageClient(context, languageOutputChannel, {
            shouldAutoRestartOnClose: () => !languageClientManagedShutdown,
            runtimeDiscoveredSourceRoots,
            onClose: ({ managedShutdown }) => {
              if (!managedShutdown) {
                languageClientUnexpectedCloseCount += 1;
              }
            },
          });
          languageClientLaunchCount += 1;
          client = nextClient;
          await nextClient.start();
          await refreshLanguageServerStatus();
          scheduleLanguageServerStatusRefresh();
          logger.info("extension", "language client started", { launchCount: languageClientLaunchCount });
        } catch (error) {
          client = undefined;
          const message = `Sage language server failed to start: ${String(error)}`;
          logger.error("extension", "language server failed to start", { error: String(error) });
          void vscode.window.showErrorMessage(
            `${message}. Check 'sage.languageServer.rustPath' and the Sage output channels.`,
          );
        }
      }
    })().finally(() => {
      languageClientOperation = undefined;
      clearSlowLanguageServerNotice();
      updateStatusBar();
    });

    updateStatusBar();
    await languageClientOperation;
  };

  const startLanguageClientInBackground = (reason: string, force = false): void => {
    if (!force && !shouldAutoStartLanguageClient(activationPolicyInput())) {
      logger.info("extension", "language client auto-start skipped outside Sage context", { reason });
      updateStatusBar();
      return;
    }
    logger.info("extension", "starting language client in background", { reason });
    scheduleSlowLanguageServerNotice(reason);
    void startLanguageClient().catch((error) => {
      logger.error("extension", "background language client start failed", {
        reason,
        error: String(error),
      });
    });
  };

  const scheduleRuntimeSourceRootDiscovery = (reason: string): void => {
    if (runtimeSourceRootDiscoveryOperation || !isWorkspaceRuntimeAvailable(currentWorkspaceRuntimeState())) {
      return;
    }
    if (!shouldAutoStartLanguageClient(activationPolicyInput())) {
      logger.debug("workspace", "runtime source-root discovery skipped outside Sage context", { reason });
      return;
    }
    const settings = readSettings(vscode.workspace.workspaceFolders?.[0]);
    if (!settings.runtimeIntrospectionEnabled || !settings.interpreterPath) {
      return;
    }

    const workspaceFolders = workspaceFolderPaths();
    const discoveryGeneration = runtimeSourceRootDiscoveryGeneration;
    const startupRoots = discoverSourceRoots(
      workspaceFolders,
      effectiveInitializationSourceRoots(settings),
      {
        interpreterPath: settings.interpreterPath,
        interpreterArgs: settings.interpreterArgs,
        runtimeProbe: false,
      },
    ).map((root) => path.resolve(root));

    runtimeSourceRootDiscoveryOperation = (async () => {
      const started = Date.now();
      try {
        const discoveredRoots = await discoverSourceRootsAsync(
          workspaceFolders,
          effectiveInitializationSourceRoots(settings),
          {
            interpreterPath: settings.interpreterPath,
            interpreterArgs: settings.interpreterArgs,
          },
        );
        if (discoveryGeneration !== runtimeSourceRootDiscoveryGeneration) {
          return;
        }
        const knownRoots = new Set(
          [...startupRoots, ...runtimeDiscoveredSourceRoots].map((root) => path.resolve(root)),
        );
        const additions = discoveredRoots
          .map((root) => path.resolve(root))
          .filter((root) => !knownRoots.has(root));
        if (additions.length === 0) {
          logger.debug("workspace", "runtime source-root discovery found no new roots", {
            reason,
            elapsedMs: Date.now() - started,
          });
          return;
        }

        runtimeDiscoveredSourceRoots = dedupeStrings([...runtimeDiscoveredSourceRoots, ...additions]);
        logger.info("workspace", "runtime source-root discovery added roots", {
          reason,
          elapsedMs: Date.now() - started,
          count: additions.length,
          roots: additions.join(","),
        });
        updateStatusBar();
        startLanguageClientInBackground("runtime-source-root-discovery", true);
      } catch (error) {
        logger.warn("workspace", "runtime source-root discovery failed", {
          reason,
          error: String(error),
        });
      } finally {
        if (discoveryGeneration === runtimeSourceRootDiscoveryGeneration) {
          runtimeSourceRootDiscoveryOperation = undefined;
        }
      }
    })();
  };

  const ensureLanguageClientReady = async (action: string): Promise<LanguageClient | undefined> => {
    if (!(await ensureWorkspaceRuntimeAvailable(action))) {
      return undefined;
    }
    if (!client) {
      if (!languageClientOperation) {
        startLanguageClientInBackground(action, true);
      }
      if (languageClientOperation) {
        await vscode.window.withProgress(
          {
            location: vscode.ProgressLocation.Notification,
            title: "Starting Sage language server",
            cancellable: false,
          },
          async () => {
            await languageClientOperation;
          },
        );
      }
    }
    if (!client) {
      void vscode.window.showWarningMessage("Sage language server is not available yet.");
      return undefined;
    }
    return client;
  };

  context.subscriptions.push(
    vscode.languages.registerReferenceProvider(
      [{ scheme: "file", language: "python" }],
      {
        async provideReferences(document, position, referenceContext, token) {
          if (!isExternalSageSourceDocument(document)) {
            return [];
          }
          const activeClient = await ensureLanguageClientReady("Finding Sage references");
          if (!activeClient || token.isCancellationRequested) {
            return [];
          }
          const payload = {
            textDocument: { uri: document.uri.toString() },
            position: {
              line: position.line,
              character: position.character,
            },
            context: { includeDeclaration: referenceContext.includeDeclaration },
          };
          try {
            const references = await activeClient.sendRequest<LspLocationPayload[]>(
              "textDocument/references",
              payload,
              token,
            );
            const locations = (references ?? [])
              .filter(isLspLocationPayload)
              .map(locationFromLspPayload)
              .filter((location): location is vscode.Location => Boolean(location));
            logger.info("navigation", "external Sage source references resolved", {
              uri: document.uri.toString(),
              line: position.line,
              character: position.character,
              includeDeclaration: referenceContext.includeDeclaration,
              count: locations.length,
            });
            return locations;
          } catch (error) {
            logger.warn("navigation", "external Sage source references failed", {
              uri: document.uri.toString(),
              line: position.line,
              character: position.character,
              error: String(error),
            });
            return [];
          }
        },
      },
    ),
  );

  const refreshLanguageServerStatus = async (): Promise<void> => {
    if (!client) {
      lastIndexStatus = undefined;
      lastDocsStatus = undefined;
      updateStatusBar();
      return;
    }
    try {
      lastIndexStatus = await executeSageCommand<IndexStatusSummary>(client, RUST_LSP_COMMANDS.indexStatus) ?? undefined;
      lastDocsStatus = await executeSageCommand<DocsStatusSummary>(client, RUST_LSP_COMMANDS.docsStatus) ?? undefined;
      maybePromptForIndexMaintenance(lastIndexStatus);
    } catch (error) {
      logger.warn("extension", "failed to refresh language server status", { error: String(error) });
    } finally {
      updateStatusBar();
    }
  };

  const maybePromptForIndexMaintenance = (status: IndexStatusSummary | undefined): void => {
    const notice = buildIndexMaintenanceNotice(status);
    if (!notice || shownIndexMaintenanceNotices.has(notice.key)) {
      return;
    }
    shownIndexMaintenanceNotices.add(notice.key);
    logger.info("index", "showing index maintenance notice", {
      key: notice.key,
      files: status?.indexed_file_count,
      cacheHits: status?.cache_hit_count,
      cacheMisses: status?.cache_miss_count,
    });
    void vscode.window
      .showInformationMessage(notice.message, "Rebuild Index", "Later")
      .then((selection) => {
        if (selection === "Rebuild Index") {
          void vscode.commands.executeCommand("sage.rebuildIndex");
        }
      });
  };

  const scheduleLanguageServerStatusRefresh = (): void => {
    clearLanguageServerStatusRefresh();
    languageServerStatusRefreshTimer = setInterval(() => {
      if (languageServerStatusRefreshInFlight) {
        return;
      }
      languageServerStatusRefreshInFlight = true;
      void refreshLanguageServerStatus().finally(() => {
        languageServerStatusRefreshInFlight = false;
        languageServerStatusRefreshAttempts += 1;
        const pendingJobs = lastIndexStatus?.pending_jobs ?? 0;
        if (pendingJobs === 0) {
          clearLanguageServerStatusRefresh();
          return;
        }
        if (languageServerStatusRefreshAttempts === LANGUAGE_SERVER_STATUS_REFRESH_MAX_ATTEMPTS) {
          logger.info("extension", "language server status still pending; continuing refresh", {
            pendingJobs,
            pendingTask: lastIndexStatus?.pending_task,
            attempts: languageServerStatusRefreshAttempts,
          });
        }
      });
    }, LANGUAGE_SERVER_STATUS_REFRESH_INTERVAL_MS);
  };

  context.subscriptions.push(
    vscode.workspace.registerTextDocumentContentProvider(
      SAGE_SOURCE_SCHEME,
      new SageSourceTextDocumentProvider(),
    ),
    vscode.window.onDidCloseTerminal((terminal) => {
      terminalManager.handleClosedTerminal(terminal);
    }),
    vscode.window.onDidChangeActiveTextEditor(async () => {
      await setWorkspaceContexts();
      cellCodeLensProvider.refresh();
      updateStatusBar();
      if (!client && !languageClientOperation && shouldAutoStartLanguageClient(activationPolicyInput())) {
        startLanguageClientInBackground("active-editor-change");
      }
    }),
    vscode.workspace.onDidGrantWorkspaceTrust(async () => {
      await setWorkspaceContexts();
      cellCodeLensProvider.refresh();
      startLanguageClientInBackground("workspace-trust-granted");
    }),
    vscode.commands.registerCommand("sage.openGettingStarted", async () => {
      await vscode.commands.executeCommand(
        "workbench.action.openWalkthrough",
        `${context.extension.id}#${GETTING_STARTED_WALKTHROUGH_ID}`,
        false,
      );
    }),
    vscode.commands.registerCommand("sage.selectInterpreter", async () => {
      if (!(await ensureWorkspaceRuntimeAvailable("Selecting a Sage interpreter"))) {
        return;
      }
      const settings = readSettings(vscode.workspace.workspaceFolders?.[0]);
      const detectedOptions = discoverInterpreterCandidates({
        currentPath: settings.interpreterPath,
        languageServerPythonPath: settings.languageServerPythonPath,
        workspaceFolders: workspaceFolderPaths(),
      });
      const picked = await vscode.window.showQuickPick(detectedOptions, {
        title: "Select Sage environment",
        placeHolder: "Choose a complete Sage environment, or fall back to the advanced path actions below.",
        matchOnDescription: true,
        matchOnDetail: true,
      });

      if (!picked) {
        return;
      }

      const updates = await resolveInterpreterConfigurationUpdate(picked, settings);
      if (!updates || updates.length === 0) {
        return;
      }

      let shouldResetRepl = false;
      for (const update of updates) {
        await vscode.workspace
          .getConfiguration("sage")
          .update(update.section, update.value, vscode.ConfigurationTarget.Workspace);
        logger.info("configuration", "updated setting", {
          setting: `sage.${update.section}`,
          value: update.value,
        });
        shouldResetRepl ||= update.section === "interpreter.path";
      }
      if (shouldResetRepl) {
        terminalManager.resetReplTerminal();
      }
    }),
    vscode.commands.registerCommand("sage.restartLanguageServer", async () => {
      if (!(await ensureWorkspaceRuntimeAvailable("Restarting the Sage language server"))) {
        return;
      }
      logger.info("extension", "restarting language server");
      await startLanguageClient();
    }),
    vscode.commands.registerCommand("sage.configureWorkspace", async () => {
      if (!(await ensureWorkspaceRuntimeAvailable("Configuring the Sage workspace"))) {
        return;
      }
      const workspaceFolder = vscode.workspace.workspaceFolders?.[0];
      if (!workspaceFolder) {
        void vscode.window.showWarningMessage("Open a workspace folder before configuring Sage.");
        return;
      }
      const recommended = recommendedWorkspaceProfile(vscode.window.activeTextEditor?.document.languageId);
      const picked = await vscode.window.showQuickPick(
        WORKSPACE_CONFIGURATION_PROFILES.map((profile) => ({
          label: profile.id === recommended.id ? `${profile.label} (Recommended)` : profile.label,
          description: profile.description,
          detail: profile.detail,
          profile,
        })),
        {
          title: "Configure Sage workspace",
          placeHolder: "Choose the closest profile for this workspace.",
          matchOnDescription: true,
          matchOnDetail: true,
        },
      );
      if (!picked) {
        return;
      }
      await applyWorkspaceConfigurationProfile(picked.profile as WorkspaceConfigurationProfile, "configure-workspace", true);
    }),
    vscode.commands.registerCommand("sage.__test.getLifecycleSnapshot", () => languageClientLifecycleSnapshot()),
    vscode.commands.registerCommand("sage.__test.getCurrentSageContext", () => {
      const activeDocument = vscode.window.activeTextEditor?.document;
      const settings = activeEditorSettings();
      const policyInput = activationPolicyInput();
      return {
        languageId: activeDocument?.languageId,
        pythonFilesEnabled: settings.pythonFilesEnabled,
        sourceRootCount: settings.sourceRoots.length,
        extraPathCount: settings.extraPaths.length,
        isSageEditor: isSageDocument(activeDocument),
        shouldAutoStartLanguageClient: shouldAutoStartLanguageClient(policyInput),
        shouldExposeSageExperience: shouldExposeSageExperience(policyInput),
      };
    }),
    vscode.commands.registerCommand("sage.__test.awaitLanguageClientStable", async () => {
      if (languageClientOperation) {
        await languageClientOperation;
      }
      return languageClientLifecycleSnapshot();
    }),
    vscode.commands.registerCommand("sage.__test.restartLanguageServerAndWait", async () => {
      await startLanguageClient();
      return languageClientLifecycleSnapshot();
    }),
    vscode.commands.registerCommand("sage.__test.configureWorkspaceProfile", async (profileId: WorkspaceConfigurationProfileId = "research") => {
      const profile = WORKSPACE_CONFIGURATION_PROFILES.find((entry) => entry.id === profileId);
      if (!profile) {
        throw new Error(`Unknown Sage workspace profile: ${profileId}`);
      }
      return applyWorkspaceConfigurationProfile(profile, "test-configure-workspace", false);
    }),
    vscode.commands.registerCommand("sage.runCurrentFile", async () => {
      if (!(await ensureWorkspaceRuntimeAvailable("Running a Sage file"))) {
        return;
      }
      const editor = vscode.window.activeTextEditor;
      if (!editor) {
        void vscode.window.showWarningMessage("Open a Sage file before running it.");
        return;
      }
      if (editor.document.uri.scheme !== "file") {
        void vscode.window.showWarningMessage("Sage can only run local files from disk.");
        return;
      }
      const settings = readSettings(vscode.workspace.getWorkspaceFolder(editor.document.uri));
      const runtimePythonPaths = resolveRuntimePythonPaths(
        workspaceFolderPaths(),
        settings.sourceRoots,
        settings.extraPaths,
      );
      const terminal = terminalManager.runFile({ ...settings, runtimePythonPaths }, editor.document.uri.fsPath);
      terminal.show(true);
      showExecutionStatus(
        settings.runTarget === "repl"
          ? "Sage: sent current file to REPL"
          : "Sage: running current file",
        {
          target: settings.runTarget,
          path: editor.document.uri.fsPath,
          runtimePythonPaths: runtimePythonPaths.length,
        },
      );
    }),
    vscode.commands.registerCommand("sage.runSelection", async () => {
      if (!(await ensureWorkspaceRuntimeAvailable("Running Sage code in the REPL"))) {
        return;
      }
      const editor = vscode.window.activeTextEditor;
      if (!editor) {
        void vscode.window.showWarningMessage("Open a Sage file before sending code to REPL.");
        return;
      }
      const selection = editor.document.getText(editor.selection) || editor.document.lineAt(editor.selection.active.line).text;
      const settings = readSettings(vscode.workspace.getWorkspaceFolder(editor.document.uri));
      const runtimePythonPaths = resolveRuntimePythonPaths(
        workspaceFolderPaths(),
        settings.sourceRoots,
        settings.extraPaths,
        editor.document.uri.fsPath,
      );
      const terminal = terminalManager.runSelection({ ...settings, runtimePythonPaths }, selection);
      terminal.show(true);
      showExecutionStatus("Sage: sent selection to REPL", {
        selectionLength: selection.length,
        runtimePythonPaths: runtimePythonPaths.length,
      });
    }),
    vscode.commands.registerCommand("sage.runCurrentCell", async (target?: RunCurrentCellTarget) => {
      if (!(await ensureWorkspaceRuntimeAvailable("Running the current Sage cell in the REPL"))) {
        return;
      }
      const editor = await editorForCellExecution(target);
      if (!editor) {
        void vscode.window.showWarningMessage("Open a Sage file before sending a cell to REPL.");
        return;
      }
      const activeLine = typeof target?.line === "number" ? target.line : editor.selection.active.line;
      const cell = currentSageCell(editor.document.getText(), activeLine);
      if (!cell) {
        void vscode.window.showWarningMessage("No Sage cell content found near the cursor.");
        return;
      }
      const settings = readSettings(vscode.workspace.getWorkspaceFolder(editor.document.uri));
      const runtimePythonPaths = resolveRuntimePythonPaths(
        workspaceFolderPaths(),
        settings.sourceRoots,
        settings.extraPaths,
        editor.document.uri.scheme === "file" ? editor.document.uri.fsPath : undefined,
      );
      const terminal = terminalManager.runSelection({ ...settings, runtimePythonPaths }, cell.text);
      terminal.show(true);
      showExecutionStatus("Sage: sent current cell to REPL", {
        startLine: cell.startLine,
        endLine: cell.endLine,
        selectionLength: cell.text.length,
        runtimePythonPaths: runtimePythonPaths.length,
      });
    }),
    vscode.commands.registerCommand("sage.startRepl", async () => {
      if (!(await ensureWorkspaceRuntimeAvailable("Starting the Sage REPL"))) {
        return;
      }
      const settings = readSettings(vscode.workspace.workspaceFolders?.[0]);
      const terminal = terminalManager.startRepl(settings);
      terminal.show(true);
      showExecutionStatus("Sage: REPL ready or starting", {
        interpreterPath: settings.interpreterPath,
      });
    }),
    vscode.commands.registerCommand("sage.showDocumentation", async () => {
      const activeClient = await ensureLanguageClientReady("Showing Sage documentation");
      if (!activeClient) {
        return;
      }

      const editor = activeOrVisibleSageEditor();
      if (!editor) {
        void vscode.window.showWarningMessage("Open a Sage file to request documentation.");
        return;
      }

      const selectedText = editor.document.getText(editor.selection).trim() || undefined;
      const result = await requestDocumentation(
        activeClient,
        editor.document.uri.toString(),
        editor.selection.active.line,
        editor.selection.active.character,
        selectedText,
      );

      if (!result) {
        void vscode.window.showInformationMessage("No documentation available for the current symbol.");
        return;
      }

      docsPanel.show(`Docs: ${result.symbol}`, renderDocumentationMarkdown(result));
    }),
    vscode.commands.registerCommand("sage.findReferences", async () => {
      const activeClient = await ensureLanguageClientReady("Finding Sage references");
      if (!activeClient) {
        return;
      }

      const editor = activeOrVisibleSageEditor();
      if (!editor) {
        logger.warn("navigation", "Sage references skipped because no Sage editor is visible", {
          activeLanguageId: vscode.window.activeTextEditor?.document.languageId ?? "none",
          activeUri: vscode.window.activeTextEditor?.document.uri.toString() ?? "none",
        });
        void vscode.window.showWarningMessage("Open a Sage file before finding references.");
        return;
      }

      const payload = {
        textDocument: { uri: editor.document.uri.toString() },
        position: {
          line: editor.selection.active.line,
          character: editor.selection.active.character,
        },
        context: { includeDeclaration: true },
      };
      let references: LspLocationPayload[];
      try {
        references = await activeClient.sendRequest<LspLocationPayload[]>("textDocument/references", payload);
      } catch (error) {
        logger.error("navigation", "Sage references request failed", {
          uri: editor.document.uri.toString(),
          line: editor.selection.active.line,
          character: editor.selection.active.character,
          error: String(error),
        });
        void vscode.window.showErrorMessage(`Sage references failed: ${String(error)}`);
        return;
      }
      const locations = (references ?? [])
        .filter(isLspLocationPayload)
        .map(locationFromLspPayload)
        .filter((location): location is vscode.Location => Boolean(location));

      logger.info("navigation", "Sage references resolved", {
        uri: editor.document.uri.toString(),
        languageId: editor.document.languageId,
        workspaceFolder: vscode.workspace.getWorkspaceFolder(editor.document.uri)?.uri.toString() ?? "external",
        line: editor.selection.active.line,
        character: editor.selection.active.character,
        count: locations.length,
      });
      if (locations.length === 0) {
        void vscode.window.showInformationMessage("No Sage references found for the current symbol.");
        return;
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
        logger.warn("navigation", "Peek references failed; falling back to quick pick", {
          uri: editor.document.uri.toString(),
          count: locations.length,
          error: String(error),
        });
        await showReferencesQuickPick(locations);
      }
    }),
    vscode.commands.registerCommand("sage.runUxSelfCheck", async () => {
      const activeClient = await ensureLanguageClientReady("Running the Sage UX self check");
      if (!activeClient) {
        return;
      }

      const editor = activeOrVisibleSageEditor();
      if (!editor) {
        void vscode.window.showWarningMessage("Open a Sage file before running the Sage UX self check.");
        return;
      }

      const selectedText = editor.document.getText(editor.selection).trim() || undefined;
      const selfCheckStarted = Date.now();
      const queryStarted = Date.now();
      let query = await executeSageCommand<QueryResponse>(
        activeClient,
        RUST_LSP_COMMANDS.queryAtPosition,
        [
          buildQueryRequestPayload(
            editor.document.uri.toString(),
            editor.selection.active.line,
            editor.selection.active.character,
            selectedText,
            { mode: "navigation" },
          ),
        ],
      );
      const queryMs = Date.now() - queryStarted;
      let fullQueryMs: number | undefined;
      if (shouldRunFullUxSelfCheckQuery(query)) {
        const fullQueryStarted = Date.now();
        query = await executeSageCommand<QueryResponse>(
          activeClient,
          RUST_LSP_COMMANDS.queryAtPosition,
          [
            buildQueryRequestPayload(
              editor.document.uri.toString(),
              editor.selection.active.line,
              editor.selection.active.character,
              selectedText,
            ),
          ],
        );
        fullQueryMs = Date.now() - fullQueryStarted;
      }
      const indexStatusStarted = Date.now();
      lastIndexStatus = await executeSageCommand<IndexStatusSummary>(activeClient, RUST_LSP_COMMANDS.indexStatus) ?? undefined;
      const indexStatusMs = Date.now() - indexStatusStarted;
      const docsStatusStarted = Date.now();
      lastDocsStatus = await executeSageCommand<DocsStatusSummary>(activeClient, RUST_LSP_COMMANDS.docsStatus) ?? undefined;
      const docsStatusMs = Date.now() - docsStatusStarted;
      updateStatusBar();
      const result = formatUxSelfCheckReport({
        documentUri: editor.document.uri.toString(),
        symbol: selectedText,
        query,
        indexStatus: lastIndexStatus,
        docsStatus: lastDocsStatus,
        timings: {
          queryMs,
          fullQueryMs,
          indexStatusMs,
          docsStatusMs,
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
      outputChannel.clear();
      outputChannel.appendLine(result.report);
      outputChannel.show(true);
      void vscode.window.showInformationMessage(`Sage UX Self Check: ${result.passed}/${result.total} checks passing`);
    }),
    vscode.commands.registerCommand("sage.showEnvironmentDetails", async () => {
      showStatusReport(
        "Sage Environment Details",
        formatEnvironmentDetails(buildEnvironmentPresentationInput()),
        "Sage environment details written to the Sage output channel.",
      );
    }),
    vscode.commands.registerCommand("sage.showIndexStatus", async () => {
      const activeClient = await ensureLanguageClientReady("Showing the Sage index status");
      if (!activeClient) {
        return;
      }
      const status = await executeSageCommand<IndexStatusSummary>(activeClient, RUST_LSP_COMMANDS.indexStatus);
      lastIndexStatus = status ?? undefined;
      updateStatusBar();
      showStatusReport(
        "Sage Index Status",
        formatIndexStatusReport(status),
        "Sage index status written to the Sage output channel.",
      );
    }),
    vscode.commands.registerCommand("sage.showDocsStatus", async () => {
      const activeClient = await ensureLanguageClientReady("Showing the Sage docs status");
      if (!activeClient) {
        return;
      }
      const status = await executeSageCommand<DocsStatusSummary>(activeClient, RUST_LSP_COMMANDS.docsStatus);
      lastDocsStatus = status ?? undefined;
      updateStatusBar();
      showStatusReport(
        "Sage Documentation Status",
        formatDocsStatusReport(status),
        "Sage documentation status written to the Sage output channel.",
      );
    }),
    vscode.commands.registerCommand("sage.copySupportBundle", async () => {
      if (client) {
        await refreshLanguageServerStatus();
      }
      const activeDocument = vscode.window.activeTextEditor?.document;
      const settings = activeEditorSettings();
      const workspaceRuntimeState = currentWorkspaceRuntimeState();
      const bundle = buildSupportBundle({
        generatedAt: new Date().toISOString(),
        extension: {
          id: context.extension.id,
          version: String(context.extension.packageJSON?.version ?? "unknown"),
        },
        host: {
          vscodeVersion: vscode.version,
          platform: process.platform,
          arch: process.arch,
          nodeVersion: process.version,
        },
        workspace: {
          folders: workspaceFolderPaths(),
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
        settings,
        environment: buildEnvironmentPresentationInput(Boolean(languageClientOperation && !client)),
        lifecycle: languageClientLifecycleSnapshot(),
        indexStatus: lastIndexStatus,
        docsStatus: lastDocsStatus,
      });
      await vscode.env.clipboard.writeText(bundle);
      outputChannel.clear();
      outputChannel.appendLine(bundle);
      outputChannel.show(true);
      void vscode.window.showInformationMessage(
        "Sage support bundle copied. It includes paths and settings, but no source contents, selected text, or environment variables.",
      );
    }),
    vscode.commands.registerCommand("sage.rebuildIndex", async () => {
      const activeClient = await ensureLanguageClientReady("Rebuilding the Sage index");
      if (!activeClient) {
        return;
      }
      const status = await executeSageCommand<IndexStatusSummary>(activeClient, RUST_LSP_COMMANDS.rebuildIndex);
      lastIndexStatus = status ?? undefined;
      await refreshLanguageServerStatus();
      scheduleLanguageServerStatusRefresh();
      void vscode.window.showInformationMessage(`Index rebuilt: ${formatIndexStatusMessage(status)}`);
    }),
    vscode.workspace.onDidChangeConfiguration(async (event) => {
      if (!event.affectsConfiguration("sage")) {
        return;
      }
      if (event.affectsConfiguration("sage.interpreter.path") || event.affectsConfiguration("sage.interpreter.args")) {
        terminalManager.resetReplTerminal();
      }
      if (
        event.affectsConfiguration("sage.interpreter.path")
        || event.affectsConfiguration("sage.interpreter.args")
        || event.affectsConfiguration("sage.analysis.sourceRoots")
      ) {
        runtimeDiscoveredSourceRoots = [];
        runtimeSourceRootDiscoveryGeneration += 1;
        runtimeSourceRootDiscoveryOperation = undefined;
        scheduleRuntimeSourceRootDiscovery("configuration-change");
      }
      if (
        event.affectsConfiguration("sage.analysis.enablePythonFiles")
        || event.affectsConfiguration("sage.run.showCellCodeLens")
      ) {
        cellCodeLensProvider.refresh();
      }
      updateStatusBar();
      if (shouldRestartLanguageServer((section) => event.affectsConfiguration(section))) {
        if (configurationProfileUpdateDepth > 0) {
          suppressedConfigurationRestartCount += 1;
          logger.debug("configuration", "coalescing language-server restart during profile update", {
            suppressedRestarts: suppressedConfigurationRestartCount,
          });
          return;
        }
        startLanguageClientInBackground("configuration-change");
      }
    }),
    vscode.workspace.onDidChangeWorkspaceFolders(async () => {
      await setWorkspaceContexts();
      cellCodeLensProvider.refresh();
      runtimeDiscoveredSourceRoots = [];
      runtimeSourceRootDiscoveryGeneration += 1;
      runtimeSourceRootDiscoveryOperation = undefined;
      updateStatusBar();
      scheduleRuntimeSourceRootDiscovery("workspace-folders-changed");
      startLanguageClientInBackground("workspace-folders-changed");
    }),
  );

  void setWorkspaceContexts();
  updateStatusBar();
  scheduleRuntimeSourceRootDiscovery("activation");
  startLanguageClientInBackground("activation");
}

export async function deactivate(): Promise<void> {
  clearLanguageServerStatusRefresh();
  clearSlowLanguageServerNotice();
  if (languageClientOperation) {
    await languageClientOperation;
  }
  if (client) {
    languageClientManagedShutdown = true;
    languageClientManagedCloseCount += 1;
    try {
      await client.stop();
      client = undefined;
    } finally {
      languageClientManagedShutdown = false;
    }
  }
}

async function resolveInterpreterConfigurationUpdate(
  picked: InterpreterCandidate,
  settings: ReturnType<typeof readSettings>,
): Promise<Array<{ section: "interpreter.path" | "languageServer.pythonPath"; value: string }> | undefined> {
  if (picked.selectionTarget === "languageServerAuto") {
    return [{ section: "languageServer.pythonPath", value: "auto" }];
  }

  if (picked.selectionTarget === "runtimeCustom") {
    const selection = await vscode.window.showInputBox({
      title: "Sage runtime path",
      value: settings.interpreterPath,
      prompt: "Enter the Sage executable used for run commands and the managed REPL.",
    });
    if (!selection) {
      return undefined;
    }
    return [{ section: "interpreter.path", value: selection }];
  }

  if (picked.selectionTarget === "languageServerCustom") {
    const selection = await vscode.window.showInputBox({
      title: "Language-server Python path",
      value: settings.languageServerPythonPath === "auto" ? "" : settings.languageServerPythonPath,
      prompt: "Enter the Python executable used to run sage_lsp.",
    });
    if (!selection) {
      return undefined;
    }
    return [{ section: "languageServer.pythonPath", value: selection }];
  }

  if (picked.updates && picked.updates.length > 0) {
    return picked.updates;
  }

  if (!picked.interpreterPath) {
    return undefined;
  }

  return [{ section: "interpreter.path", value: picked.interpreterPath }];
}
