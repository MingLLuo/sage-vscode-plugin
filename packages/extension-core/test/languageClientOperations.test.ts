import assert from "node:assert/strict";
import test from "node:test";

import { OperationTimeoutError } from "../src/boundedOperation";
import {
  startLanguageClientWithTimeout,
  stopLanguageClientWithTimeout,
  type StartableLanguageClient,
} from "../src/languageClientOperations";

test("startLanguageClientWithTimeout leaves a started client running", async () => {
  const client = fakeClient(async () => undefined, async () => undefined);

  await startLanguageClientWithTimeout(client, {
    startTimeoutMs: 50,
    cleanupTimeoutMs: 50,
  });

  assert.equal(client.startCalls, 1);
  assert.equal(client.stopCalls, 0);
});

test("startLanguageClientWithTimeout cleans up a failed partial start", async () => {
  const failure = new Error("handshake failed");
  const client = fakeClient(async () => { throw failure; }, async () => undefined);

  await assert.rejects(
    startLanguageClientWithTimeout(client, {
      startTimeoutMs: 50,
      cleanupTimeoutMs: 50,
    }),
    failure,
  );
  assert.equal(client.stopCalls, 1);
});

test("startLanguageClientWithTimeout bounds a hung start and attempts cleanup", async () => {
  const client = fakeClient(
    () => new Promise<void>(() => undefined),
    async () => undefined,
  );

  await assert.rejects(
    startLanguageClientWithTimeout(client, {
      startTimeoutMs: 10,
      cleanupTimeoutMs: 50,
      label: "test client start",
    }),
    (error: unknown) => error instanceof OperationTimeoutError
      && error.operation === "test client start",
  );
  assert.equal(client.stopCalls, 1);
});

test("start cleanup errors do not replace the primary start failure", async () => {
  const failure = new Error("start failed");
  const cleanupErrors: unknown[] = [];
  const client = fakeClient(
    async () => { throw failure; },
    () => new Promise<void>(() => undefined),
  );

  await assert.rejects(
    startLanguageClientWithTimeout(client, {
      startTimeoutMs: 50,
      cleanupTimeoutMs: 10,
      onCleanupError: (error) => cleanupErrors.push(error),
    }),
    failure,
  );
  assert.equal(cleanupErrors.length, 1);
  assert.ok(cleanupErrors[0] instanceof OperationTimeoutError);
});

test("stopLanguageClientWithTimeout bounds a hung stop", async () => {
  const client = fakeClient(
    async () => undefined,
    () => new Promise<void>(() => undefined),
  );

  await assert.rejects(
    stopLanguageClientWithTimeout(client, 10, "test client stop"),
    (error: unknown) => error instanceof OperationTimeoutError
      && error.operation === "test client stop",
  );
  assert.equal(client.stopCalls, 1);
});

interface FakeLanguageClient extends StartableLanguageClient {
  startCalls: number;
  stopCalls: number;
}

function fakeClient(
  start: () => PromiseLike<void>,
  stop: () => PromiseLike<void>,
): FakeLanguageClient {
  return {
    startCalls: 0,
    stopCalls: 0,
    start() {
      this.startCalls += 1;
      return start();
    },
    stop() {
      this.stopCalls += 1;
      return stop();
    },
  };
}
