import { settleAndStopLanguageClient } from "./languageClientShutdown";
import { isPermanentStartError } from "./serverStartup";
import { stopLanguageClientWithTimeout, type StartableLanguageClient } from "./languageClientOperations";
export interface LanguageClientLifecycleHooks {
  shouldAutoRestartOnClose(): boolean;
  onClose(event: { managedShutdown: boolean }): void;
}
export interface LanguageClientLifecycleSnapshot {
  launchCount: number;
  managedCloseCount: number;
  unexpectedCloseCount: number;
  managedShutdownActive: boolean;
  restartQueued: boolean;
  operationInFlight: boolean;
  hasClient: boolean;
  recoveryStopped: boolean;
  consecutiveFailures: number;
}

interface LanguageClientLifecycleLogger {
  info(component: string, message: string, fields?: Record<string, unknown>): void;
  warn(component: string, message: string, fields?: Record<string, unknown>): void;
  error(component: string, message: string, fields?: Record<string, unknown>): void;
}

export interface LanguageClientLifecycleControllerOptions<TClient extends StartableLanguageClient> {
  createClient(hooks: LanguageClientLifecycleHooks): TClient;
  startClient(client: TClient): Promise<void>;
  stopClient(client: TClient, label: string): Promise<void>;
  beforeRestart(): void;
  beforeStart(signal: AbortSignal): Promise<void>;
  canRun(): boolean;
  shouldAutoStart(): boolean;
  onRuntimeUnavailable(): void;
  afterStarted(client: TClient): Promise<void>;
  onStateChanged(): void;
  onSlowStartNotice(reason: string): void;
  onStartError(error: unknown): void;
  logger: LanguageClientLifecycleLogger;
  shutdownTimeoutMs: number;
  slowStartNoticeMs: number;
  setTimeout?: typeof setTimeout;
  clearTimeout?: typeof clearTimeout;
}

export class LanguageClientLifecycleController<TClient extends StartableLanguageClient> {
  private client: TClient | undefined;
  private pendingClient: TClient | undefined;
  private currentOperation: Promise<void> | undefined;
  private restartQueued = false;
  private managedShutdown = false;
  private launchCount = 0;
  private managedCloseCount = 0;
  private unexpectedCloseCount = 0;
  private deactivating = false;
  private startupAbort: AbortController | undefined;
  private slowNoticeTimer: ReturnType<typeof setTimeout> | undefined;
  private slowNoticeShown = false;
  private recoveryTimer: ReturnType<typeof setTimeout> | undefined;
  private failureTimes: number[] = [];
  private recoveryStopped = false;
  // Bumped by every manual restart so a failure from an older attempt cannot cancel it.
  private restartGeneration = 0;
  private readonly scheduleTimeout: typeof setTimeout;
  private readonly cancelTimeout: typeof clearTimeout;

  constructor(private readonly options: LanguageClientLifecycleControllerOptions<TClient>) {
    this.scheduleTimeout = options.setTimeout ?? setTimeout;
    this.cancelTimeout = options.clearTimeout ?? clearTimeout;
  }

  get activeClient(): TClient | undefined {
    return this.client;
  }

  get operation(): Promise<void> | undefined {
    return this.currentOperation;
  }

  get isDeactivating(): boolean {
    return this.deactivating;
  }

  get hasStoppedRecovery(): boolean {
    return this.recoveryStopped;
  }

  manualRestart(): Promise<void> {
    this.clearRecoveryTimer();
    this.failureTimes = [];
    this.recoveryStopped = false;
    this.restartGeneration += 1;
    return this.start();
  }

  isCurrent(client: TClient): boolean {
    return this.client === client;
  }

  snapshot(): LanguageClientLifecycleSnapshot {
    return {
      launchCount: this.launchCount,
      managedCloseCount: this.managedCloseCount,
      unexpectedCloseCount: this.unexpectedCloseCount,
      managedShutdownActive: this.managedShutdown,
      restartQueued: this.restartQueued,
      operationInFlight: Boolean(this.currentOperation),
      hasClient: Boolean(this.client),
      recoveryStopped: this.recoveryStopped,
      consecutiveFailures: this.failureTimes.length,
    };
  }

  async start(): Promise<void> {
    if (this.deactivating || this.recoveryStopped) {
      return;
    }
    this.clearRecoveryTimer();
    this.restartQueued = true;
    while (!this.deactivating) {
      if (!this.currentOperation) {
        this.currentOperation = this.runRestartLoop().finally(() => {
          this.currentOperation = undefined;
          this.clearSlowStartNotice();
          this.options.onStateChanged();
        });
        this.options.onStateChanged();
      }
      await this.currentOperation;
      if (!this.restartQueued) {
        return;
      }
    }
  }

  startInBackground(reason: string, force = false): void {
    if (this.deactivating || this.recoveryStopped || this.recoveryTimer) {
      return;
    }
    if (!force && !this.options.shouldAutoStart()) {
      this.options.logger.info(
        "extension",
        "language client auto-start skipped outside Sage context",
        { reason },
      );
      this.options.onStateChanged();
      return;
    }
    this.options.logger.info("extension", "starting language client in background", { reason });
    this.scheduleSlowStartNotice(reason);
    void this.start().catch((error) => {
      this.options.logger.error("extension", "background language client start failed", {
        reason,
        error: String(error),
      });
    });
  }

