import assert from "node:assert/strict";
import test from "node:test";

import { CancellationToken, ExecuteCommandRequest } from "vscode-languageserver-protocol";

import {
  executeSageCommand,
  executeSageCommandWithTimeout,
  type SageCommandClient,
} from "../src/sageCommandClient";
import { OperationTimeoutError } from "../src/boundedOperation";

test("executeSageCommand uses the typed by-name request and preserves cancellation", async () => {
  const calls: Array<{ type: unknown; params: unknown; token: unknown }> = [];
  const client: SageCommandClient = {
    sendRequest: async (type, params, token) => {
      calls.push({ type, params, token });
      return { generation: 7 };
    },
  };

  const result = await executeSageCommand<{ generation: number }>(
    client,
    "sage.__rust.indexStatus",
    [{ detail: true }],
    CancellationToken.None,
  );

  assert.deepEqual(result, { generation: 7 });
  assert.equal(calls.length, 1);
  assert.equal(calls[0]?.type, ExecuteCommandRequest.type);
  assert.deepEqual(calls[0]?.params, {
    command: "sage.__rust.indexStatus",
    arguments: [{ detail: true }],
  });
  assert.equal(calls[0]?.token, CancellationToken.None);
  assert.equal(ExecuteCommandRequest.type.method, "workspace/executeCommand");
});

test("executeSageCommandWithTimeout releases a hung user command", async () => {
  let timedOut = false;
  const client: SageCommandClient = {
    sendRequest: () => new Promise<unknown>(() => undefined),
  };

  await assert.rejects(
    executeSageCommandWithTimeout(client, "sage.__rust.docsStatus", [], {
      timeoutMs: 10,
      label: "Sage docs status",
      onTimeout: () => { timedOut = true; },
    }),
    (error: unknown) => error instanceof OperationTimeoutError
      && error.operation === "Sage docs status",
  );
  assert.equal(timedOut, true);
});
