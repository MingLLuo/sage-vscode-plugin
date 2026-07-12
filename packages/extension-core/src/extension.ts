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
  languageServerUriForDocument,
  registerExternalSourceNavigationProviders,
} from "./externalSourceNavigation";
import {
  createLanguageClient,
  executeSageCommand,
  RUST_LSP_COMMANDS,
  SAGE_LANGUAGE_FILE_GLOB,
  rustIndexCacheDir,
} from "./languageClient";
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
import {
  buildQueryRequestPayload,
  diagnosticCodeLabel,
  diagnosticRangeLabel,
  formatUxSelfCheckReport,
  measureAsync,
  shouldRunFullUxSelfCheckQuery,
  type QueryResponse,
} from "./uxSelfCheck";
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
import { updateWorkspaceSettingJson } from "./workspaceSettingsJson";
import {
  effectiveSourceRootPaths as resolveEffectiveSourceRootPaths,
  sourceRootContainsDocument,
} from "./sourceRootPaths";
import { LanguageServerStatusRefreshController } from "./statusRefreshController";

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
let languageServerStatusRefreshController: LanguageServerStatusRefreshController | undefined;
let slowLanguageServerNoticeTimer: ReturnType<typeof setTimeout> | undefined;
let slowLanguageServerNoticeShown = false;
let runtimeDiscoveredSourceRoots: string[] = [];
let runtimeSourceRootDiscoveryOperation: Promise<void> | undefined;
let runtimeSourceRootDiscoveryGeneration = 0;
let runtimeSourceRootDiscoveryOperationId = 0;
let extensionDeactivating = false;
const shownIndexMaintenanceNotices = new Set<string>();

const LANGUAGE_SERVER_STATUS_REFRESH_INTERVAL_MS = 1500;
const LANGUAGE_SERVER_STATUS_REFRESH_LOG_EVERY = 12;
const SLOW_LANGUAGE_SERVER_NOTICE_MS = 8000;
const GETTING_STARTED_WALKTHROUGH_ID = "gettingStarted";

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
  let source = "";

  try {
    const existing = await vscode.workspace.fs.readFile(settingsUri);
    source = Buffer.from(existing).toString("utf8");
  } catch (error) {
    if (!(error instanceof vscode.FileSystemError && error.code === "FileNotFound")) {
      throw error;
    }
  }

  const updated = updateWorkspaceSettingJson(source, setting, value);
  await vscode.workspace.fs.createDirectory(vscodeDirectoryUri);
  await vscode.workspace.fs.writeFile(
    settingsUri,
    Buffer.from(updated, "utf8"),
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
  if (document.uri.scheme === SAGE_SOURCE_SCHEME) {
    return isExternalSageSourceDocument(document);
  }
  if (document.uri.scheme !== "file" || document.languageId !== "python") {
    return false;
  }
  return sourceRootContainsDocument(effectiveSourceRootPaths(settings), document.uri.fsPath);
}

function isExternalSageSourceDocument(document: vscode.TextDocument): boolean {
  const settings = readSettings(vscode.workspace.workspaceFolders?.[0]);
  return isExternalSageSourceDocumentInRoots(document, effectiveSourceRootPaths(settings));
}

function effectiveSourceRootPaths(settings: ReturnType<typeof readSettings>): string[] {
  const indexedRoots = (lastIndexStatus?.source_root_fingerprints ?? [])
    .map((fingerprint) => fingerprint.root)
    .filter((root): root is string => Boolean(root));
  return resolveEffectiveSourceRootPaths({
    configuredRoots: effectiveInitializationSourceRoots(settings),
    indexedRoots,
    workspaceFolders: workspaceFolderPaths(),
  });
}

function effectiveInitializationSourceRoots(settings: ReturnType<typeof readSettings>): string[] {
  return dedupeStrings([...settings.sourceRoots, ...runtimeDiscoveredSourceRoots]);
}

function dedupeStrings(values: readonly string[]): string[] {
  return [...new Set(values)];
}

