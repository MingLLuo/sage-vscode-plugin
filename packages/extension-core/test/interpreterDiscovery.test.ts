import test from "node:test";
import assert from "node:assert/strict";
import { chmodSync, mkdirSync, mkdtempSync, writeFileSync } from "node:fs";
import os from "node:os";
import path from "node:path";

import {
  discoverInterpreterCandidates,
  resolveInterpreterConfigurationUpdates,
} from "../src/interpreterDiscovery";

test("discoverInterpreterCandidates prioritizes local development and system Sage environments", () => {
  const tempRoot = mkdtempSync(path.join(os.tmpdir(), "sage-discovery-"));
  const binDir = path.join(tempRoot, "bin");
  const workspaceDir = path.join(tempRoot, "workspace");
  const siblingSageDir = path.join(tempRoot, "sage");
  const pathPython = path.join(binDir, "python3");
  const pathSage = path.join(binDir, "sage");
  const workspaceSage = path.join(siblingSageDir, "sage");
  const homeDir = path.join(tempRoot, "home");
  const sageDevPython = path.join(homeDir, "miniforge3", "envs", "sage-dev", "bin", "python");
  const homePython = path.join(homeDir, "miniforge3", "bin", "python");
  const workspaceSageBootstrap = path.join(siblingSageDir, "src", "bin", "sage");
  const workspaceSagePackage = path.join(siblingSageDir, "src", "sage", "__init__.py");

  writeExecutable(pathPython);
  writeExecutable(pathSage);
  writeExecutable(workspaceSage);
  writeExecutable(workspaceSageBootstrap);
  writeExecutable(homePython);
  writeExecutable(sageDevPython);
  writeExecutable(workspaceSagePackage);

  const items = discoverInterpreterCandidates({
    currentPath: workspaceSage,
    languageServerPythonPath: "auto",
    workspaceFolders: [workspaceDir],
    envPath: binDir,
    homeDir,
    environment: {},
  });

  assertEnvironmentCandidate(
    items,
    workspaceSage,
    sageDevPython,
    "Local Sage development environment",
  );
  assertEnvironmentCandidate(
    items,
    pathSage,
    pathPython,
    "System Sage (stable)",
  );
  assert.ok(items.some((item) => item.selectionTarget === "runtimeCustom"));
  assert.ok(items.some((item) => item.selectionTarget === "languageServerCustom"));
  assert.ok(items.some((item) => item.selectionTarget === "languageServerAuto"));
});

test("resolveInterpreterConfigurationUpdates maps environment and auto selections", async () => {
  assert.deepEqual(
    await resolveInterpreterConfigurationUpdates(
      {
        label: "Use auto-detected language-server Python",
        selectionTarget: "languageServerAuto",
      },
      { interpreterPath: "/usr/bin/sage", languageServerPythonPath: "/usr/bin/python3" },
      unusedPrompts(),
    ),
    [{ section: "languageServer.pythonPath", value: "auto" }],
  );

  assert.deepEqual(
    await resolveInterpreterConfigurationUpdates(
      {
        label: "Detected",
        selectionTarget: "environment",
        updates: [
          { section: "interpreter.path", value: "/opt/sage" },
          { section: "languageServer.pythonPath", value: "/opt/python" },
        ],
      },
      { interpreterPath: "/usr/bin/sage", languageServerPythonPath: "auto" },
      unusedPrompts(),
    ),
    [
      { section: "interpreter.path", value: "/opt/sage" },
      { section: "languageServer.pythonPath", value: "/opt/python" },
    ],
  );
});

test("resolveInterpreterConfigurationUpdates prompts for custom paths", async () => {
  assert.deepEqual(
    await resolveInterpreterConfigurationUpdates(
      { label: "Custom Sage", selectionTarget: "runtimeCustom" },
      { interpreterPath: "/old/sage", languageServerPythonPath: "auto" },
      {
        runtimePath: async (initialValue) => `${initialValue}-new`,
        languageServerPythonPath: async () => {
          throw new Error("unexpected language-server prompt");
        },
      },
    ),
    [{ section: "interpreter.path", value: "/old/sage-new" }],
  );

  assert.deepEqual(
    await resolveInterpreterConfigurationUpdates(
      { label: "Custom Python", selectionTarget: "languageServerCustom" },
      { interpreterPath: "/usr/bin/sage", languageServerPythonPath: "auto" },
      {
        runtimePath: async () => {
          throw new Error("unexpected runtime prompt");
        },
        languageServerPythonPath: async (initialValue) => initialValue || "/custom/python",
      },
    ),
    [{ section: "languageServer.pythonPath", value: "/custom/python" }],
  );
});

test("resolveInterpreterConfigurationUpdates trims custom paths and skips no-op updates", async () => {
  assert.deepEqual(
    await resolveInterpreterConfigurationUpdates(
      { label: "Current", selectionTarget: "languageServerAuto" },
      { interpreterPath: "sage", languageServerPythonPath: "auto" },
      unusedPrompts(),
    ),
    [],
  );
  assert.deepEqual(
    await resolveInterpreterConfigurationUpdates(
      { label: "Custom Sage", selectionTarget: "runtimeCustom" },
      { interpreterPath: "sage", languageServerPythonPath: "auto" },
      {
        ...unusedPrompts(),
        runtimePath: async () => "  /opt/sage/bin/sage  ",
      },
    ),
    [{ section: "interpreter.path", value: "/opt/sage/bin/sage" }],
  );
  assert.equal(
    await resolveInterpreterConfigurationUpdates(
      { label: "Blank", selectionTarget: "runtimeCustom" },
      { interpreterPath: "sage", languageServerPythonPath: "auto" },
      {
        ...unusedPrompts(),
        runtimePath: async () => "   ",
      },
    ),
    undefined,
  );
  assert.deepEqual(
    await resolveInterpreterConfigurationUpdates(
      {
        label: "Current environment",
        selectionTarget: "environment",
        updates: [
          { section: "interpreter.path", value: "sage" },
          { section: "languageServer.pythonPath", value: "/usr/bin/python3" },
        ],
      },
      { interpreterPath: "sage", languageServerPythonPath: "auto" },
      unusedPrompts(),
    ),
    [{ section: "languageServer.pythonPath", value: "/usr/bin/python3" }],
  );
});

function unusedPrompts() {
  return {
    runtimePath: async () => {
      throw new Error("unexpected runtime prompt");
    },
    languageServerPythonPath: async () => {
      throw new Error("unexpected language-server prompt");
    },
  };
}

function assertEnvironmentCandidate(
  items: ReturnType<typeof discoverInterpreterCandidates>,
  interpreterPath: string,
  languageServerPythonPath: string,
  label: string,
): void {
  assert.ok(
    items.some(
      (item) =>
        item.interpreterPath === interpreterPath
        && item.languageServerPythonPath === languageServerPythonPath
        && item.selectionTarget === "environment"
        && item.label === label,
    ),
  );
}

function writeExecutable(targetPath: string): void {
  mkdirSync(path.dirname(targetPath), { recursive: true });
  writeFileSync(targetPath, "#!/bin/sh\nexit 0\n", { encoding: "utf-8" });
  chmodSync(targetPath, 0o755);
}
