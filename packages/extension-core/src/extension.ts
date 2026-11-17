import * as path from "node:path";
import * as fs from "node:fs";

import * as vscode from "vscode";
import type { LanguageClient } from "vscode-languageclient/node";

import {
  isSageDocumentLanguage,
  shouldAutoStartLanguageClient,
  shouldExposeSageExperience,
} from "./activationPolicy";
import {
  waitForOperationOrCancellation,
  withOperationTimeout,
} from "./boundedOperation";
import { readSettings } from "./configuration";
import { SageCellCodeLensProvider } from "./cellCodeLens";
import { DocumentationPanel } from "./docsPanel";
import {
  formatStatusBarText,
  formatStatusBarTooltip,
  buildIndexMaintenanceNotice,
  type DocsStatusSummary,
  type EnvironmentPresentationInput,
  type IndexStatusSummary,
} from "./environmentPresentation";
import { createOutputLogger } from "./extensionLogger";
import { registerExecutionCommands } from "./executionCommands";
import {
  isExternalSageSourceDocument as isExternalSageSourceDocumentInRoots,
  registerExternalSourceNavigationProviders,
} from "./externalSourceNavigation";
import {
  createLanguageClient,
  executeSageCommand,
  RUST_LSP_COMMANDS,
  SAGE_LANGUAGE_FILE_GLOB,
  rustIndexCacheDir,
  languageServerLaunch,
} from "./languageClient";
import {
  startLanguageClientWithTimeout,
  stopLanguageClientWithTimeout,
} from "./languageClientOperations";
import { verifyLanguageServerLaunch, LanguageServerLaunchError } from "./serverStartup";
import { LanguageClientLifecycleController } from "./languageClientLifecycleController";
import { RuntimeSourceRootDiscoveryController } from "./runtimeSourceRootDiscoveryController";
import { STATUS_MENU_COMMAND, statusMenuActions } from "./statusMenu";
import {
  DEFAULT_INDEX_CACHE_KEEP_LATEST_DATABASES,
  DEFAULT_INDEX_CACHE_MAX_AGE_DAYS,
  DEFAULT_INDEX_CACHE_MAX_TOTAL_BYTES,
  DEFAULT_INDEX_CACHE_ORPHAN_MAX_AGE_DAYS,
  DEFAULT_INDEX_CACHE_SIZE_PRUNE_MIN_AGE_DAYS,
  maintainIndexCache,
} from "./indexCacheMaintenance";
import {
  SageSourceTextDocumentProvider,
  SAGE_SOURCE_SCHEME,
} from "./sageSourceView";
import {
  discoverInterpreterCandidates,
  resolveInterpreterConfigurationUpdates,
} from "./interpreterDiscovery";
import { shouldRestartLanguageServer } from "./serverRestart";
import { SageTerminalManager } from "./terminalManager";
import { registerNavigationCommands } from "./navigationCommands";
import { registerUxSelfCheckCommand } from "./uxSelfCheckCommand";
import { registerStatusCommands } from "./statusCommands";
import {
  formatWorkspaceRuntimeUnavailableMessage,
  isWorkspaceRuntimeAvailable,
  type WorkspaceRuntimeState,
} from "./workspaceTrust";
import {
  buildWorkspaceInitializationData,
  discoverSourceRoots,
  discoverSourceRootsAsync,
} from "./workspaceDiscovery";
import {
  buildWorkspaceConfigurationUpdates,
  recommendedWorkspaceProfile,
  WORKSPACE_CONFIGURATION_PROFILES,
  type WorkspaceConfigurationProfile,
  type WorkspaceConfigurationProfileId,
} from "./workspaceConfigurator";
import {
  activeWorkspaceFolder,
  isLooseDocument,
  looseDocumentDirectories,
  formatLoggedConfigurationValue,
  isUnregisteredConfigurationError,
  updateWorkspaceSettingsJson,
  workspaceConfigurationTarget,
  workspaceFolderPaths,
} from "./workspaceScope";
import {
  effectiveSourceRootPaths as resolveEffectiveSourceRootPaths,
  sourceRootContainsDocument,
} from "./sourceRootPaths";
import { LanguageServerStatusRefreshController } from "./statusRefreshController";

let languageClientLifecycleController: LanguageClientLifecycleController<LanguageClient> | undefined;
let controllers = new Map<string, LanguageClientLifecycleController<LanguageClient>>();
let configurationProfileUpdateDepth = 0;
let suppressedConfigurationRestartCount = 0;
let lastIndexStatus: IndexStatusSummary | undefined;
let lastDocsStatus: DocsStatusSummary | undefined;
let languageServerStatusRefreshController: LanguageServerStatusRefreshController | undefined;
let runtimeSourceRootDiscoveryController: RuntimeSourceRootDiscoveryController | undefined;
let extensionDeactivating = false;
const shownIndexMaintenanceNotices = new Set<string>();

