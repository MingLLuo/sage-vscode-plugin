export interface TerminableTaskExecution {
  terminate(): void;
}

export function buildRunTaskDefinition(
  filePath: string,
  activationNonce: string,
  invocation: number,
): { type: "sage.runFile"; file: string; invocation: string } {
  return {
    type: "sage.runFile",
    file: filePath,
    invocation: `${activationNonce}:${invocation}`,
  };
}

export class RunTaskLifecycle<T extends object & TerminableTaskExecution> {
  private readonly tasks = new Map<T, string | undefined>();
  private readonly endedTasks = new WeakSet<T>();
  private pendingLaunches = 0;
  private disposed = false;
  private disposedIdleNotified = false;

  constructor(
    private readonly cleanup: (filePath: string | undefined) => void,
    private readonly onDisposedIdle: () => void,
  ) {}

  assertActive(): void {
    if (this.disposed) {
      throw new Error("Run task lifecycle has been disposed.");
    }
  }

  beginLaunch(): void {
    this.assertActive();
    this.pendingLaunches += 1;
  }

  completeLaunch(execution: T, cleanupPath: string | undefined): void {
    this.finishPendingLaunch();
    this.tasks.set(execution, cleanupPath);
    if (this.endedTasks.has(execution)) {
      this.completeTask(execution);
    } else if (this.disposed) {
      execution.terminate();
    }
    this.notifyDisposedIdle();
  }

  failLaunch(): void {
    this.finishPendingLaunch();
    this.notifyDisposedIdle();
  }

  end(execution: T): void {
    if (!this.tasks.has(execution)) {
      if (this.pendingLaunches > 0) {
        this.endedTasks.add(execution);
      }
      return;
    }
    this.completeTask(execution);
    if (this.pendingLaunches > 0) {
      this.endedTasks.add(execution);
    }
  }

  dispose(): void {
    if (this.disposed) {
      return;
    }
    this.disposed = true;
    for (const execution of this.tasks.keys()) {
      execution.terminate();
    }
    this.notifyDisposedIdle();
  }

  snapshot(): { activeTasks: number; pendingLaunches: number; disposed: boolean } {
    return {
      activeTasks: this.tasks.size,
      pendingLaunches: this.pendingLaunches,
      disposed: this.disposed,
    };
  }

  private finishPendingLaunch(): void {
    if (this.pendingLaunches === 0) {
      throw new Error("Run task launch completed without a matching beginLaunch().");
    }
    this.pendingLaunches -= 1;
  }

  private completeTask(execution: T): void {
    const cleanupPath = this.tasks.get(execution);
    this.tasks.delete(execution);
    this.endedTasks.delete(execution);
    this.cleanup(cleanupPath);
    this.notifyDisposedIdle();
  }

  private notifyDisposedIdle(): void {
    if (
      this.disposed
      && !this.disposedIdleNotified
      && this.tasks.size === 0
      && this.pendingLaunches === 0
    ) {
      this.disposedIdleNotified = true;
      this.onDisposedIdle();
    }
  }
}
