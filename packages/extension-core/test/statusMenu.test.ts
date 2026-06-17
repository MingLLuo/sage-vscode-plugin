import test from "node:test";
import assert from "node:assert/strict";

import {
  STATUS_MENU_COMMAND,
  statusMenuActions,
} from "../src/statusMenu";

test("statusMenuActions expose the core troubleshooting commands", () => {
  assert.equal(STATUS_MENU_COMMAND, "sage.__internal.showStatusMenu");
  const commands = statusMenuActions().map((action) => action.command);
  assert.deepEqual(commands, [
    "sage.showEnvironmentDetails",
    "sage.showIndexStatus",
    "sage.showDocsStatus",
    "sage.runUxSelfCheck",
    "sage.rebuildIndex",
    "sage.copySupportBundle",
  ]);
});

test("statusMenuActions include searchable descriptions and details", () => {
  for (const action of statusMenuActions()) {
    assert.match(action.label, /^\$\([a-z-]+\) /);
    assert.ok(action.description.length > 0);
    assert.ok(action.detail.length > 0);
  }
});
