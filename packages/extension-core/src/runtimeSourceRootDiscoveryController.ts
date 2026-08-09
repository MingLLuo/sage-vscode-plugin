import * as path from "node:path";

export interface RuntimeSourceRootDiscoverySnapshot {
  scopeKey: string;
  workspaceFolders: readonly string[];
  configuredSourceRoots: readonly string[];
  interpreterPath: string;
  interpreterArgs: readonly string[];
}

export type RuntimeSourceRootDiscoveryEvent =
  | { type: "no-new-roots"; reason: string; elapsedMs: number }
  | {
    type: "roots-added";
    reason: string;
    elapsedMs: number;
    additions: readonly string[];
    roots: readonly string[];
  }
  | { type: "failed"; reason: string; error: unknown };

export interface RuntimeSourceRootDiscoveryControllerOptions {
  prepare(reason: string): RuntimeSourceRootDiscoverySnapshot | undefined;
  discoverStartupRoots(
    snapshot: RuntimeSourceRootDiscoverySnapshot,
    effectiveRoots: readonly string[],
  ): readonly string[];
  discoverRuntimeRoots(
    snapshot: RuntimeSourceRootDiscoverySnapshot,
    effectiveRoots: readonly string[],
  ): Promise<readonly string[]>;
  onEvent(event: RuntimeSourceRootDiscoveryEvent): void;
  now?: () => number;
  resolvePath?: (root: string) => string;
  snapshotKey?: (snapshot: RuntimeSourceRootDiscoverySnapshot) => string;
}

interface PendingDiscovery {
  reason: string;
  snapshot: RuntimeSourceRootDiscoverySnapshot;
  key: string;
}

export class RuntimeSourceRootDiscoveryController {
  private roots: string[] = [];
  private currentOperation: Promise<void> | undefined;
  private readonly pendingDiscoveries = new Map<string, PendingDiscovery>();
  private readonly completedKeys = new Set<string>();
  private activeKey: string | undefined;
  private generation = 0;
  private deactivating = false;
  private readonly now: () => number;
  private readonly resolvePath: (root: string) => string;
  private readonly snapshotKey: (snapshot: RuntimeSourceRootDiscoverySnapshot) => string;

  constructor(private readonly options: RuntimeSourceRootDiscoveryControllerOptions) {
    this.now = options.now ?? Date.now;
    this.resolvePath = options.resolvePath ?? path.resolve;
    this.snapshotKey = options.snapshotKey ?? defaultSnapshotKey;
  }

  get discoveredRoots(): readonly string[] {
    return [...this.roots];
  }

  get operation(): Promise<void> | undefined {
    return this.currentOperation;
  }

  effectiveRoots(configuredRoots: readonly string[]): string[] {
    return [...new Set([...configuredRoots, ...this.roots])];
  }

  schedule(reason: string): Promise<void> | undefined {
    if (this.deactivating) {
      return this.currentOperation;
    }
    const snapshot = this.options.prepare(reason);
    if (!snapshot) {
      return this.currentOperation;
    }
    return this.enqueue({ reason, snapshot, key: this.snapshotKey(snapshot) }, false);
  }

  invalidateAndSchedule(reason: string): Promise<void> | undefined {
    this.roots = [];
    this.generation += 1;
    this.completedKeys.clear();
    this.pendingDiscoveries.clear();
    if (this.deactivating) {
      return this.currentOperation;
    }
    const snapshot = this.options.prepare(reason);
    if (!snapshot) {
      return this.currentOperation;
    }
    return this.enqueue({ reason, snapshot, key: this.snapshotKey(snapshot) }, true);
  }

  beginDeactivation(): Promise<void> | undefined {
    this.deactivating = true;
    this.generation += 1;
    this.pendingDiscoveries.clear();
    return this.currentOperation;
  }

  private enqueue(discovery: PendingDiscovery, alreadyInvalidated: boolean): Promise<void> | undefined {
    if (
      !alreadyInvalidated
      && (
        discovery.key === this.activeKey
        || this.pendingDiscoveries.has(discovery.key)
        || this.completedKeys.has(discovery.key)
      )
    ) {
      return this.currentOperation;
    }
    this.pendingDiscoveries.set(discovery.key, discovery);
    return this.currentOperation ?? this.startPump();
  }

  private startPump(): Promise<void> {
    const operation = this.runDiscoveryPump();
    this.currentOperation = operation;
    return operation;
  }

  private async runDiscoveryPump(): Promise<void> {
    try {
      while (!this.deactivating && this.pendingDiscoveries.size > 0) {
        const next = this.pendingDiscoveries.entries().next().value as
          | [string, PendingDiscovery]
          | undefined;
        if (!next) {
          return;
        }
        const [key, discovery] = next;
        this.pendingDiscoveries.delete(key);
        this.activeKey = discovery.key;
        const discoveryGeneration = this.generation;
        try {
          const completed = await this.runDiscovery(discovery, discoveryGeneration);
          if (completed && discoveryGeneration === this.generation) {
            this.completedKeys.add(discovery.key);
          }
        } finally {
          this.activeKey = undefined;
        }
      }
    } finally {
      this.activeKey = undefined;
      this.currentOperation = undefined;
      if (!this.deactivating && this.pendingDiscoveries.size > 0) {
        this.startPump();
      }
    }
  }

  private async runDiscovery(
    discovery: PendingDiscovery,
    discoveryGeneration: number,
  ): Promise<boolean> {
    const started = this.now();
    try {
      const effectiveRoots = this.effectiveRoots(discovery.snapshot.configuredSourceRoots);
      const startupRoots = this.options
        .discoverStartupRoots(discovery.snapshot, effectiveRoots)
        .map((root) => this.resolvePath(root));
      const discoveredRoots = await this.options.discoverRuntimeRoots(
        discovery.snapshot,
        effectiveRoots,
      );
      if (this.deactivating || discoveryGeneration !== this.generation) {
        return false;
      }
      const knownRoots = new Set(
        [...startupRoots, ...this.roots].map((root) => this.resolvePath(root)),
      );
      const additions: string[] = [];
      for (const discoveredRoot of discoveredRoots) {
        const root = this.resolvePath(discoveredRoot);
        if (knownRoots.has(root)) {
          continue;
        }
        knownRoots.add(root);
        additions.push(root);
      }
      if (additions.length === 0) {
        this.options.onEvent({
          type: "no-new-roots",
          reason: discovery.reason,
          elapsedMs: this.now() - started,
        });
        return true;
      }

      this.roots = [...new Set([...this.roots, ...additions])];
      this.options.onEvent({
        type: "roots-added",
        reason: discovery.reason,
        elapsedMs: this.now() - started,
        additions,
        roots: this.discoveredRoots,
      });
      return true;
    } catch (error) {
      if (!this.deactivating && discoveryGeneration === this.generation) {
        this.options.onEvent({ type: "failed", reason: discovery.reason, error });
      }
      return false;
    }
  }
}

function defaultSnapshotKey(snapshot: RuntimeSourceRootDiscoverySnapshot): string {
  return JSON.stringify([
    snapshot.scopeKey,
    snapshot.workspaceFolders,
    snapshot.configuredSourceRoots,
    snapshot.interpreterPath,
    snapshot.interpreterArgs,
  ]);
}
