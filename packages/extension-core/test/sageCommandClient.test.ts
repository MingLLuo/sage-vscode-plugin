import assert from "node:assert/strict";
import test from "node:test";

import { CancellationToken, ExecuteCommandRequest } from "vscode-languageserver-protocol";

import {
  executeSageCommand,
  type SageCommandClient,
} from "../src/sageCommandClient";

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
