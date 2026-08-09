import { withOperationTimeout } from "./boundedOperation";
import type { StartableLanguageClient } from "./languageClientOperations";

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
  beforeStart(): Promise<void>;
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
  private slowNoticeTimer: ReturnType<typeof setTimeout> | undefined;
  private slowNoticeShown = false;
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
    };
  }

  async start(): Promise<void> {
    if (this.deactivating) {
      return;
    }
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
    if (this.deactivating) {
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
    this.restartQueued = false;
    this.clearSlowStartNotice();
    const operation = this.currentOperation;
    if (operation) {
      try {
        await withOperationTimeout(
          operation,
          this.options.shutdownTimeoutMs,
          "Sage language client operation during shutdown",
        );
      } catch {
        // Continue with the best available client handle. A status request or
        // startup handshake must never keep the extension host alive forever.
      }
    }

    const activeClient = this.client ?? this.pendingClient;
    if (!activeClient) {
      return;
    }
    this.client = undefined;
    this.pendingClient = undefined;
    this.managedShutdown = true;
    this.managedCloseCount += 1;
    try {
      await withOperationTimeout(
        Promise.resolve().then(() => activeClient.stop()),
        this.options.shutdownTimeoutMs,
        "Sage language client stop",
      );
    } catch {
      // VS Code is already deactivating; bounded shutdown is preferable to
      // waiting indefinitely for a failed child-process transport.
    } finally {
      this.managedShutdown = false;
    }
  }

  private async runRestartLoop(): Promise<void> {
    while (this.restartQueued && !this.deactivating) {
      this.restartQueued = false;
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
        await this.options.beforeStart();
        if (this.deactivating) {
          break;
        }
        let nextClient: TClient;
        nextClient = this.options.createClient({
          shouldAutoRestartOnClose: () => (
            this.client === nextClient
            && !this.managedShutdown
            && !this.deactivating
          ),
          onClose: ({ managedShutdown }) => {
            if (!managedShutdown) {
              this.unexpectedCloseCount += 1;
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
        this.client = undefined;
        this.pendingClient = undefined;
        if (this.deactivating) {
          continue;
        }
        this.options.onStartError(error);
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
}
