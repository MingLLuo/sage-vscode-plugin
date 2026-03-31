import test from "node:test";
import assert from "node:assert/strict";
import os from "node:os";
import path from "node:path";
import fs from "node:fs";

import {
  buildWorkspaceInitializationData,
  discoverSourceRoots,
} from "../src/workspaceDiscovery";

test("discoverSourceRoots prefers configured roots when provided", () => {
  assert.deepEqual(discoverSourceRoots(["/workspace"], ["/configured/src"]), ["/configured/src"]);
});

test("discoverSourceRoots maps a Sage source checkout to its src root", () => {
  const tmpRoot = fs.mkdtempSync(path.join(os.tmpdir(), "sage-vscode-workspace-"));
  fs.mkdirSync(path.join(tmpRoot, "src", "sage"), { recursive: true });

  assert.deepEqual(discoverSourceRoots([tmpRoot], []), [path.join(tmpRoot, "src")]);
});

test("buildWorkspaceInitializationData emits folder and source-root URIs", () => {
  const tmpRoot = fs.mkdtempSync(path.join(os.tmpdir(), "sage-vscode-init-"));
  fs.mkdirSync(path.join(tmpRoot, "src", "sage"), { recursive: true });

  const result = buildWorkspaceInitializationData([tmpRoot], []);
  assert.equal(result.folders.length, 1);
  assert.equal(result.sourceRoots.length, 1);
  assert.ok(result.rootUri?.startsWith("file://"));
  assert.ok(result.sourceRoots[0]?.endsWith("/src"));
});
