import test from "node:test";
import assert from "node:assert/strict";

import {
  formatWorkspaceRuntimeMode,
  formatWorkspaceRuntimeUnavailableMessage,
  isWorkspaceRuntimeAvailable,
} from "../src/workspaceTrust";

test("workspace runtime is available only for trusted local workspaces", () => {
  assert.equal(isWorkspaceRuntimeAvailable({ trusted: true, hasVirtualWorkspace: false }), true);
  assert.equal(isWorkspaceRuntimeAvailable({ trusted: false, hasVirtualWorkspace: false }), false);
  assert.equal(isWorkspaceRuntimeAvailable({ trusted: true, hasVirtualWorkspace: true }), false);
});

test("workspace runtime labels explain restricted and virtual modes", () => {
  assert.equal(formatWorkspaceRuntimeMode({ trusted: true, hasVirtualWorkspace: false }), "trusted local workspace");
  assert.equal(formatWorkspaceRuntimeMode({ trusted: false, hasVirtualWorkspace: false }), "restricted workspace");
  assert.equal(formatWorkspaceRuntimeMode({ trusted: true, hasVirtualWorkspace: true }), "virtual workspace");
});

test("workspace runtime warning messages name the unsafe capability", () => {
  assert.match(
    formatWorkspaceRuntimeUnavailableMessage({ trusted: false, hasVirtualWorkspace: false }, "Running a Sage file"),
    /starts local processes and can execute workspace code/,
  );
  assert.match(
    formatWorkspaceRuntimeUnavailableMessage({ trusted: true, hasVirtualWorkspace: true }, "Rebuilding the Sage index"),
    /indexes files from disk/,
  );
});
