import test from "node:test";
import assert from "node:assert/strict";
import { chmodSync, mkdirSync, mkdtempSync, writeFileSync } from "node:fs";
import os from "node:os";
import path from "node:path";

import { discoverInterpreterCandidates } from "../src/interpreterDiscovery";

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
