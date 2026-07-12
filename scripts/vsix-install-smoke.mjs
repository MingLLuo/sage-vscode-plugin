#!/usr/bin/env node
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { assertPinnedNodeVersion } from "./package-toolchain.mjs";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const repositoryRoot = path.resolve(__dirname, "..");
const packageRoot = path.join(repositoryRoot, "packages", "extension-core");
const manifest = JSON.parse(fs.readFileSync(path.join(packageRoot, "package.json"), "utf8"));
const extensionId = `${manifest.publisher}.${manifest.name}`;
const tempRoot = fs.mkdtempSync(path.join(os.tmpdir(), "sage-vsix-install-smoke-"));
const vsixOutDir = path.join(tempRoot, "vsix");
const extensionsDir = path.join(tempRoot, "extensions");
const userDataDir = path.join(tempRoot, "user-data");
const codeCli = findCodeCli();

if (!codeCli) {
  console.log(JSON.stringify({
    status: "skipped",
    reason: "VS Code CLI not found. Set SAGE_CODE_CLI to run the install smoke.",
  }, null, 2));
  process.exit(0);
}

assertPinnedNodeVersion(repositoryRoot);
stagePackagedRustBinary();
buildExtension();

const packageResult = spawnSync(
  process.execPath,
  [path.join(repositoryRoot, "scripts", "package-vsix.mjs"), "--out-dir", vsixOutDir],
  { cwd: repositoryRoot, encoding: "utf8" },
);
if (packageResult.status !== 0) {
  process.stderr.write(packageResult.stdout);
  process.stderr.write(packageResult.stderr);
  process.exit(packageResult.status ?? 1);
}

const vsixPath = path.join(vsixOutDir, `${manifest.name}-${manifest.version}.vsix`);
const installResult = runCode([
  "--install-extension",
  vsixPath,
  "--extensions-dir",
  extensionsDir,
  "--user-data-dir",
  userDataDir,
  "--force",
]);
const listResult = runCode([
  "--list-extensions",
  "--extensions-dir",
  extensionsDir,
  "--user-data-dir",
  userDataDir,
]);

const installedExtensions = listResult.stdout
  .split(/\r?\n/)
  .map((line) => line.trim())
  .filter(Boolean);
const installedInventory = readInstalledInventory(extensionsDir);
const installedExtensionRecord = installedInventory.find(
  (entry) => entry.identifier?.id === extensionId && entry.version === manifest.version,
);
const installedExtensionDir = installedExtensionRecord?.location?.fsPath
  ?? path.join(extensionsDir, `${extensionId}-${manifest.version}`);
const installedManifestPath = path.join(installedExtensionDir, "package.json");
const installedManifest = readJsonIfExists(installedManifestPath);
const checks = [
  {
    name: "VSIX installs with VS Code CLI",
    pass: installResult.status === 0,
    actual: compactOutput(installResult),
  },
  {
    name: "installed extension is recorded in isolated inventory",
    pass: Boolean(installedExtensionRecord),
    actual: installedInventory.map((entry) => ({
      id: entry.identifier?.id,
      version: entry.version,
      relativeLocation: entry.relativeLocation,
    })),
  },
  {
    name: "installed package manifest matches extension id",
    pass:
      installedManifest?.publisher === manifest.publisher
      && installedManifest?.name === manifest.name
      && installedManifest?.version === manifest.version,
    actual: installedManifest
      ? {
          publisher: installedManifest.publisher,
          name: installedManifest.name,
          version: installedManifest.version,
        }
      : null,
  },
  {
    name: "installed extension id is listed or inventory-backed",
    pass: installedExtensions.includes(extensionId) || Boolean(installedExtensionRecord),
    actual: {
      list: installedExtensions,
      inventory: installedInventory.map((entry) => entry.identifier?.id).filter(Boolean),
    },
  },
];
const failures = checks.filter((check) => !check.pass);
console.log(JSON.stringify({
  status: failures.length ? "failed" : "passed",
  codeCli,
  vsix: vsixPath,
  extensionId,
  checks,
}, null, 2));
if (failures.length) {
  process.exitCode = 1;
}

function findCodeCli() {
  const candidates = [
    process.env.SAGE_CODE_CLI,
    "code",
    "/Applications/Visual Studio Code.app/Contents/Resources/app/bin/code",
    "/Applications/Visual Studio Code - Insiders.app/Contents/Resources/app/bin/code",
  ].filter(Boolean);
  for (const candidate of candidates) {
    const result = spawnSync(candidate, ["--version"], { encoding: "utf8" });
    if (result.status === 0) {
      return candidate;
    }
  }
  return null;
}

function buildExtension() {
  const npmCommand = process.platform === "win32" ? "npm.cmd" : "npm";
  const result = spawnSync(npmCommand, ["run", "build", "--workspace", "sage-vscode-extension"], {
    cwd: repositoryRoot,
    encoding: "utf8",
  });
  if (result.status !== 0) {
    process.stderr.write(result.stdout);
    process.stderr.write(result.stderr);
    process.exit(result.status ?? 1);
  }
}

function stagePackagedRustBinary() {
  const npmCommand = process.platform === "win32" ? "npm.cmd" : "npm";
  const result = spawnSync(npmCommand, ["run", "package:rust-binary"], {
    cwd: repositoryRoot,
    encoding: "utf8",
  });
  if (result.status !== 0) {
    process.stderr.write(result.stdout);
    process.stderr.write(result.stderr);
    process.exit(result.status ?? 1);
  }
}

function runCode(args) {
  return spawnSync(codeCli, args, {
    cwd: repositoryRoot,
    encoding: "utf8",
    maxBuffer: 10 * 1024 * 1024,
  });
}

function compactOutput(result) {
  return {
    status: result.status,
    stdout: result.stdout.trim().split(/\r?\n/).slice(-5),
    stderr: result.stderr.trim().split(/\r?\n/).filter(Boolean).slice(-5),
  };
}

function readInstalledInventory(directory) {
  const inventoryPath = path.join(directory, "extensions.json");
  const inventory = readJsonIfExists(inventoryPath);
  return Array.isArray(inventory) ? inventory : [];
}

function readJsonIfExists(filePath) {
  try {
    return JSON.parse(fs.readFileSync(filePath, "utf8"));
  } catch {
    return null;
  }
}