  async deactivate(): Promise<void> {
    this.deactivating = true;
    this.startupAbort?.abort();
    this.clearRecoveryTimer();
    this.restartQueued = false;
    this.clearSlowStartNotice();
    await settleAndStopLanguageClient({
      operation: this.currentOperation,
      timeoutMs: this.options.shutdownTimeoutMs,
      takeClient: () => {
        const active = this.client ?? this.pendingClient;
        this.client = undefined;
        this.pendingClient = undefined;
        if (active) { this.managedShutdown = true; this.managedCloseCount += 1; }
        return active;
      },
      afterStop: () => { this.managedShutdown = false; },
    });
  }

  private async runRestartLoop(): Promise<void> {
    while (this.restartQueued && !this.deactivating) {
      this.restartQueued = false;
      const generation = this.restartGeneration;
      this.options.beforeRestart();

      if (this.client) {
        const previousClient = this.client;
        this.client = undefined;
        this.managedShutdown = true;
        this.managedCloseCount += 1;
        try {
          await this.options.stopClient(
            previousClient,
            "Sage language client stop during restart",
          );
        } catch (error) {
          this.options.logger.warn(
            "extension",
            "language client stop did not complete before restart",
            { error: String(error) },
          );
        } finally {
          this.managedShutdown = false;
        }
      }
      if (this.deactivating) {
        break;
      }

      if (!this.options.canRun()) {
        this.options.onRuntimeUnavailable();
        continue;
      }

      try {
        this.startupAbort = new AbortController();
        await this.options.beforeStart(this.startupAbort.signal);
        if (this.deactivating) {
          break;
        }
        const nextClient: TClient = this.options.createClient({
          shouldAutoRestartOnClose: () => (
            this.client === nextClient
            && !this.managedShutdown
            && !this.deactivating
          ),
          onClose: ({ managedShutdown }) => {
            if (!managedShutdown && this.client === nextClient && !this.deactivating) {
              this.unexpectedCloseCount += 1;
              this.client = undefined;
              this.scheduleRecovery();
            }
          },
        });
        this.launchCount += 1;
        this.pendingClient = nextClient;
        await this.options.startClient(nextClient);
        if (this.deactivating) {
          if (this.pendingClient !== nextClient) {
            // deactivate() already took ownership of a startup that exceeded
            // its bounded wait and stopped the best available client handle.
            continue;
          }
          // Claim the pending handle before awaiting stop. deactivate() may
          // time out while this stop is still in flight and must not stop the
          // same LanguageClient a second time.
          this.pendingClient = undefined;
          this.managedShutdown = true;
          this.managedCloseCount += 1;
          try {
            await nextClient.stop();
          } finally {
            this.managedShutdown = false;
          }
          continue;
        }
        this.client = nextClient;
        this.pendingClient = undefined;
        await this.options.afterStarted(nextClient);
        this.options.logger.info("extension", "language client started", {
          launchCount: this.launchCount,
        });
      } catch (error) {
        const failedClient = this.client;
        this.client = undefined;
        this.pendingClient = undefined;
        if (failedClient) {
          this.managedCloseCount += 1;
          await stopLanguageClientWithTimeout(failedClient, this.options.shutdownTimeoutMs,
            "Sage language client cleanup after failed start").catch(() => undefined);
        }
        if (this.deactivating) {
          continue;
        }
        if (generation !== this.restartGeneration) {
          // The configuration was corrected while this attempt was still running (for example
          // during the server version handshake). Its failure is stale: run the queued restart.
          this.restartQueued = true;
          continue;
        }
        if (isPermanentStartError(error)) { this.recoveryStopped = true; this.restartQueued = false; }
        this.options.onStartError(error);
        this.scheduleRecovery();
      }
    }
  }

  private scheduleSlowStartNotice(reason: string): void {
    if (this.slowNoticeShown || this.slowNoticeTimer) {
      return;
    }
    this.slowNoticeTimer = this.scheduleTimeout(() => {
      this.slowNoticeTimer = undefined;
      if (this.client || !this.currentOperation) {
        return;
      }
      this.slowNoticeShown = true;
      this.options.onSlowStartNotice(reason);
    }, this.options.slowStartNoticeMs);
  }

  private clearSlowStartNotice(): void {
    if (this.slowNoticeTimer !== undefined) {
      this.cancelTimeout(this.slowNoticeTimer);
      this.slowNoticeTimer = undefined;
    }
  }

  private scheduleRecovery(): void {
    if (this.deactivating || this.recoveryStopped) {
      return;
    }
    const now = Date.now();
    this.failureTimes = this.failureTimes.filter((time) => now - time < 3 * 60_000);
    this.failureTimes.push(now);
    if (this.failureTimes.length >= 5) {
      this.recoveryStopped = true;
      this.options.logger.error("extension", "language server recovery stopped after repeated failures", {
        failures: this.failureTimes.length,
      });
      this.options.onStateChanged();
      return;
    }
    this.clearRecoveryTimer();
    const delay = Math.min(1000 * 2 ** (this.failureTimes.length - 1), 8000);
    this.options.logger.warn("extension", "language server will restart after an unexpected failure", {
      failures: this.failureTimes.length,
      delayMs: delay,
    });
    this.recoveryTimer = this.scheduleTimeout(() => {
      this.recoveryTimer = undefined;
      this.startInBackground("automatic-recovery", true);
    }, delay);
    this.options.onStateChanged();
  }

  private clearRecoveryTimer(): void {
    if (this.recoveryTimer !== undefined) {
      this.cancelTimeout(this.recoveryTimer);
      this.recoveryTimer = undefined;
    }
  }
}
