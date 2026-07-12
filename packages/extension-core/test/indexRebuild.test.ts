import test from "node:test";
import assert from "node:assert/strict";

import {
  isIndexRebuildComplete,
  waitForIndexRebuild,
} from "../src/indexRebuild";
import type { IndexStatusSummary } from "../src/environmentPresentation";

test("isIndexRebuildComplete requires an idle generation newer than the baseline", () => {
  assert.equal(isIndexRebuildComplete({ generation: 8, pending_jobs: 0, last_operation: "rebuild" }, 7), true);
  assert.equal(isIndexRebuildComplete({ generation: 7, pending_jobs: 0, last_operation: "rebuild" }, 7), false);
  assert.equal(isIndexRebuildComplete({ generation: 8, pending_jobs: 1, last_operation: "rebuild" }, 7), false);
  assert.equal(isIndexRebuildComplete({ generation: 8, pending_jobs: 0, last_operation: "refresh" }, 7), false);
  assert.equal(isIndexRebuildComplete({ generation: 8, pending_jobs: 0 }, 7), false);
});

test("waitForIndexRebuild ignores an idle stale snapshot and waits for completed new work", async () => {
  const statuses: IndexStatusSummary[] = [
    { generation: 4, pending_jobs: 0 },
    { generation: 4, pending_jobs: 1, pending_task: "rebuild" },
    { generation: 5, pending_jobs: 1, pending_task: "rebuild" },
    { generation: 5, pending_jobs: 0, last_operation: "rebuild" },
  ];
  const observed: IndexStatusSummary[] = [];

  const result = await waitForIndexRebuild({
    baselineGeneration: 4,
    readStatus: async () => statuses.shift(),
    onStatus: (status) => observed.push(status),
    timeoutMs: 1_000,
    pollIntervalMs: 0,
    sleep: async () => undefined,
  });

  assert.equal(result.generation, 5);
  assert.equal(result.pending_jobs, 0);
  assert.equal(observed.length, 4);
});

test("waitForIndexRebuild reschedules when refresh supersedes the requested rebuild", async () => {
  const statuses: IndexStatusSummary[] = [
    { generation: 5, pending_jobs: 0, last_operation: "refresh" },
    { generation: 5, pending_jobs: 1, pending_task: "rebuild" },
    { generation: 6, pending_jobs: 0, last_operation: "rebuild" },
  ];
  let reschedules = 0;

  const result = await waitForIndexRebuild({
    baselineGeneration: 4,
    readStatus: async () => statuses.shift(),
    reschedule: async () => {
      reschedules += 1;
    },
    timeoutMs: 1_000,
    pollIntervalMs: 0,
    sleep: async () => undefined,
  });

  assert.equal(reschedules, 1);
  assert.equal(result.generation, 6);
  assert.equal(result.last_operation, "rebuild");
});

test("waitForIndexRebuild applies a real timeout even when a status request hangs", async () => {
  await assert.rejects(
    waitForIndexRebuild({
      baselineGeneration: 9,
      readStatus: async () => new Promise<IndexStatusSummary>(() => undefined),
      timeoutMs: 10,
      pollIntervalMs: 1,
    }),
    /baseline generation 9/,
  );
});
