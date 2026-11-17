import { LanguageServerLaunchError } from "../src/serverStartup";
import assert from "node:assert/strict";
import test from "node:test";

import {
  LanguageClientLifecycleController,
  type LanguageClientLifecycleControllerOptions,
  type LanguageClientLifecycleHooks,
} from "../src/languageClientLifecycleController";

interface FakeClient {
  id: number;
  start(): Promise<void>;
  stop(): Promise<void>;
  stopCalls: number;
}

interface Deferred<T> {
  promise: Promise<T>;
  resolve(value: T): void;
  reject(error: unknown): void;
}

interface Harness {
  controller: LanguageClientLifecycleController<FakeClient>;
  clients: FakeClient[];
  hooks: LanguageClientLifecycleHooks[];
  starts: Array<Deferred<void>>;
  stops: Array<Deferred<void>>;
  errors: unknown[];
  runtimeUnavailable: number;
  slowNotices: string[];
  events: string[];
  runTimers(): void;
  setCanRun(value: boolean): void;
  setShouldAutoStart(value: boolean): void;
}

function deferred<T>(): Deferred<T> {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

async function flushAsyncWork(): Promise<void> {
  await new Promise<void>((resolve) => setImmediate(resolve));
}

function createHarness(options: {
  immediateStart?: boolean;
  deferredStop?: boolean;
  shutdownTimeoutMs?: number;
  afterStarted?: () => Promise<void>;
  beforeStart?: (signal: AbortSignal) => Promise<void>;
} = {}): Harness {
  const clients: FakeClient[] = [];
  const hooks: LanguageClientLifecycleHooks[] = [];
  const starts: Array<Deferred<void>> = [];
  const stops: Array<Deferred<void>> = [];
  const errors: unknown[] = [];
  const slowNotices: string[] = [];
  const events: string[] = [];
  const timers = new Map<number, () => void>();
  let timerId = 0;
  let canRun = true;
  let shouldAutoStart = true;
  let runtimeUnavailable = 0;
  const lifecycleOptions: LanguageClientLifecycleControllerOptions<FakeClient> = {
    createClient(clientHooks) {
      hooks.push(clientHooks);
      const start = deferred<void>();
      starts.push(start);
      const client: FakeClient = {
        id: clients.length + 1,
        stopCalls: 0,
        start: () => options.immediateStart ? Promise.resolve() : start.promise,
        stop: async () => {
          client.stopCalls += 1;
          if (options.deferredStop) {
            const stop = deferred<void>();
            stops.push(stop);
            await stop.promise;
          }
        },
      };
      clients.push(client);
      return client;
    },
    startClient: async (client) => client.start(),
    stopClient: async (client) => {
      events.push("stop");
      await client.stop();
    },
    beforeRestart: () => { events.push("before-restart"); },
    beforeStart: options.beforeStart ?? (async () => { events.push("before-start"); }),
    canRun: () => canRun,
    shouldAutoStart: () => shouldAutoStart,
    onRuntimeUnavailable: () => { runtimeUnavailable += 1; },
    afterStarted: options.afterStarted ?? (async () => undefined),
    onStateChanged: () => undefined,
    onSlowStartNotice: (reason) => slowNotices.push(reason),
    onStartError: (error) => errors.push(error),
    logger: {
      info: () => undefined,
      warn: () => undefined,
      error: () => undefined,
    },
    shutdownTimeoutMs: options.shutdownTimeoutMs ?? 10,
    slowStartNoticeMs: 10,
    setTimeout: ((callback: () => void) => {
      const id = ++timerId;
      timers.set(id, callback);
      return id as unknown as ReturnType<typeof setTimeout>;
    }) as typeof setTimeout,
    clearTimeout: ((id: ReturnType<typeof setTimeout>) => {
      timers.delete(id as unknown as number);
    }) as typeof clearTimeout,
  };
  const controller = new LanguageClientLifecycleController(lifecycleOptions);
  return {
    controller,
    clients,
    hooks,
    starts,
    stops,
    errors,
    slowNotices,
    events,
    get runtimeUnavailable() { return runtimeUnavailable; },
    runTimers: () => {
      const callbacks = [...timers.values()];
      timers.clear();
      callbacks.forEach((callback) => callback());
    },
    setCanRun: (value) => { canRun = value; },
    setShouldAutoStart: (value) => { shouldAutoStart = value; },
  };
}

test("concurrent start requests queue exactly one follow-up restart", async () => {
  const harness = createHarness();
  const first = harness.controller.start();
  await flushAsyncWork();
  assert.equal(harness.clients.length, 1);

  const second = harness.controller.start();
  harness.starts[0].resolve();
  await flushAsyncWork();

  assert.equal(harness.clients.length, 2);
  assert.equal(harness.clients[0].stopCalls, 1);
  harness.starts[1].resolve();
  await Promise.all([first, second]);
  assert.equal(harness.controller.activeClient, harness.clients[1]);
  assert.deepEqual(harness.controller.snapshot(), {
    launchCount: 2,
    managedCloseCount: 1,
    unexpectedCloseCount: 0,
    managedShutdownActive: false,
    restartQueued: false,
    operationInFlight: false,
    hasClient: true,
    recoveryStopped: false,
    consecutiveFailures: 0,
  });
});

test("a restart requested after loop exit is pumped after operation cleanup", async () => {
  const afterStarted = deferred<void>();
  const clients: FakeClient[] = [];
  let edgeRestart: Promise<void> | undefined;
  let controller!: LanguageClientLifecycleController<FakeClient>;
  afterStarted.promise.then(() => {
    queueMicrotask(() => { edgeRestart = controller.start(); });
  });
  controller = new LanguageClientLifecycleController({
    createClient: () => {
      const client: FakeClient = {
        id: clients.length + 1,
        stopCalls: 0,
        start: async () => undefined,
        stop: async () => { client.stopCalls += 1; },
      };
      clients.push(client);
      return client;
    },
    startClient: async (client) => client.start(),
    stopClient: async (client) => client.stop(),
    beforeRestart: () => undefined,
    beforeStart: async () => undefined,
    canRun: () => true,
    shouldAutoStart: () => true,
    onRuntimeUnavailable: () => undefined,
    afterStarted: async () => {
      if (clients.length === 1) {
        await afterStarted.promise;
      }
    },
    onStateChanged: () => undefined,
    onSlowStartNotice: () => undefined,
    onStartError: (error) => { throw error; },
    logger: {
      info: () => undefined,
      warn: () => undefined,
      error: () => undefined,
    },
    shutdownTimeoutMs: 10,
    slowStartNoticeMs: 10,
  });

  const initialStart = controller.start();
  await flushAsyncWork();
  afterStarted.resolve();
  await initialStart;
  await flushAsyncWork();
  await edgeRestart;

  assert.equal(clients.length, 2);
  assert.equal(clients[0].stopCalls, 1);
  assert.equal(controller.snapshot().restartQueued, false);
  assert.equal(controller.snapshot().operationInFlight, false);
});

test("a failed start clears transient state and allows a later retry", async () => {
  const harness = createHarness();
  const failure = new Error("handshake failed");
  const first = harness.controller.start();
  await flushAsyncWork();
  harness.starts[0].reject(failure);
  await first;

  assert.deepEqual(harness.errors, [failure]);
  assert.equal(harness.controller.activeClient, undefined);
  assert.equal(harness.controller.operation, undefined);

  const retry = harness.controller.start();
  await flushAsyncWork();
  harness.starts[1].resolve();
  await retry;
  assert.equal(harness.controller.activeClient, harness.clients[1]);
});

test("an unavailable runtime prevents client construction", async () => {
  const harness = createHarness();
  harness.setCanRun(false);

  await harness.controller.start();

  assert.equal(harness.clients.length, 0);
  assert.equal(harness.runtimeUnavailable, 1);
  assert.equal(harness.controller.operation, undefined);
});

test("managed restarts and unexpected closes keep separate counters", async () => {
  const harness = createHarness({ immediateStart: true });
  await harness.controller.start();
  assert.equal(harness.hooks[0].shouldAutoRestartOnClose(), true);
  harness.hooks[0].onClose({ managedShutdown: false });

  await harness.controller.start();
  harness.hooks[0].onClose({ managedShutdown: true });

  const snapshot = harness.controller.snapshot();
  assert.equal(snapshot.launchCount, 2);
  assert.equal(snapshot.managedCloseCount, 0);
  assert.equal(snapshot.unexpectedCloseCount, 1);
  assert.equal(harness.hooks[0].shouldAutoRestartOnClose(), false);
});

test("repeated crashes stop recovery until manual restart", async () => {
  const harness = createHarness({ immediateStart: true });
  await harness.controller.start();
  for (let attempt = 0; attempt < 4; attempt += 1) {
    harness.hooks[attempt].onClose({ managedShutdown: false });
    assert.equal(harness.controller.activeClient, undefined);
    harness.runTimers();
    await flushAsyncWork();
    assert.equal(harness.clients.length, attempt + 2);
  }
  harness.hooks[4].onClose({ managedShutdown: false });
  assert.equal(harness.controller.snapshot().recoveryStopped, true);
  harness.runTimers();
  await flushAsyncWork();
  assert.equal(harness.clients.length, 5);
  await harness.controller.manualRestart();
  assert.equal(harness.clients.length, 6);
  assert.equal(harness.controller.snapshot().recoveryStopped, false);
  assert.equal(harness.controller.snapshot().consecutiveFailures, 0);
});

test("deactivation cancels a pending crash recovery", async () => {
  const harness = createHarness({ immediateStart: true });
  await harness.controller.start();
  harness.hooks[0].onClose({ managedShutdown: false });
  await harness.controller.deactivate();
  harness.runTimers();
  await flushAsyncWork();
  assert.equal(harness.clients.length, 1);
});

test("a restart clears status refresh before stopping the active client", async () => {
  const harness = createHarness({ immediateStart: true });
  await harness.controller.start();
  harness.events.length = 0;

  await harness.controller.start();

  assert.deepEqual(harness.events.slice(0, 3), ["before-restart", "stop", "before-start"]);
});

test("background auto-start honors policy and slow-start notice is shown once", async () => {
  const harness = createHarness();
  harness.setShouldAutoStart(false);
  harness.controller.startInBackground("ordinary-python");
  harness.runTimers();
  assert.equal(harness.clients.length, 0);
  assert.deepEqual(harness.slowNotices, []);

  harness.controller.startInBackground("forced", true);
  await flushAsyncWork();
  harness.runTimers();
  harness.runTimers();
  assert.deepEqual(harness.slowNotices, ["forced"]);

  harness.starts[0].resolve();
  await harness.controller.operation;
  harness.controller.startInBackground("later", true);
  await flushAsyncWork();
  harness.runTimers();
  harness.starts[1].resolve();
  await harness.controller.operation;
  assert.deepEqual(harness.slowNotices, ["forced"]);
});

test("deactivation takes ownership of a pending start and blocks later starts", async () => {
  const harness = createHarness({ shutdownTimeoutMs: 5 });
  harness.controller.startInBackground("activation", true);
  await flushAsyncWork();
  assert.equal(harness.clients.length, 1);

  await harness.controller.deactivate();
  assert.equal(harness.clients[0].stopCalls, 1);
  harness.runTimers();
  assert.deepEqual(harness.slowNotices, []);

  harness.starts[0].resolve();
  await harness.controller.operation;
  await harness.controller.start();
  assert.equal(harness.clients.length, 1);
  assert.equal(harness.controller.activeClient, undefined);
});

test("startup and deactivation cannot stop the same pending client twice", async () => {
  const harness = createHarness({ deferredStop: true, shutdownTimeoutMs: 5 });
  harness.controller.startInBackground("activation", true);
  await flushAsyncWork();

  const shutdown = harness.controller.deactivate();
  harness.starts[0].resolve();
  await flushAsyncWork();
  await shutdown;

  assert.equal(harness.clients[0].stopCalls, 1);
  assert.equal(harness.controller.snapshot().managedCloseCount, 1);

  harness.stops[0].resolve();
  await harness.controller.operation;
});

test("a post-start hook failure stops the client before scheduling recovery", async () => {
  const failure = new Error("status request failed");
  const harness = createHarness({ immediateStart: true, afterStarted: async () => { throw failure; } });
  await harness.controller.start();
  assert.equal(harness.clients[0].stopCalls, 1);
  assert.equal(harness.controller.activeClient, undefined);
  assert.deepEqual(harness.errors, [failure]);
  await harness.controller.deactivate();
});


test("permanent launch failures stop recovery until a manual retry", async () => {
  let fail = true;
  const harness = createHarness({ immediateStart: true, beforeStart: async () => {
    if (fail) throw new LanguageServerLaunchError("Executable not found", "/missing/sage-ls");
  } });
  await harness.controller.start();
  assert.equal(harness.controller.hasStoppedRecovery, true);
  assert.equal(harness.clients.length, 0);
  await harness.controller.start();
  assert.equal(harness.errors.length, 1);
  fail = false;
  await harness.controller.manualRestart();
  assert.equal(harness.controller.hasStoppedRecovery, false);
  assert.equal(harness.clients.length, 1);
  await harness.controller.deactivate();
});

test("a restart requested during a failing startup probe is not discarded", async () => {
  // The user fixes the server path while the old path is still being probed.
  let release: ((error: Error) => void) | undefined;
  let attempts = 0;
  const harness = createHarness({ immediateStart: true, beforeStart: () => {
    attempts += 1;
    if (attempts > 1) return Promise.resolve();
    return new Promise<void>((_resolve, reject) => { release = reject; });
  } });
  const start = harness.controller.start();
  await flushAsyncWork();
  const restart = harness.controller.manualRestart();
  release?.(new LanguageServerLaunchError("Executable not found", "/missing/sage-ls"));
  await Promise.all([start, restart]);
  assert.equal(harness.controller.hasStoppedRecovery, false);
  assert.equal(harness.errors.length, 0, "the stale failure must not be reported");
  assert.equal(harness.clients.length, 1, "the corrected configuration starts a client");
  await harness.controller.deactivate();
});

test("deactivation aborts a pending startup probe", async () => {
  let aborted = false;
  const harness = createHarness({ beforeStart: (signal) => new Promise<void>((_resolve, reject) => {
    signal.addEventListener("abort", () => { aborted = true; reject(new Error("cancelled")); }, { once: true });
  }) });
  const start = harness.controller.start();
  await flushAsyncWork();
  await harness.controller.deactivate();
  await start;
  assert.equal(aborted, true);
  assert.equal(harness.clients.length, 0);
  assert.equal(harness.errors.length, 0);
});
