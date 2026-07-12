export interface LanguageServerStatusRefreshSnapshot {
  pendingJobs: number;
  pendingTask?: string;
}

export interface LanguageServerStatusRefreshControllerOptions {
  intervalMs: number;
  logEvery: number;
  refresh(): Promise<void>;
  snapshot(): LanguageServerStatusRefreshSnapshot;
  shouldContinue(): boolean;
  logPending(attempts: number, snapshot: LanguageServerStatusRefreshSnapshot): void;
  setInterval?: typeof setInterval;
  clearInterval?: typeof clearInterval;
}

export class LanguageServerStatusRefreshController {
  private timer: ReturnType<typeof setInterval> | undefined;
  private attempts = 0;
  private inFlight = false;
  private generation = 0;
  private readonly startInterval: typeof setInterval;
  private readonly stopInterval: typeof clearInterval;

  constructor(private readonly options: LanguageServerStatusRefreshControllerOptions) {
    this.startInterval = options.setInterval ?? setInterval;
    this.stopInterval = options.clearInterval ?? clearInterval;
  }

  schedule(): void {
    if (!this.options.shouldContinue()) {
      return;
    }
    this.clear();
    const generation = this.generation;
    this.timer = this.startInterval(() => {
      void this.tick(generation);
    }, this.options.intervalMs);
  }

  clear(): void {
    this.generation += 1;
    if (this.timer !== undefined) {
      this.stopInterval(this.timer);
      this.timer = undefined;
    }
    this.attempts = 0;
  }

  dispose(): void {
    this.clear();
  }

  private async tick(generation: number): Promise<void> {
    if (generation !== this.generation || this.inFlight || !this.options.shouldContinue()) {
      return;
    }
    this.inFlight = true;
    try {
      await this.options.refresh();
    } finally {
      this.inFlight = false;
      if (generation !== this.generation) {
        return;
      }
      this.attempts += 1;
      const snapshot = this.options.snapshot();
      if (snapshot.pendingJobs === 0 || !this.options.shouldContinue()) {
        this.clear();
        return;
      }
      if (this.attempts % this.options.logEvery === 0) {
        this.options.logPending(this.attempts, snapshot);
      }
    }
  }
}