function clearLanguageServerStatusRefresh(): void {
  languageServerStatusRefreshController?.clear();
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

export async function activate(context: vscode.ExtensionContext): Promise<void> {
  extensionDeactivating = false;
  const outputChannel = vscode.window.createOutputChannel("Sage");
  const languageOutputChannel = vscode.window.createOutputChannel("Sage Language Server");
  const logger = createOutputLogger(outputChannel);
  const docsPanel = new DocumentationPanel();
  const terminalManager = new SageTerminalManager();
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
    docsPanel,
    terminalManager,
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
    statusBarItem.command = STATUS_MENU_COMMAND;
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
    if (extensionDeactivating) {
      return;
    }
    languageClientRestartQueued = true;
    if (languageClientOperation) {
      await languageClientOperation;
      return;
    }

    languageClientOperation = (async () => {
      while (languageClientRestartQueued && !extensionDeactivating) {
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
        if (extensionDeactivating) {
          break;
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
          if (extensionDeactivating) {
            break;
          }
          const nextClient = createLanguageClient(context, languageOutputChannel, {
            fileSystemWatcher: languageServerFileWatcher,
            shouldAutoRestartOnClose: () => !languageClientManagedShutdown && !extensionDeactivating,
            runtimeDiscoveredSourceRoots,
            onClose: ({ managedShutdown }) => {
              if (!managedShutdown) {
                languageClientUnexpectedCloseCount += 1;
              }
            },
          });
          languageClientLaunchCount += 1;
          await nextClient.start();
          if (extensionDeactivating) {
            languageClientManagedShutdown = true;
            languageClientManagedCloseCount += 1;
            try {
              await nextClient.stop();
            } finally {
              languageClientManagedShutdown = false;
            }
            continue;
          }
          client = nextClient;
          await refreshLanguageServerStatus();
          scheduleLanguageServerStatusRefresh();
          logger.info("extension", "language client started", { launchCount: languageClientLaunchCount });
        } catch (error) {
          client = undefined;
          if (extensionDeactivating) {
            continue;
          }
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
    if (extensionDeactivating) {
      return;
    }
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
    if (
      extensionDeactivating
      || runtimeSourceRootDiscoveryOperation
      || !isWorkspaceRuntimeAvailable(currentWorkspaceRuntimeState())
    ) {
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

    const operationId = ++runtimeSourceRootDiscoveryOperationId;
    const operation = (async () => {
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
        if (extensionDeactivating || discoveryGeneration !== runtimeSourceRootDiscoveryGeneration) {
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
        if (runtimeSourceRootDiscoveryOperationId === operationId) {
          runtimeSourceRootDiscoveryOperation = undefined;
          if (!extensionDeactivating && discoveryGeneration !== runtimeSourceRootDiscoveryGeneration) {
            scheduleRuntimeSourceRootDiscovery("superseded-runtime-discovery");
          }
        }
      }
    })();
    runtimeSourceRootDiscoveryOperation = operation;
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
    ...registerExternalSourceNavigationProviders({
      ensureLanguageClientReady,
      isExternalSourceDocument: isExternalSageSourceDocument,
      logger,
    }),
  );

  const refreshLanguageServerStatus = async (): Promise<void> => {
    const activeClient = client;
    if (!activeClient) {
      lastIndexStatus = undefined;
      lastDocsStatus = undefined;
      updateStatusBar();
      return;
    }
    try {
      const [indexStatus, docsStatus] = await Promise.all([
        executeSageCommand<IndexStatusSummary>(activeClient, RUST_LSP_COMMANDS.indexStatus),
        executeSageCommand<DocsStatusSummary>(activeClient, RUST_LSP_COMMANDS.docsStatus),
      ]);
      if (client !== activeClient || extensionDeactivating) {
        return;
      }
      lastIndexStatus = indexStatus ?? undefined;
      lastDocsStatus = docsStatus ?? undefined;
      maybePromptForIndexMaintenance(lastIndexStatus);
    } catch (error) {
      logger.warn("extension", "failed to refresh language server status", { error: String(error) });
    } finally {
      if (!extensionDeactivating && client === activeClient) {
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

      const updates = await resolveInterpreterConfigurationUpdates(picked, settings, {
        runtimePath: (initialValue) => vscode.window.showInputBox({
          title: "Sage runtime path",
          value: initialValue,
          prompt: "Enter the Sage executable used for run commands and the managed REPL.",
        }),
        languageServerPythonPath: (initialValue) => vscode.window.showInputBox({
          title: "Language-server Python path",
          value: initialValue,
          prompt: "Enter the Python executable used to run sage_lsp.",
        }),
      });
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
      showExecutionStatus("Sage environment updated", {
        settings: updates.map((update) => `sage.${update.section}`).join(","),
      });
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
    ...registerExecutionCommands({
      terminalManager,
      ensureWorkspaceRuntimeAvailable,
      workspaceFolderPaths,
      showExecutionStatus,
    }),
    ...registerNavigationCommands({
      docsPanel,
      logger,
      ensureLanguageClientReady,
      activeOrVisibleSageEditor,
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
      const languageServerUri = languageServerUriForDocument(editor.document);
      const selfCheckStarted = Date.now();
      const queryStarted = Date.now();
      let query = await executeSageCommand<QueryResponse>(
        activeClient,
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
      if (shouldRunFullUxSelfCheckQuery(query, workspaceFolderPaths())) {
        const fullQueryStarted = Date.now();
        query = await executeSageCommand<QueryResponse>(
          activeClient,
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
        measureAsync(() => executeSageCommand<IndexStatusSummary>(activeClient, RUST_LSP_COMMANDS.indexStatus)),
        measureAsync(() => executeSageCommand<DocsStatusSummary>(activeClient, RUST_LSP_COMMANDS.docsStatus)),
      ]);
      lastIndexStatus = indexStatusResult.value ?? undefined;
      lastDocsStatus = docsStatusResult.value ?? undefined;
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
      outputChannel.clear();
      outputChannel.appendLine(result.report);
      outputChannel.show(true);
      void vscode.window.showInformationMessage(`Sage UX Self Check: ${result.passed}/${result.total} checks passing`);
    }),
    ...registerStatusCommands({
      context,
      outputChannel,
      logger,
      ensureLanguageClientReady,
      refreshLanguageServerStatus,
      activeEditorSettings,
      workspaceFolderPaths,
      currentWorkspaceRuntimeState,
      buildEnvironmentPresentationInput,
      languageClientLifecycleSnapshot,
      languageClientState: () => ({
        available: Boolean(client),
        starting: Boolean(languageClientOperation),
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
        terminalManager.resetReplTerminal();
      }
      if (
        event.affectsConfiguration("sage.interpreter.path")
        || event.affectsConfiguration("sage.interpreter.args")
        || event.affectsConfiguration("sage.analysis.sourceRoots")
      ) {
        runtimeDiscoveredSourceRoots = [];
        runtimeSourceRootDiscoveryGeneration += 1;
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
      updateStatusBar();
      scheduleRuntimeSourceRootDiscovery("workspace-folders-changed");
      startLanguageClientInBackground("workspace-folders-changed");
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
        if (languageClientOperation) {
          await languageClientOperation;
        }
        return languageClientLifecycleSnapshot();
      }),
      vscode.commands.registerCommand("sage.__test.restartLanguageServerAndWait", async () => {
        await startLanguageClient();
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
  startLanguageClientInBackground("activation");
}

export async function deactivate(): Promise<void> {
  extensionDeactivating = true;
  languageClientRestartQueued = false;
  runtimeSourceRootDiscoveryGeneration += 1;
  clearLanguageServerStatusRefresh();
  clearSlowLanguageServerNotice();
  const sourceRootDiscovery = runtimeSourceRootDiscoveryOperation;
  try {
    if (languageClientOperation) {
      await languageClientOperation;
    }
    if (client) {
      const activeClient = client;
      client = undefined;
      languageClientManagedShutdown = true;
      languageClientManagedCloseCount += 1;
      try {
        await activeClient.stop();
      } finally {
        languageClientManagedShutdown = false;
      }
    }
  } finally {
    await sourceRootDiscovery;
  }
}
