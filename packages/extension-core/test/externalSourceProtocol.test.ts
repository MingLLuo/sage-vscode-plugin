import assert from "node:assert/strict";
import test from "node:test";

import { protocolItemWithUri } from "../src/externalSourceProtocol";

test("protocolItemWithUri changes only the transport URI and preserves hierarchy data", () => {
  const item = {
    name: "caller",
    uri: "sage-source:/external/caller.py",
    range: { start: { line: 1, character: 0 }, end: { line: 3, character: 0 } },
    data: { serverIdentity: "caller:1" },
  };

  const rewritten = protocolItemWithUri(item, "file:///external/caller.py");

  assert.notEqual(rewritten, item);
  assert.equal(rewritten.uri, "file:///external/caller.py");
  assert.equal(item.uri, "sage-source:/external/caller.py");
  assert.deepEqual(rewritten.data, item.data);
  assert.deepEqual(rewritten.range, item.range);
});

test("protocolItemWithUri keeps an already-routable hierarchy item intact", () => {
  const item = { uri: "file:///workspace/caller.sage", data: { id: 7 } };
  assert.equal(protocolItemWithUri(item, item.uri), item);
});
