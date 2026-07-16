import assert from "node:assert/strict";
import test from "node:test";

import {
  OperationTimeoutError,
  waitForOperationOrCancellation,
  withOperationTimeout,
} from "../src/boundedOperation";

test("withOperationTimeout returns a completed operation", async () => {
  assert.equal(await withOperationTimeout(Promise.resolve("ready"), 100, "startup"), "ready");
});

test("withOperationTimeout bounds a hung operation and runs cancellation", async () => {
  let cancellations = 0;
  const hung = new Promise<never>(() => undefined);
  const started = Date.now();

  await assert.rejects(
    withOperationTimeout(hung, 15, "language server status", () => { cancellations += 1; }),
    (error: unknown) => {
      assert.ok(error instanceof OperationTimeoutError);
      assert.equal(error.operation, "language server status");
      assert.equal(error.timeoutMs, 15);
      return true;
    },
  );
  assert.equal(cancellations, 1);
  assert.ok(Date.now() - started < 1_000, "the hung operation should be released promptly");
});

test("withOperationTimeout validates timeout values", async () => {
  await assert.rejects(
    withOperationTimeout(Promise.resolve(), Number.POSITIVE_INFINITY, "invalid"),
    /non-negative finite timeout/,
  );
  await assert.rejects(
    withOperationTimeout(Promise.resolve(), -1, "invalid"),
    /non-negative finite timeout/,
  );
});

test("waitForOperationOrCancellation completes work and releases its listener", async () => {
  const token = fakeCancellationToken();

  assert.equal(
    await waitForOperationOrCancellation(Promise.resolve(), token),
    "completed",
  );
  assert.equal(token.disposeCalls, 1);
});

test("waitForOperationOrCancellation lets a user stop waiting for shared work", async () => {
  const token = fakeCancellationToken();
  let rejectOperation: ((error: Error) => void) | undefined;
  const operation = new Promise<void>((_resolve, reject) => {
    rejectOperation = reject;
  });
  const waiting = waitForOperationOrCancellation(operation, token);

  token.cancel();
  assert.equal(await waiting, "cancelled");
  assert.equal(token.disposeCalls, 1);

  rejectOperation?.(new Error("late failure"));
  await Promise.resolve();
});

test("waitForOperationOrCancellation observes a token cancelled before subscription", async () => {
  const token = fakeCancellationToken();
  token.cancel();
  let rejectOperation: ((error: Error) => void) | undefined;
  const operation = new Promise<void>((_resolve, reject) => {
    rejectOperation = reject;
  });

  assert.equal(await waitForOperationOrCancellation(operation, token), "cancelled");
  assert.equal(token.disposeCalls, 1);

  rejectOperation?.(new Error("late failure after early cancellation"));
  await Promise.resolve();
});

function fakeCancellationToken() {
  let listener: (() => void) | undefined;
  return {
    isCancellationRequested: false,
    disposeCalls: 0,
    onCancellationRequested(next: () => void) {
      listener = next;
      return {
        dispose: () => {
          this.disposeCalls += 1;
          listener = undefined;
        },
      };
    },
    cancel() {
      this.isCancellationRequested = true;
      listener?.();
    },
  };
}
