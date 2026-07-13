import assert from "node:assert/strict";
import test from "node:test";

import { OperationTimeoutError, withOperationTimeout } from "../src/boundedOperation";

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
