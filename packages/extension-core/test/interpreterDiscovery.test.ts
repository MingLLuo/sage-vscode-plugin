import test from "node:test";
import assert from "node:assert/strict";
import { chmodSync, mkdirSync, mkdtempSync, writeFileSync } from "node:fs";
import os from "node:os";
import path from "node:path";

import { discoverInterpreterCandidates } from "../src/interpreterDiscovery";

test("discoverInterpreterCandidates lists Sage runtimes, Python runtimes, and manual actions", () => {
  const tempRoot = mkdtempSync(path.join(os.tmpdir(), "sage-discovery-"));
  const binDir = path.join(tempRoot, "bin");
  const workspaceDir = path.join(tempRoot, "workspace");
  const siblingSageDir = path.join(tempRoot, "sage");
  const workspaceVenvPython = path.join(workspaceDir, ".venv", "bin", "python");
  const pathPython = path.join(binDir, "python3");
  const pathSage = path.join(binDir, "sage");
  const workspaceSage = path.join(siblingSageDir, "sage");
  const homeDir = path.join(tempRoot, "home");
  const homePython = path.join(homeDir, "miniforge3", "bin", "python");

  writeExecutable(pathPython);
  writeExecutable(pathSage);
  writeExecutable(workspaceSage);
  writeExecutable(workspaceVenvPython);
  writeExecutable(homePython);

  const items = discoverInterpreterCandidates({
    currentPath: "/opt/current/sage",
    languageServerPythonPath: "/opt/custom/python",
    workspaceFolders: [workspaceDir],
    envPath: binDir,
    homeDir,
  });

  assertCandidate(items, "/opt/current/sage", "runtime", "Current Sage runtime");
  assertCandidate(items, "/opt/custom/python", "languageServer", "Current language-server Python");
  assertCandidate(items, pathSage, "runtime", "Detected Sage runtime");
  assertCandidate(items, pathPython, "languageServer", "Detected language-server Python");
  assertCandidate(items, workspaceSage, "runtime", "Detected Sage runtime");
  assertCandidate(items, workspaceVenvPython, "languageServer", "Detected language-server Python");
  assertCandidate(items, homePython, "languageServer", "Detected language-server Python");
  assert.ok(items.some((item) => item.selectionTarget === "runtimeCustom"));
  assert.ok(items.some((item) => item.selectionTarget === "languageServerCustom"));
  assert.ok(items.some((item) => item.selectionTarget === "languageServerAuto"));
});

function assertCandidate(
  items: ReturnType<typeof discoverInterpreterCandidates>,
  interpreterPath: string,
  selectionTarget: "runtime" | "languageServer",
  label: string,
): void {
  assert.ok(
    items.some(
      (item) =>
        item.interpreterPath === interpreterPath
        && item.selectionTarget === selectionTarget
        && item.label === label,
    ),
  );
}

function writeExecutable(targetPath: string): void {
  mkdirSync(path.dirname(targetPath), { recursive: true });
  writeFileSync(targetPath, "#!/bin/sh\nexit 0\n", { encoding: "utf-8" });
  chmodSync(targetPath, 0o755);
}
