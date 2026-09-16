import assert from "node:assert/strict";
import test from "node:test";

import {
  RuntimeSourceRootDiscoveryController,
  type RuntimeSourceRootDiscoveryEvent,
  type RuntimeSourceRootDiscoverySnapshot,
} from "../src/runtimeSourceRootDiscoveryController";

interface Deferred<T> {
  promise: Promise<T>;
  resolve(value: T): void;
  reject(error: unknown): void;
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

function createHarness() {
  const discoveries: Array<Deferred<readonly string[]>> = [];
  const events: RuntimeSourceRootDiscoveryEvent[] = [];
  const reasons: string[] = [];
  let startupRoots: readonly string[] = ["/sage/src"];
  let startupError: unknown;
  let prepared = true;
  let snapshot: RuntimeSourceRootDiscoverySnapshot = {
    scopeKey: "file:///workspace",
    workspaceFolders: ["/workspace"],
    configuredSourceRoots: ["/configured"],
    interpreterPath: "/sage/sage",
    interpreterArgs: [],
  };
  let now = 100;
  const controller = new RuntimeSourceRootDiscoveryController({
    prepare: (reason) => {
      reasons.push(reason);
      return prepared ? snapshot : undefined;
    },
    discoverStartupRoots: () => {
      if (startupError) {
        throw startupError;
      }
      return startupRoots;
    },
    discoverRuntimeRoots: async () => {
      const discovery = deferred<readonly string[]>();
      discoveries.push(discovery);
      return discovery.promise;
    },
    onEvent: (event) => events.push(event),
    now: () => now,
    resolvePath: (root) => root.replace(/\/$/, ""),
  });
  return {
    controller,
    discoveries,
    events,
    reasons,
    advance: (milliseconds: number) => { now += milliseconds; },
    setPrepared: (value: boolean) => { prepared = value; },
    setSnapshot: (value: RuntimeSourceRootDiscoverySnapshot) => { snapshot = value; },
    setStartupRoots: (roots: readonly string[]) => { startupRoots = roots; },
    setStartupError: (error: unknown) => { startupError = error; },
  };
}

test("runtime discovery publishes new roots once and preserves root order", async () => {
  const harness = createHarness();
  const operation = harness.controller.schedule("activation");
  harness.advance(25);
  harness.discoveries[0].resolve(["/runtime", "/runtime/"]);
  await operation;

  assert.deepEqual(harness.controller.discoveredRoots, ["/runtime"]);
  assert.deepEqual(harness.controller.effectiveRoots(["/configured", "/runtime"]), [
    "/configured",
    "/runtime",
  ]);
  assert.deepEqual(harness.events, [{
    type: "roots-added",
    reason: "activation",
    elapsedMs: 25,
    additions: ["/runtime"],
    roots: ["/runtime"],
  }]);
});

test("startup and existing roots are not republished", async () => {
  const harness = createHarness();
  const first = harness.controller.schedule("first");
  harness.discoveries[0].resolve(["/runtime"]);
  await first;

  harness.events.length = 0;
  harness.setStartupRoots(["/sage/src", "/startup"]);
  harness.setSnapshot({
    scopeKey: "file:///workspace",
    workspaceFolders: ["/workspace"],
    configuredSourceRoots: ["/configured"],
    interpreterPath: "/sage/sage",
    interpreterArgs: ["--new-environment"],
  });
  const second = harness.controller.schedule("second");
  harness.discoveries[1].resolve(["/runtime", "/startup", "/sage/src"]);
  await second;
  assert.deepEqual(harness.events, [{ type: "no-new-roots", reason: "second", elapsedMs: 0 }]);
});

test("ordinary schedule calls do not start parallel discoveries", async () => {
  const harness = createHarness();
  const first = harness.controller.schedule("first");
  const duplicate = harness.controller.schedule("duplicate");

  assert.equal(duplicate, first);
  assert.equal(harness.discoveries.length, 1);
  harness.discoveries[0].resolve([]);
  await first;
  assert.deepEqual(harness.reasons, ["first", "duplicate"]);
  assert.equal(harness.controller.schedule("same-input-after-completion"), undefined);
  assert.equal(harness.discoveries.length, 1);
});

test("different workspace snapshots are serialized and each discovered once", async () => {
  const harness = createHarness();
  const first = harness.controller.schedule("workspace-a");
  harness.setSnapshot({
    scopeKey: "file:///workspace-b",
    workspaceFolders: ["/workspace-a", "/workspace-b"],
    configuredSourceRoots: ["/configured-b"],
    interpreterPath: "/sage-b/sage",
    interpreterArgs: ["--isolated"],
  });
  assert.equal(harness.controller.schedule("workspace-b"), first);
  assert.equal(harness.controller.schedule("workspace-b-duplicate"), first);

  harness.discoveries[0].resolve(["/runtime-a"]);
  await flushAsyncWork();
  assert.equal(harness.discoveries.length, 2);
  assert.deepEqual(harness.controller.discoveredRoots, ["/runtime-a"]);

  harness.discoveries[1].resolve(["/runtime-b"]);
  await first;
  assert.deepEqual(harness.controller.discoveredRoots, ["/runtime-a", "/runtime-b"]);
  assert.deepEqual(harness.events.map((event) => event.type), ["roots-added", "roots-added"]);
  assert.deepEqual(harness.reasons, ["workspace-a", "workspace-b", "workspace-b-duplicate"]);
});

test("invalidation rejects stale results and schedules one replacement", async () => {
  const harness = createHarness();
  const first = harness.controller.schedule("first");
  harness.controller.invalidateAndSchedule("configuration-change");
  harness.controller.invalidateAndSchedule("workspace-change");
  harness.discoveries[0].resolve(["/stale"]);
  await flushAsyncWork();

  assert.equal(harness.discoveries.length, 2);
  assert.deepEqual(harness.controller.discoveredRoots, []);
  harness.discoveries[1].resolve(["/fresh"]);
  await first;

  assert.deepEqual(harness.controller.discoveredRoots, ["/fresh"]);
  assert.deepEqual(harness.reasons, ["first", "configuration-change", "workspace-change"]);
  assert.deepEqual(harness.events.map((event) => event.type), ["roots-added"]);
});

test("a microtask invalidation joins the same drain operation", async () => {
  const harness = createHarness();
  const first = harness.controller.schedule("first");
  let replacement: Promise<void> | undefined;
  harness.discoveries[0].promise.then(() => {
    queueMicrotask(() => {
      replacement = harness.controller.invalidateAndSchedule("microtask-invalidation");
    });
  });

  harness.discoveries[0].resolve([]);
  await flushAsyncWork();
  assert.equal(replacement, first);
  assert.equal(harness.discoveries.length, 2);

  harness.discoveries[1].resolve(["/replacement"]);
  await replacement;
  assert.equal(harness.controller.operation, undefined);
  assert.deepEqual(harness.controller.discoveredRoots, ["/replacement"]);
});

test("a failed discovery releases the operation and can be retried", async () => {
  const harness = createHarness();
  const first = harness.controller.schedule("first");
  const failure = new Error("probe failed");
  harness.discoveries[0].reject(failure);
  await first;
  assert.equal(harness.controller.operation, undefined);
  assert.deepEqual(harness.events, [{ type: "failed", reason: "first", error: failure }]);

  const retry = harness.controller.schedule("retry");
  harness.discoveries[1].resolve(["/recovered"]);
  await retry;
  assert.deepEqual(harness.controller.discoveredRoots, ["/recovered"]);
});

test("a startup-root failure clears the active key and permits the same input to retry", async () => {
  const harness = createHarness();
  const failure = new Error("startup discovery failed");
  harness.setStartupError(failure);
  await harness.controller.schedule("first");
  assert.deepEqual(harness.events, [{ type: "failed", reason: "first", error: failure }]);

  harness.setStartupError(undefined);
  const retry = harness.controller.schedule("retry");
  assert.equal(harness.discoveries.length, 1);
  harness.discoveries[0].resolve(["/recovered"]);
  await retry;
  assert.deepEqual(harness.controller.discoveredRoots, ["/recovered"]);
});

test("deactivation suppresses in-flight results and successor discovery", async () => {
  const harness = createHarness();
  const first = harness.controller.schedule("first");
  const pending = harness.controller.beginDeactivation();
  harness.discoveries[0].resolve(["/late"]);
  await Promise.all([first, pending]);

  assert.deepEqual(harness.controller.discoveredRoots, []);
  assert.deepEqual(harness.events, []);
  assert.equal(harness.controller.schedule("after-deactivate"), undefined);
  assert.equal(harness.discoveries.length, 1);
});

test("prepare can skip discovery without allocating an operation", () => {
  const harness = createHarness();
  harness.setPrepared(false);
  assert.equal(harness.controller.schedule("not-eligible"), undefined);
  assert.equal(harness.controller.operation, undefined);
  assert.equal(harness.discoveries.length, 0);
});


test("cached Sage roots are available before the runtime probe finishes and new roots are saved", async () => {
  const probe = deferred<readonly string[]>();
  const saved: string[][] = [];
  const snapshot: RuntimeSourceRootDiscoverySnapshot = {
    scopeKey: "workspace", workspaceFolders: ["/workspace"], configuredSourceRoots: [],
    interpreterPath: "sage", interpreterArgs: [],
  };
  const controller = new RuntimeSourceRootDiscoveryController({
    prepare: () => snapshot,
    loadCachedRoots: () => ["/cached/sage/src"],
    saveCachedRoots: async (_snapshot, roots) => { saved.push([...roots]); },
    discoverStartupRoots: () => ["/workspace"],
    discoverRuntimeRoots: () => probe.promise,
    onEvent: () => {},
  });
  const operation = controller.schedule("activation");
  assert.deepEqual(controller.effectiveRoots([]), ["/cached/sage/src"]);
  probe.resolve(["/cached/sage/src", "/new/sage/src"]);
  await operation;
  assert.deepEqual(saved, [["/cached/sage/src", "/new/sage/src"]]);
  assert.deepEqual(controller.discoveredRoots, ["/cached/sage/src", "/new/sage/src"]);
});
