import test from "node:test";
import assert from "node:assert/strict";
import { chmodSync, mkdirSync, mkdtempSync, writeFileSync } from "node:fs";
import os from "node:os";
import path from "node:path";

import {
  discoverInterpreterCandidates,
  resolveInterpreterConfigurationUpdates,
} from "../src/interpreterDiscovery";

test("discoverInterpreterCandidates lists the current, local development, and system Sage runtimes once each", () => {
  const tempRoot = mkdtempSync(path.join(os.tmpdir(), "sage-discovery-"));
  const binDir = path.join(tempRoot, "bin");
  const workspaceDir = path.join(tempRoot, "workspace");
  const siblingSageDir = path.join(tempRoot, "sage");
  const pathSage = path.join(binDir, "sage");
  const workspaceSage = path.join(siblingSageDir, "sage");

  writeExecutable(pathSage);
  writeExecutable(workspaceSage);
  writeExecutable(path.join(siblingSageDir, "src", "bin", "sage"));
  writeExecutable(path.join(siblingSageDir, "src", "sage", "__init__.py"));

  const items = discoverInterpreterCandidates({
    currentPath: workspaceSage,
    workspaceFolders: [workspaceDir],
    envPath: binDir,
  });

  assertEnvironmentCandidate(items, workspaceSage, "Current local Sage development environment");
  assertEnvironmentCandidate(items, pathSage, "System Sage (stable)");
  assert.equal(
    items.filter((item) => item.interpreterPath === workspaceSage).length,
    1,
    "the configured runtime and the detected checkout are the same environment",
  );
  assert.deepEqual(
    items.filter((item) => item.selectionTarget !== "environment").map((item) => item.selectionTarget),
    ["runtimeCustom"],
    "the picker offers exactly one manual action and no language-server Python entries",
  );
});

test("discoverInterpreterCandidates offers an unverified configured runtime as the current environment", () => {
  const items = discoverInterpreterCandidates({
    currentPath: "sage",
    workspaceFolders: [],
    envPath: "",
  });

  assertEnvironmentCandidate(items, "sage", "Current Sage environment");
});

test("resolveInterpreterConfigurationUpdates maps environment selections", async () => {
  assert.deepEqual(
    await resolveInterpreterConfigurationUpdates(
      { label: "Detected", selectionTarget: "environment", interpreterPath: "/opt/sage" },
      { interpreterPath: "/usr/bin/sage" },
      unusedPrompts(),
    ),
    [{ section: "interpreter.path", value: "/opt/sage" }],
  );
});

test("resolveInterpreterConfigurationUpdates prompts for a custom path", async () => {
  assert.deepEqual(
    await resolveInterpreterConfigurationUpdates(
      { label: "Custom Sage", selectionTarget: "runtimeCustom" },
      { interpreterPath: "/old/sage" },
      { runtimePath: async (initialValue) => `${initialValue}-new` },
    ),
    [{ section: "interpreter.path", value: "/old/sage-new" }],
  );
});

test("resolveInterpreterConfigurationUpdates trims custom paths and skips no-op updates", async () => {
  assert.deepEqual(
    await resolveInterpreterConfigurationUpdates(
      { label: "Custom Sage", selectionTarget: "runtimeCustom" },
      { interpreterPath: "sage" },
      { runtimePath: async () => "  /opt/sage/bin/sage  " },
    ),
    [{ section: "interpreter.path", value: "/opt/sage/bin/sage" }],
  );
  assert.equal(
    await resolveInterpreterConfigurationUpdates(
      { label: "Blank", selectionTarget: "runtimeCustom" },
      { interpreterPath: "sage" },
      { runtimePath: async () => "   " },
    ),
    undefined,
  );
  assert.deepEqual(
    await resolveInterpreterConfigurationUpdates(
      { label: "Current environment", selectionTarget: "environment", interpreterPath: "sage" },
      { interpreterPath: "sage" },
      unusedPrompts(),
    ),
    [],
  );
});

function unusedPrompts() {
  return {
    runtimePath: async () => {
      throw new Error("unexpected runtime prompt");
    },
  };
}

function assertEnvironmentCandidate(
  items: ReturnType<typeof discoverInterpreterCandidates>,
  interpreterPath: string,
  label: string,
): void {
  assert.ok(
    items.some(
      (item) =>
        item.interpreterPath === interpreterPath
        && item.selectionTarget === "environment"
        && item.label === label,
    ),
    `expected ${label} for ${interpreterPath}: ${JSON.stringify(items.map((item) => [item.label, item.interpreterPath]))}`,
  );
}

function writeExecutable(targetPath: string): void {
  mkdirSync(path.dirname(targetPath), { recursive: true });
  writeFileSync(targetPath, "#!/bin/sh\nexit 0\n", { encoding: "utf-8" });
  chmodSync(targetPath, 0o755);
}
