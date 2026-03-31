import test from "node:test";
import assert from "node:assert/strict";
import os from "node:os";
import path from "node:path";
import fs from "node:fs";

import {
  buildWorkspaceInitializationData,
  discoverInterpreterSourceRoots,
  discoverSourceRoots,
  resolveConfiguredPaths,
} from "../src/workspaceDiscovery";

test("discoverSourceRoots prefers configured roots when provided", () => {
  assert.deepEqual(discoverSourceRoots(["/workspace"], ["/configured/src"]), ["/configured/src"]);
});

test("resolveConfiguredPaths resolves relative paths against workspace folders", () => {
  assert.deepEqual(
    resolveConfiguredPaths(["/workspace-a", "/workspace-b"], ["src"]),
    ["/workspace-a/src", "/workspace-b/src"],
  );
});

test("discoverSourceRoots maps a Sage source checkout to its src root", () => {
  const tmpRoot = fs.mkdtempSync(path.join(os.tmpdir(), "sage-vscode-workspace-"));
  fs.mkdirSync(path.join(tmpRoot, "src", "sage"), { recursive: true });

  assert.deepEqual(discoverSourceRoots([tmpRoot], []), [path.join(tmpRoot, "src")]);
});

test("discoverInterpreterSourceRoots finds a sibling src tree for local Sage checkouts", () => {
  const tmpRoot = fs.mkdtempSync(path.join(os.tmpdir(), "sage-vscode-interpreter-"));
  const interpreterPath = path.join(tmpRoot, "sage");
  fs.mkdirSync(path.join(tmpRoot, "src", "sage"), { recursive: true });
  fs.writeFileSync(interpreterPath, "#!/bin/sh\nexit 0\n", { encoding: "utf-8" });

  assert.deepEqual(discoverInterpreterSourceRoots(interpreterPath, []), [path.join(tmpRoot, "src")]);
});

test("discoverInterpreterSourceRoots finds site-packages roots near Sage installations", () => {
  const tmpRoot = fs.mkdtempSync(path.join(os.tmpdir(), "sage-vscode-site-packages-"));
  const interpreterPath = path.join(tmpRoot, "bin", "sage");
  const sitePackagesRoot = path.join(tmpRoot, "local", "lib", "python3.12", "site-packages");
  fs.mkdirSync(path.dirname(interpreterPath), { recursive: true });
  fs.mkdirSync(path.join(sitePackagesRoot, "sage"), { recursive: true });
  fs.writeFileSync(interpreterPath, "#!/bin/sh\nexit 0\n", { encoding: "utf-8" });

  assert.deepEqual(discoverInterpreterSourceRoots(interpreterPath, []), [sitePackagesRoot]);
});

test("discoverInterpreterSourceRoots falls back to runtime probing for command-based interpreters", () => {
  assert.deepEqual(
    discoverInterpreterSourceRoots("sage", [], {
      runtimeProbe: () => ["/runtime/src"],
      exists: (candidate) => candidate === "/runtime/src/sage",
    }),
    ["/runtime/src"],
  );
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

test("buildWorkspaceInitializationData includes discovered interpreter roots when no source roots are configured", () => {
  const tmpRoot = fs.mkdtempSync(path.join(os.tmpdir(), "sage-vscode-runtime-init-"));
  const workspaceRoot = path.join(tmpRoot, "workspace");
  fs.mkdirSync(workspaceRoot, { recursive: true });

  const result = buildWorkspaceInitializationData([workspaceRoot], [], {
    interpreterPath: "sage",
    runtimeProbe: () => ["/runtime/src"],
    exists: (candidate) => candidate === workspaceRoot || candidate === "/runtime/src/sage",
  });

  assert.equal(result.sourceRoots.length, 2);
  assert.ok(result.sourceRoots.some((value) => value.endsWith("/workspace")));
  assert.ok(result.sourceRoots.some((value) => value.endsWith("/runtime/src")));
});
