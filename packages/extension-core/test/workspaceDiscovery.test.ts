import test from "node:test";
import assert from "node:assert/strict";
import os from "node:os";
import path from "node:path";
import fs from "node:fs";

import {
  buildWorkspaceInitializationData,
  discoverInterpreterSourceRoots,
  discoverInterpreterSourceRootsAsync,
  discoverNearbySageSourceRoots,
  discoverSourceRoots,
  discoverSourceRootsAsync,
  resolveConfiguredPaths,
  resolveRuntimePythonPaths,
} from "../src/workspaceDiscovery";

test("discoverSourceRoots prefers configured roots when provided", () => {
  assert.deepEqual(discoverSourceRoots(["/workspace"], ["/configured/src"]), ["/configured/src"]);
});

test("discoverSourceRoots supplements configured roots with interpreter roots", () => {
  assert.deepEqual(
    discoverSourceRoots(["/workspace"], ["src"], {
      interpreterPath: "sage",
      runtimeProbe: () => ["/runtime/src"],
      exists: (candidate) => candidate === "/runtime/src/sage",
    }),
    ["/workspace/src", "/runtime/src"],
  );
});

test("discoverSourceRoots skips interpreter site-packages when a Sage checkout is nearby", () => {
  const tmpRoot = fs.mkdtempSync(path.join(os.tmpdir(), "sage-vscode-nearby-preferred-"));
  const workspaceRoot = path.join(tmpRoot, "sage-vscode-plugin", "examples", "manual-smoke-workspace");
  const workspaceSrc = path.join(workspaceRoot, "src");
  const sageSrc = path.join(tmpRoot, "sage", "src");
  const sitePackagesRoot = path.join(tmpRoot, "sage", "local", "lib", "python3.14", "site-packages");
  fs.mkdirSync(workspaceSrc, { recursive: true });
  fs.mkdirSync(path.join(sageSrc, "sage"), { recursive: true });
  fs.mkdirSync(path.join(sitePackagesRoot, "sage"), { recursive: true });

  assert.deepEqual(
    discoverSourceRoots([workspaceRoot], ["src"], {
      interpreterPath: "sage",
      runtimeProbe: () => [sitePackagesRoot],
    }),
    [workspaceSrc, sageSrc],
  );
});

test("discoverSourceRoots keeps interpreter site-packages when it is the only Sage source", () => {
  const tmpRoot = fs.mkdtempSync(path.join(os.tmpdir(), "sage-vscode-installed-only-"));
  const workspaceRoot = path.join(tmpRoot, "workspace");
  const sitePackagesRoot = path.join(tmpRoot, "local", "lib", "python3.14", "site-packages");
  fs.mkdirSync(workspaceRoot, { recursive: true });
  fs.mkdirSync(path.join(sitePackagesRoot, "sage"), { recursive: true });

  assert.deepEqual(
    discoverSourceRoots([workspaceRoot], [], {
      interpreterPath: "sage",
      runtimeProbe: () => [sitePackagesRoot],
    }),
    [workspaceRoot, sitePackagesRoot],
  );
});

test("resolveConfiguredPaths resolves relative paths against workspace folders", () => {
  assert.deepEqual(
    resolveConfiguredPaths(["/workspace-a", "/workspace-b"], ["src"]),
    ["/workspace-a/src", "/workspace-b/src"],
  );
});

test("resolveRuntimePythonPaths combines active file directory, source roots, and extra paths", () => {
  assert.deepEqual(
    resolveRuntimePythonPaths(
      ["/workspace-a", "/workspace-b"],
      ["src"],
      ["/shared/stubs"],
      "/workspace-a/examples/demo.sage",
    ),
    [
      "/workspace-a/examples",
      "/workspace-a/src",
      "/workspace-b/src",
      "/shared/stubs",
    ],
  );
});

test("discoverSourceRoots maps a Sage source checkout to its src root", () => {
  const tmpRoot = fs.mkdtempSync(path.join(os.tmpdir(), "sage-vscode-workspace-"));
  fs.mkdirSync(path.join(tmpRoot, "src", "sage"), { recursive: true });

  assert.deepEqual(discoverSourceRoots([tmpRoot], []), [path.join(tmpRoot, "src")]);
});

test("discoverNearbySageSourceRoots finds sibling Sage checkouts from nested workspaces", () => {
  const tmpRoot = fs.mkdtempSync(path.join(os.tmpdir(), "sage-vscode-nearby-"));
  const workspaceRoot = path.join(tmpRoot, "sage-vscode-plugin", "examples", "manual-smoke-workspace");
  const sageSrc = path.join(tmpRoot, "sage", "src");
  fs.mkdirSync(workspaceRoot, { recursive: true });
  fs.mkdirSync(path.join(sageSrc, "sage"), { recursive: true });

  assert.deepEqual(discoverNearbySageSourceRoots([workspaceRoot]), [sageSrc]);
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

test("discoverInterpreterSourceRoots can skip runtime probing for startup paths", () => {
  let probeCalled = false;
  assert.deepEqual(
    discoverInterpreterSourceRoots("sage", [], {
      runtimeProbe: false,
      exists: (candidate) => {
        probeCalled = probeCalled || candidate === "/runtime/src/sage";
        return false;
      },
    }),
    [],
  );
  assert.equal(probeCalled, false);
});

test("discoverInterpreterSourceRootsAsync awaits runtime probing off startup path", async () => {
  assert.deepEqual(
    await discoverInterpreterSourceRootsAsync("sage", [], {
      runtimeProbe: async () => ["/runtime/src"],
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

test("buildWorkspaceInitializationData skips runtime probe when requested", () => {
  const tmpRoot = fs.mkdtempSync(path.join(os.tmpdir(), "sage-vscode-runtime-skip-init-"));
  const workspaceRoot = path.join(tmpRoot, "workspace");
  fs.mkdirSync(workspaceRoot, { recursive: true });

  const result = buildWorkspaceInitializationData([workspaceRoot], [], {
    interpreterPath: "sage",
    runtimeProbe: false,
    exists: (candidate) => candidate === workspaceRoot || candidate === "/runtime/src/sage",
  });

  assert.equal(result.sourceRoots.length, 1);
  assert.ok(result.sourceRoots[0]?.endsWith("/workspace"));
});

test("discoverSourceRootsAsync can supplement startup roots from runtime probe", async () => {
  const tmpRoot = fs.mkdtempSync(path.join(os.tmpdir(), "sage-vscode-runtime-async-"));
  const workspaceRoot = path.join(tmpRoot, "workspace");
  fs.mkdirSync(workspaceRoot, { recursive: true });

  const result = await discoverSourceRootsAsync([workspaceRoot], [], {
    interpreterPath: "sage",
    runtimeProbe: async () => ["/runtime/src"],
    exists: (candidate) => candidate === workspaceRoot || candidate === "/runtime/src/sage",
  });

  assert.deepEqual(result, [workspaceRoot, "/runtime/src"]);
});