const LANGUAGE_SERVER_STATUS_REFRESH_INTERVAL_MS = 1500;
const LANGUAGE_SERVER_STATUS_REFRESH_LOG_EVERY = 12;
const LANGUAGE_SERVER_STATUS_REQUEST_TIMEOUT_MS = 5_000;
const LANGUAGE_SERVER_SHUTDOWN_TIMEOUT_MS = 4_000;
const LANGUAGE_SERVER_START_TIMEOUT_MS = 30_000;
const SLOW_LANGUAGE_SERVER_NOTICE_MS = 8000;
const GETTING_STARTED_WALKTHROUGH_ID = "gettingStarted";

interface TestConfigureWorkspaceProfileResult {
  profileId: WorkspaceConfigurationProfileId;
  updates: Array<{ setting: string; value: unknown }>;
}

function currentWorkspaceRuntimeState(): WorkspaceRuntimeState {
  return {
    trusted: vscode.workspace.isTrusted,
    hasVirtualWorkspace: (activeWorkspaceFolder()?.uri.scheme ?? "file") !== "file",
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
  if (document.uri.scheme === SAGE_SOURCE_SCHEME) {
    return isExternalSageSourceDocument(document);
  }
  if (document.uri.scheme !== "file" || document.languageId !== "python") {
    return false;
  }
  return sourceRootContainsDocument(
    effectiveSourceRootPaths(
      settings,
      workspaceFolder ? [workspaceFolder.uri.fsPath] : workspaceFolderPaths(),
    ),
    document.uri.fsPath,
  );
}

function isExternalSageSourceDocument(document: vscode.TextDocument): boolean {
  const configuredRoots = (vscode.workspace.workspaceFolders ?? []).flatMap((folder) =>
    readSettings(folder).sourceRoots.map((root) =>
      path.isAbsolute(root) ? root : path.resolve(folder.uri.fsPath, root)
    )
  );
  const indexedRoots = (lastIndexStatus?.source_root_fingerprints ?? [])
    .map((fingerprint) => fingerprint.root)
    .filter((root): root is string => Boolean(root));
  const roots = resolveEffectiveSourceRootPaths({
    configuredRoots: [
      ...configuredRoots,
      ...(runtimeSourceRootDiscoveryController?.discoveredRoots ?? []),
    ],
    indexedRoots,
    workspaceFolders: [],
  });
  return isExternalSageSourceDocumentInRoots(document, roots);
}

function effectiveSourceRootPaths(
  settings: ReturnType<typeof readSettings>,
  workspaceFolders = workspaceFolderPaths(),
): string[] {
  const indexedRoots = (lastIndexStatus?.source_root_fingerprints ?? [])
    .map((fingerprint) => fingerprint.root)
    .filter((root): root is string => Boolean(root));
  return resolveEffectiveSourceRootPaths({
    configuredRoots: effectiveInitializationSourceRoots(settings),
    indexedRoots,
    workspaceFolders,
  });
}

function effectiveInitializationSourceRoots(settings: ReturnType<typeof readSettings>): string[] {
  return runtimeSourceRootDiscoveryController?.effectiveRoots(settings.sourceRoots)
    ?? [...settings.sourceRoots];
}

function clearLanguageServerStatusRefresh(): void {
  languageServerStatusRefreshController?.clear();
}

function languageClientLifecycleSnapshot(): Record<string, boolean | number> {
  return {
    ...(languageClientLifecycleController?.snapshot() ?? {
      launchCount: 0,
      managedCloseCount: 0,
      unexpectedCloseCount: 0,
      managedShutdownActive: false,
      restartQueued: false,
      operationInFlight: false,
      hasClient: false,
    }),
    configurationProfileUpdateDepth,
    suppressedConfigurationRestartCount,
  };
}

export async function activate(context: vscode.ExtensionContext): Promise<void> {
  extensionDeactivating = false;
  controllers = new Map<string, LanguageClientLifecycleController<LanguageClient>>();
  const statuses = new Map<string, { index?: IndexStatusSummary; docs?: DocsStatusSummary }>();
  let activeScope = "";
  const folderKey = (folder: vscode.WorkspaceFolder | undefined): string => folder?.uri.toString() ?? "__no_workspace__";
  const selectActiveScope = (): void => {
    const key = vscode.window.activeTextEditor?.document.uri.scheme === SAGE_SOURCE_SCHEME && activeScope
      ? activeScope
      : isLooseDocument(vscode.window.activeTextEditor?.document.uri)
        // A file outside every folder is served by the folder-less controller: no folder
        // server's document selector covers it.
        ? folderKey(undefined)
        : folderKey(activeWorkspaceFolder());
    if (key === activeScope) {
      languageClientLifecycleController = controllers.get(key);
      return;
    }
    if (activeScope) statuses.set(activeScope, { index: lastIndexStatus, docs: lastDocsStatus });
    activeScope = key;
    const status = statuses.get(key);
    lastIndexStatus = status?.index;
    lastDocsStatus = status?.docs;
    languageClientLifecycleController = controllers.get(key);
    void runtimeSourceRootDiscoveryController?.invalidateAndSchedule("active-project-change");
  };
  const currentProjectFolder = (): vscode.WorkspaceFolder | undefined => {
    const editor = vscode.window.activeTextEditor;
    if (editor?.document.uri.scheme === SAGE_SOURCE_SCHEME) {
      return vscode.workspace.workspaceFolders?.find((folder) => folderKey(folder) === activeScope)
        ?? activeWorkspaceFolder();
    }
    return activeWorkspaceFolder();
  };
  const outputChannel = vscode.window.createOutputChannel("Sage");
  const languageOutputChannel = vscode.window.createOutputChannel("Sage Language Server");
  const logger = createOutputLogger(outputChannel);
  const docsPanel = new DocumentationPanel();
  const terminalManager = new SageTerminalManager();
  const sageSourceProvider = new SageSourceTextDocumentProvider();
  const languageServerFileWatcher = vscode.workspace.createFileSystemWatcher(SAGE_LANGUAGE_FILE_GLOB);
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
  const buildEnvironmentPresentationInput = (languageServerStarting = false): EnvironmentPresentationInput => {
    const settings = readSettings(currentProjectFolder());
    const workspaceData = buildWorkspaceInitializationData(
      currentProjectFolder() ? [currentProjectFolder()!.uri.fsPath] : workspaceFolderPaths(),
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
      languageServerAvailable: Boolean(languageClientLifecycleController?.activeClient),
      languageServerFailed: languageClientLifecycleController?.hasStoppedRecovery,
      workspaceLabel: (vscode.workspace.workspaceFolders?.length ?? 0) > 1 ? currentProjectFolder()?.name : undefined,
    };
  };

  context.subscriptions.push(
    outputChannel,
    languageOutputChannel,
    docsPanel,
    terminalManager,
    sageSourceProvider,
    languageServerFileWatcher,
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
    return readSettings(activeDocument && activeDocument.uri.scheme !== SAGE_SOURCE_SCHEME
      ? vscode.workspace.getWorkspaceFolder(activeDocument.uri) ?? currentProjectFolder()
      : currentProjectFolder());
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

  const applyWorkspaceConfigurationProfile = async (
    profile: WorkspaceConfigurationProfile,
    reason: string,
    showCompletionMessage: boolean,
  ): Promise<TestConfigureWorkspaceProfileResult | undefined> => {
    const workspaceFolder = currentProjectFolder();
    if (!workspaceFolder) {
      void vscode.window.showWarningMessage("Open a workspace folder before configuring Sage.");
      return undefined;
    }

    const settings = readSettings(workspaceFolder);
    const profileWorkspaceFolders = [workspaceFolder.uri.fsPath];
    const discoveredSourceRoots = discoverSourceRoots(
      profileWorkspaceFolders,
      settings.sourceRoots,
      {
        interpreterPath: settings.interpreterPath,
        interpreterArgs: settings.interpreterArgs,
      },
    );
    const pythonConfiguration = vscode.workspace.getConfiguration("python", workspaceFolder.uri);
    const ruffConfiguration = vscode.workspace.getConfiguration("ruff", workspaceFolder.uri);
    const updates = buildWorkspaceConfigurationUpdates({
      workspaceFolders: profileWorkspaceFolders,
      discoveredSourceRoots,
      configuredExtraPaths: settings.extraPaths,
      configuredPythonExtraPaths: pythonConfiguration.get<string[]>("analysis.extraPaths", []),
      configuredPythonDiagnosticSeverityOverrides: pythonConfiguration.get("analysis.diagnosticSeverityOverrides"),
      configuredPythonExclude: pythonConfiguration.get<string[]>("analysis.exclude", []),
      configuredPythonIgnore: pythonConfiguration.get<string[]>("analysis.ignore", []),
      configuredRuffExclude: ruffConfiguration.get<string[]>("exclude", []),
      configuredRuffConfiguration: ruffConfiguration.get("configuration"),
      profile,
    });
    const applied: TestConfigureWorkspaceProfileResult["updates"] = [];
    configurationProfileUpdateDepth += 1;
    try {
      for (const update of updates) {
        const namespace = update.namespace ?? "sage";
        const configuration = vscode.workspace.getConfiguration(namespace, workspaceFolder.uri);
        try {
          await configuration.update(update.section, update.value, workspaceConfigurationTarget());
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
      languageClientLifecycleController?.startInBackground(reason, true);
    } else {
      await languageClientLifecycleController?.manualRestart();
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

  const updateStatusBar = (): void => {
    selectActiveScope();
    const policyInput = activationPolicyInput();
    if (!shouldExposeSageExperience(policyInput)) {
      statusBarItem.hide();
      return;
    }
    const settings = activeEditorSettings();
    logger.setLevel(settings.loggingLevel);
    const presentationInput = buildEnvironmentPresentationInput(Boolean(
      languageClientLifecycleController?.operation
      && !languageClientLifecycleController.activeClient
    ));
    statusBarItem.text = formatStatusBarText(presentationInput);
    statusBarItem.tooltip = formatStatusBarTooltip(presentationInput);
    statusBarItem.command = STATUS_MENU_COMMAND;
    statusBarItem.show();
  };

  const runIndexCacheMaintenance = async (reason: string): Promise<void> => {
    // Other projects or VS Code windows may have these databases open.
    // Startup inspects usage; explicit offline maintenance owns deletion.
    const report = await maintainIndexCache({
      dryRun: true,
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

  const scheduleRuntimeSourceRootDiscovery = (reason: string): void => {
    void runtimeSourceRootDiscoveryController?.schedule(reason);
  };

  const ensureLanguageClientReady = async (action: string): Promise<LanguageClient | undefined> => {
    selectActiveScope();
    const controller = languageClientLifecycleController;
    if (!(await ensureWorkspaceRuntimeAvailable(action))) {
      return undefined;
    }
    if (!controller?.activeClient) {
      if (!controller?.operation) {
        controller?.startInBackground(action, true);
      }
      const operation = controller?.operation;
      if (operation) {
        const waitResult = await vscode.window.withProgress(
          {
            location: vscode.ProgressLocation.Notification,
            title: "Starting Sage language server",
            cancellable: true,
          },
          async (_progress, token) => waitForOperationOrCancellation(
            operation,
            token,
          ),
        );
        if (waitResult === "cancelled") {
          logger.info("extension", "user stopped waiting for language server startup", { action });
          return undefined;
        }
      }
    }
    const activeClient = controller?.activeClient;
    if (!activeClient) {
      void vscode.window.showWarningMessage(controller?.hasStoppedRecovery
        ? "Sage language server stopped after repeated failures. Run 'Sage: Restart Language Server' to retry."
        : "Sage language server is not available yet.");
      return undefined;
    }
    return activeClient;
  };

  context.subscriptions.push(
    ...registerExternalSourceNavigationProviders({
      ensureLanguageClientReady,
      isExternalSourceDocument: isExternalSageSourceDocument,
      refreshExternalSourceDocument: (document) => sageSourceProvider.refresh(document.uri),
      logger,
    }),
  );

  const refreshLanguageServerStatus = async (
    controller = languageClientLifecycleController,
    scope = activeScope,
  ): Promise<void> => {
    const activeClient = controller?.activeClient;
    if (!activeClient) {
      statuses.delete(scope);
      if (scope === activeScope) {
        lastIndexStatus = undefined;
        lastDocsStatus = undefined;
      }
      updateStatusBar();
      return;
    }
    const cancellation = new vscode.CancellationTokenSource();
    try {
      const [indexStatus, docsStatus] = await withOperationTimeout(
        Promise.all([
          executeSageCommand<IndexStatusSummary>(
            activeClient,
            RUST_LSP_COMMANDS.indexStatus,
            [],
            cancellation.token,
          ),
          executeSageCommand<DocsStatusSummary>(
            activeClient,
            RUST_LSP_COMMANDS.docsStatus,
            [],
            cancellation.token,
          ),
        ]),
        LANGUAGE_SERVER_STATUS_REQUEST_TIMEOUT_MS,
        "Sage language server status refresh",
        () => cancellation.cancel(),
      );
      if (!controller?.isCurrent(activeClient) || extensionDeactivating) {
        return;
      }
      statuses.set(scope, { index: indexStatus ?? undefined, docs: docsStatus ?? undefined });
      if (scope === activeScope) {
        lastIndexStatus = indexStatus ?? undefined;
        lastDocsStatus = docsStatus ?? undefined;
        maybePromptForIndexMaintenance(lastIndexStatus);
      }
    } catch (error) {
      logger.warn("extension", "failed to refresh language server status", { error: String(error) });
    } finally {
      cancellation.dispose();
      if (!extensionDeactivating && controller?.isCurrent(activeClient) && scope === activeScope) {
        updateStatusBar();
      }
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

  languageServerStatusRefreshController?.dispose();
  languageServerStatusRefreshController = new LanguageServerStatusRefreshController({
    intervalMs: LANGUAGE_SERVER_STATUS_REFRESH_INTERVAL_MS,
    logEvery: LANGUAGE_SERVER_STATUS_REFRESH_LOG_EVERY,
    refresh: refreshLanguageServerStatus,
    snapshot: () => ({
      pendingJobs: lastIndexStatus?.pending_jobs ?? 0,
      pendingTask: lastIndexStatus?.pending_task ?? undefined,
    }),
    shouldContinue: () => !extensionDeactivating,
    logPending: (attempts, snapshot) => {
      logger.debug("extension", "language server status still pending; continuing automatic refresh", {
        pendingJobs: snapshot.pendingJobs,
        pendingTask: snapshot.pendingTask,
        attempts,
      });
    },
  });
  context.subscriptions.push(languageServerStatusRefreshController);

  const scheduleLanguageServerStatusRefresh = (): void => {
    languageServerStatusRefreshController?.schedule();
  };

  const looseDirectoriesServed = new Map<string, readonly string[] | undefined>();
  const rememberLooseDirectories = (scope: string): string[] | undefined => {
    const directories = looseDocumentDirectories(isSageDocument);
    looseDirectoriesServed.set(scope, directories);
    return directories;
  };
  const restartLooseControllerForNewDirectory = (): void => {
    const document = vscode.window.activeTextEditor?.document;
    if (!document || !isLooseDocument(document.uri) || !isSageDocument(document)) return;
    const controller = controllers.get(folderKey(undefined));
    const served = looseDirectoriesServed.get(folderKey(undefined));
    if (controller?.activeClient && !controller.operation && !served?.includes(path.dirname(document.uri.fsPath))) {
      void controller.manualRestart();
    }
  };

  const createScopedController = (folder?: vscode.WorkspaceFolder): LanguageClientLifecycleController<LanguageClient> => {
    const scope = folderKey(folder);
    let controller: LanguageClientLifecycleController<LanguageClient>;
    let startupFailureShown = false;
    let verifiedLaunch: ReturnType<typeof languageServerLaunch> | undefined;
    controller = new LanguageClientLifecycleController<LanguageClient>({
    createClient: (hooks) => createLanguageClient(context, languageOutputChannel, {
      fileSystemWatcher: languageServerFileWatcher,
      serverLaunch: verifiedLaunch,
      workspaceFolder: folder,
      looseDocumentDirectories: folder ? undefined : rememberLooseDirectories(scope),
      shouldAutoRestartOnClose: hooks.shouldAutoRestartOnClose,
      runtimeDiscoveredSourceRoots: activeScope === scope ? [
        ...(runtimeSourceRootDiscoveryController?.discoveredRoots ?? []),
      ] : [],
      onClose: hooks.onClose,
    }),
    startClient: (nextClient) => startLanguageClientWithTimeout(nextClient, {
      startTimeoutMs: LANGUAGE_SERVER_START_TIMEOUT_MS,
      cleanupTimeoutMs: LANGUAGE_SERVER_SHUTDOWN_TIMEOUT_MS,
      label: "Sage language client start",
      onCleanupError: (cleanupError) => {
        logger.warn("extension", "failed to clean up a language client after startup failure", {
          error: String(cleanupError),
        });
      },
    }),
    stopClient: (activeClient, label) => stopLanguageClientWithTimeout(
      activeClient,
      LANGUAGE_SERVER_SHUTDOWN_TIMEOUT_MS,
      label,
    ),
    beforeRestart: () => { if (scope === activeScope) clearLanguageServerStatusRefresh(); },
    beforeStart: async (signal) => {
      await runIndexCacheMaintenance("language-client-start");
      verifiedLaunch = languageServerLaunch(context, folder);
      await verifyLanguageServerLaunch(verifiedLaunch, { signal, cwd: context.extensionPath });
    },
    canRun: () => isWorkspaceRuntimeAvailable(currentWorkspaceRuntimeState()),
    shouldAutoStart: () => scope === activeScope && shouldAutoStartLanguageClient(activationPolicyInput()),
    onRuntimeUnavailable: () => {
      const workspaceRuntimeState = currentWorkspaceRuntimeState();
      statuses.delete(scope);
      if (scope === activeScope) {
        lastIndexStatus = undefined;
        lastDocsStatus = undefined;
        updateStatusBar();
      }
      logger.info("extension", "language client disabled by workspace runtime state", {
        trusted: workspaceRuntimeState.trusted,
        hasVirtualWorkspace: workspaceRuntimeState.hasVirtualWorkspace,
      });
    },
    afterStarted: async () => {
      startupFailureShown = false;
      await refreshLanguageServerStatus(controller, scope);
      if (scope === activeScope) scheduleLanguageServerStatusRefresh();
    },
    onStateChanged: () => { if (scope === activeScope) updateStatusBar(); },
    onSlowStartNotice: (reason) => {
      logger.info("extension", "language server is still starting in the background", { reason });
      updateStatusBar();
    },
    onStartError: (error) => {
      if (!startupFailureShown) {
        startupFailureShown = true;
        void vscode.window.showErrorMessage(
          error instanceof LanguageServerLaunchError ? error.message : "Sage language server startup failed. See the logs for details.",
          "Show Logs", "Restart", "Open Settings",
        ).then((action) => {
          if (action === "Show Logs") outputChannel.show();
          if (action === "Restart") { startupFailureShown = false; void controller.manualRestart(); }
          if (action === "Open Settings") void vscode.commands.executeCommand("workbench.action.openSettings", "sage.languageServer.rustPath");
        });
      }
      logger.error("extension", "language server failed to start", { error: String(error),
        command: error instanceof LanguageServerLaunchError ? error.command : undefined });
      updateStatusBar();
    },
    logger,
    shutdownTimeoutMs: LANGUAGE_SERVER_SHUTDOWN_TIMEOUT_MS,
    slowStartNoticeMs: SLOW_LANGUAGE_SERVER_NOTICE_MS,
  });
    return controller;
  };
  controllers.set(folderKey(undefined), createScopedController());
  for (const folder of vscode.workspace.workspaceFolders ?? []) {
    if (folder.uri.scheme === "file") controllers.set(folderKey(folder), createScopedController(folder));
  }
  selectActiveScope();

  runtimeSourceRootDiscoveryController = new RuntimeSourceRootDiscoveryController({
    loadCachedRoots: (snapshot) => {
      const key = JSON.stringify([snapshot.scopeKey, snapshot.interpreterPath, snapshot.interpreterArgs, snapshot.configuredSourceRoots]);
      const cached = context.globalState.get<Record<string, string[]>>("sage.runtimeSourceRoots.v1", {});
      return (cached[key] ?? []).filter((root) => fs.existsSync(path.join(root, "sage")));
    },
    saveCachedRoots: async (snapshot, roots) => {
      const key = JSON.stringify([snapshot.scopeKey, snapshot.interpreterPath, snapshot.interpreterArgs, snapshot.configuredSourceRoots]);
      const cached = context.globalState.get<Record<string, string[]>>("sage.runtimeSourceRoots.v1", {});
      const sageRoots = roots.filter((root) => fs.existsSync(path.join(root, "sage")));
      if (sageRoots.length > 0) {
        await context.globalState.update("sage.runtimeSourceRoots.v1", { ...cached, [key]: sageRoots });
      }
    },
    prepare: (reason) => {
      if (!isWorkspaceRuntimeAvailable(currentWorkspaceRuntimeState())) {
        return undefined;
      }
      if (!shouldAutoStartLanguageClient(activationPolicyInput())) {
        logger.debug("workspace", "runtime source-root discovery skipped outside Sage context", { reason });
        return undefined;
      }
      const settings = readSettings(currentProjectFolder());
      if (!settings.runtimeIntrospectionEnabled || !settings.interpreterPath) {
        return undefined;
      }
      return {
        scopeKey: currentProjectFolder()?.uri.toString() ?? "workspace",
        workspaceFolders: currentProjectFolder() ? [currentProjectFolder()!.uri.fsPath] : workspaceFolderPaths(),
        configuredSourceRoots: settings.sourceRoots,
        interpreterPath: settings.interpreterPath,
        interpreterArgs: settings.interpreterArgs,
      };
    },
    discoverStartupRoots: (snapshot, effectiveRoots) => discoverSourceRoots(
      [...snapshot.workspaceFolders],
      [...effectiveRoots],
      {
        interpreterPath: snapshot.interpreterPath,
        interpreterArgs: [...snapshot.interpreterArgs],
        runtimeProbe: false,
      },
    ),
    discoverRuntimeRoots: (snapshot, effectiveRoots, signal) => discoverSourceRootsAsync(
      [...snapshot.workspaceFolders],
      [...effectiveRoots],
      {
        interpreterPath: snapshot.interpreterPath,
        interpreterArgs: [...snapshot.interpreterArgs],
        signal,
      },
    ),
    onEvent: (event) => {
      if (event.type === "no-new-roots") {
        logger.debug("workspace", "runtime source-root discovery found no new roots", {
          reason: event.reason,
          elapsedMs: event.elapsedMs,
        });
        return;
      }
      if (event.type === "failed") {
        logger.warn("workspace", "runtime source-root discovery failed", {
          reason: event.reason,
          error: String(event.error),
        });
        return;
      }
      logger.info("workspace", "runtime source-root discovery added roots", {
        reason: event.reason,
        elapsedMs: event.elapsedMs,
        count: event.additions.length,
        roots: event.additions.join(","),
      });
      updateStatusBar();
      languageClientLifecycleController?.startInBackground("runtime-source-root-discovery", true);
    },
  });

  context.subscriptions.push(
    languageServerFileWatcher.onDidCreate(scheduleLanguageServerStatusRefresh),
    languageServerFileWatcher.onDidChange(scheduleLanguageServerStatusRefresh),
    languageServerFileWatcher.onDidDelete(scheduleLanguageServerStatusRefresh),
    vscode.workspace.registerTextDocumentContentProvider(
      SAGE_SOURCE_SCHEME,
      sageSourceProvider,
    ),
    vscode.workspace.onDidCloseTextDocument((document) => {
      sageSourceProvider.release(document.uri);
    }),
    vscode.window.onDidCloseTerminal((terminal) => {
      terminalManager.handleClosedTerminal(terminal);
    }),
    vscode.window.onDidChangeActiveTextEditor(async () => {
      await setWorkspaceContexts();
      cellCodeLensProvider.refresh();
      updateStatusBar();
      scheduleRuntimeSourceRootDiscovery("active-editor-change");
      selectActiveScope();
      restartLooseControllerForNewDirectory();
      if (
        !languageClientLifecycleController?.activeClient
        && !languageClientLifecycleController?.operation
        && shouldAutoStartLanguageClient(activationPolicyInput())
      ) {
        languageClientLifecycleController?.startInBackground("active-editor-change");
      }
    }),
    vscode.workspace.onDidGrantWorkspaceTrust(async () => {
      await setWorkspaceContexts();
      cellCodeLensProvider.refresh();
      scheduleRuntimeSourceRootDiscovery("workspace-trust-granted");
      for (const controller of controllers.values()) {
        controller.startInBackground("workspace-trust-granted");
      }
    }),
    vscode.commands.registerCommand("sage.openGettingStarted", async () => {
      await vscode.commands.executeCommand(
        "workbench.action.openWalkthrough",
        `${context.extension.id}#${GETTING_STARTED_WALKTHROUGH_ID}`,
        false,
      );
    }),
    vscode.commands.registerCommand(STATUS_MENU_COMMAND, async () => {
      const picked = await vscode.window.showQuickPick(statusMenuActions(), {
        title: "Sage Status",
        placeHolder: "Open diagnostics, rebuild the index, or copy a support bundle.",
        matchOnDescription: true,
        matchOnDetail: true,
      });
      if (picked) {
        await vscode.commands.executeCommand(picked.command);
      }
    }),
    vscode.commands.registerCommand("sage.selectInterpreter", async () => {
      if (!(await ensureWorkspaceRuntimeAvailable("Selecting a Sage interpreter"))) {
        return;
      }
      const workspaceFolder = currentProjectFolder();
      const settings = readSettings(workspaceFolder);
      const detectedOptions = discoverInterpreterCandidates({
        currentPath: settings.interpreterPath,
        workspaceFolders: workspaceFolder ? [workspaceFolder.uri.fsPath] : workspaceFolderPaths(),
      });
      const picked = await vscode.window.showQuickPick(detectedOptions, {
        title: "Select Sage environment",
        placeHolder: "Choose a detected Sage runtime, or enter a custom path.",
        matchOnDescription: true,
        matchOnDetail: true,
      });

      if (!picked) {
        return;
      }

      const updates = await resolveInterpreterConfigurationUpdates(picked, settings, {
        runtimePath: (initialValue) => vscode.window.showInputBox({
          title: "Sage runtime path",
          value: initialValue,
          prompt: "Enter the Sage executable used for run commands and the managed REPL.",
        }),
      });
      if (!updates || updates.length === 0) {
        return;
      }

      let shouldResetRepl = false;
      for (const update of updates) {
        await vscode.workspace
          .getConfiguration("sage", workspaceFolder?.uri ?? null)
          .update(update.section, update.value, workspaceConfigurationTarget());
        logger.info("configuration", "updated setting", {
          setting: `sage.${update.section}`,
          value: update.value,
        });
        shouldResetRepl ||= update.section === "interpreter.path";
      }
      if (shouldResetRepl) {
        terminalManager.resetReplTerminal(workspaceFolder?.uri.fsPath);
      }
      showExecutionStatus("Sage environment updated", {
        settings: updates.map((update) => `sage.${update.section}`).join(","),
      });
    }),
    vscode.commands.registerCommand("sage.restartLanguageServer", async () => {
      if (!(await ensureWorkspaceRuntimeAvailable("Restarting the Sage language server"))) {
        return;
      }
      logger.info("extension", "restarting language server");
      await languageClientLifecycleController?.manualRestart();
    }),
    vscode.commands.registerCommand("sage.configureWorkspace", async () => {
      if (!(await ensureWorkspaceRuntimeAvailable("Configuring the Sage workspace"))) {
        return;
      }
      const workspaceFolder = currentProjectFolder();
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
    ...registerExecutionCommands({
      terminalManager,
      ensureWorkspaceRuntimeAvailable,
      workspaceFolderPaths,
      activeProjectFolder: currentProjectFolder,
      showExecutionStatus,
    }),
    ...registerNavigationCommands({
      docsPanel,
      logger,
      ensureLanguageClientReady,
      activeOrVisibleSageEditor,
    }),
    registerUxSelfCheckCommand({
      ensureLanguageClientReady,
      activeOrVisibleSageEditor,
      workspaceFolderPaths,
      outputChannel,
      updateLanguageServerStatus: (indexStatus, docsStatus) => {
        lastIndexStatus = indexStatus;
        lastDocsStatus = docsStatus;
      },
      updateStatusBar,
    }),
    ...registerStatusCommands({
      context,
      outputChannel,
      logger,
      ensureLanguageClientReady,
      prepareDatabaseBuild: async () => {
        await runtimeSourceRootDiscoveryController?.invalidateAndSchedule("build-database");
        await languageClientLifecycleController?.operation;
        if (!effectiveSourceRootPaths(activeEditorSettings()).some((root) => fs.existsSync(path.join(root, "sage")))) {
          throw new Error("Sage sources were not found. Set sage.analysis.sourceRoots to the directory containing the sage package, then retry.");
        }
      },
      refreshLanguageServerStatus,
      activeEditorSettings,
      activeProjectFolder: currentProjectFolder,
      workspaceFolderPaths,
      currentWorkspaceRuntimeState,
      buildEnvironmentPresentationInput,
      languageClientLifecycleSnapshot,
      languageClientState: () => ({
        available: Boolean(languageClientLifecycleController?.activeClient),
        starting: Boolean(languageClientLifecycleController?.operation),
      }),
      getIndexStatus: () => lastIndexStatus,
      setIndexStatus: (status) => { lastIndexStatus = status; },
      getDocsStatus: () => lastDocsStatus,
      setDocsStatus: (status) => { lastDocsStatus = status; },
      updateStatusBar,
    }),
    vscode.workspace.onDidChangeConfiguration(async (event) => {
      if (!event.affectsConfiguration("sage")) {
        return;
      }
      if (event.affectsConfiguration("sage.interpreter.path") || event.affectsConfiguration("sage.interpreter.args")) {
        for (const folder of vscode.workspace.workspaceFolders ?? []) {
          if (event.affectsConfiguration("sage.interpreter.path", folder.uri)
            || event.affectsConfiguration("sage.interpreter.args", folder.uri)) {
            terminalManager.resetReplTerminal(folder.uri.fsPath);
          }
        }
      }
      for (const folder of vscode.workspace.workspaceFolders ?? []) {
        if (event.affectsConfiguration("sage.analysis.sourceRoots", folder.uri)
          || event.affectsConfiguration("sage.analysis.extraPaths", folder.uri)) {
          terminalManager.invalidateRepl(folder.uri.fsPath);
        }
      }
      const runtimeSourceRootInputsChanged = (
        event.affectsConfiguration("sage.interpreter.path")
        || event.affectsConfiguration("sage.interpreter.args")
        || event.affectsConfiguration("sage.analysis.sourceRoots")
        || event.affectsConfiguration("sage.analysis.enableRuntimeIntrospection")
      );
      if (runtimeSourceRootInputsChanged) {
        void runtimeSourceRootDiscoveryController?.invalidateAndSchedule("configuration-change");
      } else if (event.affectsConfiguration("sage.analysis.enablePythonFiles")) {
        scheduleRuntimeSourceRootDiscovery("python-file-analysis-change");
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
        {
          // The folder-less controller serves loose files even while folders are open.
          const controller = controllers.get(folderKey(undefined));
          if (controller?.snapshot().launchCount || controller?.operation || controller?.hasStoppedRecovery) void controller.manualRestart();
        }
        for (const folder of vscode.workspace.workspaceFolders ?? []) {
          if (shouldRestartLanguageServer((section) => event.affectsConfiguration(section, folder.uri))) {
            const controller = controllers.get(folderKey(folder));
            if (controller?.snapshot().launchCount || controller?.operation || controller?.hasStoppedRecovery) void controller.manualRestart();
          }
        }
      }
    }),
    vscode.workspace.onDidChangeWorkspaceFolders(async (event) => {
      for (const folder of event.removed) {
        const key = folderKey(folder);
        const controller = controllers.get(key);
        controllers.delete(key);
        statuses.delete(key);
        if (controller) await controller.deactivate();
        terminalManager.resetReplTerminal(folder.uri.fsPath);
      }
      for (const folder of event.added) {
        if (folder.uri.scheme === "file") controllers.set(folderKey(folder), createScopedController(folder));
      }
      if (activeScope && controllers.has(activeScope)) {
        statuses.set(activeScope, { index: lastIndexStatus, docs: lastDocsStatus });
      }
      activeScope = "";
      selectActiveScope();
      await setWorkspaceContexts();
      cellCodeLensProvider.refresh();
      void runtimeSourceRootDiscoveryController?.invalidateAndSchedule("workspace-folders-changed");
      updateStatusBar();
      languageClientLifecycleController?.startInBackground("workspace-folders-changed");
    }),
  );

  if (context.extensionMode !== vscode.ExtensionMode.Production) {
    context.subscriptions.push(
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
        if (languageClientLifecycleController?.operation) {
          await languageClientLifecycleController.operation;
        }
        return languageClientLifecycleSnapshot();
      }),
      vscode.commands.registerCommand("sage.__test.restartLanguageServerAndWait", async () => {
        await languageClientLifecycleController?.start();
        return languageClientLifecycleSnapshot();
      }),
      vscode.commands.registerCommand(
        "sage.__test.configureWorkspaceProfile",
        async (profileId: WorkspaceConfigurationProfileId = "research") => {
          const profile = WORKSPACE_CONFIGURATION_PROFILES.find((entry) => entry.id === profileId);
          if (!profile) {
            throw new Error(`Unknown Sage workspace profile: ${profileId}`);
          }
          return applyWorkspaceConfigurationProfile(profile, "test-configure-workspace", false);
        },
      ),
    );
  }

  void setWorkspaceContexts();
  updateStatusBar();
  scheduleRuntimeSourceRootDiscovery("activation");
  languageClientLifecycleController?.startInBackground("activation");
}

export async function deactivate(): Promise<void> {
  extensionDeactivating = true;
  clearLanguageServerStatusRefresh();
  const sourceRootDiscovery = runtimeSourceRootDiscoveryController?.beginDeactivation();
  await withOperationTimeout(Promise.all([
    ...[...controllers.values()].map((controller) => controller.deactivate()),
    sourceRootDiscovery,
  ]), LANGUAGE_SERVER_SHUTDOWN_TIMEOUT_MS, "Sage extension shutdown").catch(() => undefined);
}
