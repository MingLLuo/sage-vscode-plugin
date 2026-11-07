#!/usr/bin/env node
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";

const repositoryRoot = path.resolve(import.meta.dirname, "..");
const tempRoot = fs.mkdtempSync(path.join(os.tmpdir(), "sage-configure-smoke-"));
const workspace = path.join(tempRoot, "project");
const externalRoot = path.join(tempRoot, "external-sources");
fs.mkdirSync(workspace);
fs.mkdirSync(externalRoot);
fs.writeFileSync(path.join(workspace, "example.sage"), "def square(x):\n    return x*x\n");

function configure(extraArgs = []) {
  const result = spawnSync(process.execPath, [
    path.join(repositoryRoot, "scripts", "configure-workspace.mjs"),
    "--workspace", workspace, "--profile", "auto", "--json", ...extraArgs,
  ], { cwd: repositoryRoot, encoding: "utf8", timeout: 30_000 });
  assert.equal(result.status, 0, result.stderr || result.stdout);
  return JSON.parse(result.stdout);
}

try {
  const first = configure();
  assert.equal(first.profile, "standard");
  assert.equal(first.next_settings["sage.analysis.sourceRoots"].includes("."), true);
  for (const key of ["python.analysis.exclude", "python.analysis.ignore", "ruff.exclude", "ruff.configuration"]) {
    assert.equal(Object.hasOwn(first.next_settings, key), false, `${key} must not exclude the project`);
  }
  assert.equal(configure().status, "unchanged");

  const external = configure(["--source-root", externalRoot]);
  for (const key of ["python.analysis.exclude", "python.analysis.ignore", "ruff.exclude"]) {
    assert.deepEqual(external.next_settings[key], [externalRoot], key);
  }
  assert.deepEqual(external.next_settings["ruff.configuration"].exclude, [externalRoot]);
  const settingsPath = path.join(workspace, ".vscode", "settings.json");
  fs.writeFileSync(settingsPath, '{\n// keep my settings\n"editor.fontSize": 17,\n}');
  configure();
  assert.match(fs.readFileSync(settingsPath, "utf8"), /keep my settings/);
  assert.equal(configure().status, "unchanged");
  const invalid = '{"editor.fontSize":';
  fs.writeFileSync(settingsPath, invalid);
  const rejected = spawnSync(process.execPath, [path.join(repositoryRoot, "scripts/configure-workspace.mjs"), "--workspace", workspace], { encoding: "utf8" });
  assert.notEqual(rejected.status, 0);
  assert.equal(fs.readFileSync(settingsPath, "utf8"), invalid);
  console.log(JSON.stringify({ status: "passed", checks: ["standard profile", "idempotent configuration", "external source excludes"] }));
} finally {
  fs.rmSync(tempRoot, { recursive: true, force: true });
}
