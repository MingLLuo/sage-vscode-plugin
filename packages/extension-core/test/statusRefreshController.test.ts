import assert from "node:assert/strict";
import test from "node:test";

import { LanguageServerStatusRefreshController } from "../src/statusRefreshController";

interface FakeInterval {
  run(): void;
  clearCount(): number;
  setInterval: typeof setInterval;
  clearInterval: typeof clearInterval;
}

function fakeInterval(): FakeInterval {
  let callback: (() => void) | undefined;
  let clears = 0;
  return {
    run: () => callback?.(),
    clearCount: () => clears,
    setInterval: ((handler: (...args: unknown[]) => void) => {
      callback = () => handler();
      return 1 as unknown as ReturnType<typeof setInterval>;
    }) as typeof setInterval,
    clearInterval: (() => {
      clears += 1;
      callback = undefined;
    }) as typeof clearInterval,
  };
}

async function flushAsyncWork(): Promise<void> {
  await new Promise<void>((resolve) => setImmediate(resolve));
}

test("status refresh keeps polling pending work and logs at the configured cadence", async () => {
  const interval = fakeInterval();
  let refreshes = 0;
  const logs: number[] = [];
  const controller = new LanguageServerStatusRefreshController({
    intervalMs: 10,
    logEvery: 2,
    refresh: async () => { refreshes += 1; },
    snapshot: () => ({ pendingJobs: 1, pendingTask: "cache-check" }),
    shouldContinue: () => true,
    logPending: (attempts) => logs.push(attempts),
    setInterval: interval.setInterval,
    clearInterval: interval.clearInterval,
  });

  controller.schedule();
  interval.run();
  await flushAsyncWork();
  interval.run();
  await flushAsyncWork();

  assert.equal(refreshes, 2);
  assert.deepEqual(logs, [2]);
  assert.equal(interval.clearCount(), 0);
  controller.dispose();
  assert.equal(interval.clearCount(), 1);
});

test("status refresh suppresses overlapping calls and stops when the server becomes idle", async () => {
  const interval = fakeInterval();
  let pendingJobs = 1;
  let releaseRefresh: (() => void) | undefined;
  let refreshes = 0;
  const controller = new LanguageServerStatusRefreshController({
    intervalMs: 10,
    logEvery: 10,
    refresh: () => {
      refreshes += 1;
      return new Promise<void>((resolve) => { releaseRefresh = resolve; });
    },
    snapshot: () => ({ pendingJobs }),
    shouldContinue: () => true,
    logPending: () => undefined,
    setInterval: interval.setInterval,
    clearInterval: interval.clearInterval,
  });

  controller.schedule();
  interval.run();
  interval.run();
  assert.equal(refreshes, 1);

  pendingJobs = 0;
  releaseRefresh?.();
  await flushAsyncWork();
  assert.equal(interval.clearCount(), 1);
});

test("a superseded in-flight refresh cannot clear the replacement schedule", async () => {
  const interval = fakeInterval();
  let releaseFirstRefresh: (() => void) | undefined;
  let refreshes = 0;
  let pendingJobs = 1;
  const controller = new LanguageServerStatusRefreshController({
    intervalMs: 10,
    logEvery: 10,
    refresh: () => {
      refreshes += 1;
      if (refreshes === 1) {
        return new Promise<void>((resolve) => { releaseFirstRefresh = resolve; });
      }
      return Promise.resolve();
    },
    snapshot: () => ({ pendingJobs }),
    shouldContinue: () => true,
    logPending: () => undefined,
    setInterval: interval.setInterval,
    clearInterval: interval.clearInterval,
  });

  controller.schedule();
  interval.run();
  controller.schedule();
  assert.equal(interval.clearCount(), 1);

  releaseFirstRefresh?.();
  await flushAsyncWork();
  assert.equal(interval.clearCount(), 1, "the old refresh must leave the new timer installed");

  pendingJobs = 0;
  interval.run();
  await flushAsyncWork();
  assert.equal(refreshes, 2);
  assert.equal(interval.clearCount(), 2);
});

test("status refresh does not schedule while the extension is shutting down", () => {
  const interval = fakeInterval();
  const controller = new LanguageServerStatusRefreshController({
    intervalMs: 10,
    logEvery: 10,
    refresh: async () => undefined,
    snapshot: () => ({ pendingJobs: 1 }),
    shouldContinue: () => false,
    logPending: () => undefined,
    setInterval: interval.setInterval,
    clearInterval: interval.clearInterval,
  });

  controller.schedule();
  interval.run();
  assert.equal(interval.clearCount(), 0);
});
