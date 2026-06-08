#!/usr/bin/env node
import crypto from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const repositoryRoot = path.resolve(__dirname, "..");
const packageRoot = path.join(repositoryRoot, "packages", "extension-core");
const args = parseArgs(process.argv.slice(2));
const checks = [];

const manifestPath = path.join(packageRoot, "package.json");
const manifest = readJsonIfExists(manifestPath);
const extensionId = manifest ? `${manifest.publisher}.${manifest.name}` : "unknown";
const vsixPath = manifest
  ? path.join(repositoryRoot, "dist", `${manifest.name}-${manifest.version}.vsix`)
  : path.join(repositoryRoot, "dist", "sage-vscode-extension-unknown.vsix");
const binaryDirectory = `darwin-${normalizeArch(process.arch)}`;
const packagedBinaryPath = path.join(packageRoot, "resources", "bin", binaryDirectory, "sage-ls");
const sourceRoot = resolveSourceRoot();
const sageRuntime = resolveSageRuntime();
const codeCli = findCodeCli();

pushCheck("required", "macOS host", process.platform === "darwin", platformSummary());
pushCheck("required", "supported Mac architecture", ["arm64", "x64"].includes(normalizeArch(process.arch)), platformSummary());
pushCheck("required", "extension manifest exists", Boolean(manifest), relativeOrAbsolute(manifestPath));
pushCheck("required", "extension identity is valid", Boolean(manifest?.name && manifest?.publisher && manifest?.version), manifest ? {
  id: extensionId,
  version: manifest.version,
} : null);
pushCheck("required", "packaged Rust language server exists for this Mac", fs.existsSync(packagedBinaryPath), relativeOrAbsolute(packagedBinaryPath));
pushCheck("required", "packaged Rust language server is executable", isExecutable(packagedBinaryPath), relativeOrAbsolute(packagedBinaryPath));
pushHashChecks(packagedBinaryPath);
pushCheck("required", "VSIX package exists", fs.existsSync(vsixPath), relativeOrAbsolute(vsixPath));
pushCheck("recommended", "VS Code CLI is available", Boolean(codeCli), codeCli ?? "Set SAGE_CODE_CLI or install the `code` shell command.");
pushCheck("recommended", "Sage runtime is available", Boolean(sageRuntime.path), sageRuntime);
pushCheck("recommended", "Sage source root is available", Boolean(sourceRoot.path), sourceRoot);

const requiredFailures = checks.filter((check) => check.severity === "required" && !check.pass);
const recommendedFailures = checks.filter((check) => check.severity === "recommended" && !check.pass);
const status = requiredFailures.length
  ? "action-needed"
  : recommendedFailures.length
    ? "usable-with-warnings"
    : "ready";
const report = {
  schema_version: 1,
  status,
  extension_id: extensionId,
  version: manifest?.version ?? null,
  platform: platformSummary(),
  next_commands: nextCommands(),
  checks,
};

if (args.json) {
  console.log(JSON.stringify(report, null, 2));
} else {
  printHuman(report);
}

if (args.strict && status === "action-needed") {
  process.exitCode = 1;
}
if (args.requireSage && !sageRuntime.path) {
  process.exitCode = 1;
}
if (args.requireVsCodeCli && !codeCli) {
  process.exitCode = 1;
}

function pushHashChecks(binaryPath) {
  const hashPath = `${binaryPath}.sha256`;
  const metadataPath = path.join(path.dirname(binaryPath), "sage-ls.meta.json");
  pushCheck("required", "packaged Rust hash file exists", fs.existsSync(hashPath), relativeOrAbsolute(hashPath));
  pushCheck("required", "packaged Rust metadata exists", fs.existsSync(metadataPath), relativeOrAbsolute(metadataPath));
  if (!fs.existsSync(binaryPath) || !fs.existsSync(hashPath) || !fs.existsSync(metadataPath)) {
    return;
  }
  const binaryHash = sha256(binaryPath);
  const hashText = fs.readFileSync(hashPath, "utf8");
  const metadata = readJsonIfExists(metadataPath);
  pushCheck("required", "packaged Rust hash matches binary", hashText.includes(binaryHash), {
    expected: binaryHash,
    file: relativeOrAbsolute(hashPath),
  });
  pushCheck("required", "packaged Rust metadata matches this Mac", metadata?.platform === "darwin"
    && normalizeArch(metadata?.arch) === normalizeArch(process.arch)
    && metadata?.sha256 === binaryHash, metadata);
}

function resolveSageRuntime() {
  const candidates = [
    args.sage,
    process.env.SAGE_PATH,
    "sage",
    "/Applications/SageMath/sage",
    "/Applications/SageMath.app/Contents/MacOS/sage",
    "/Applications/Sage Math.app/Contents/MacOS/sage",
  ].filter(Boolean);
  for (const candidate of candidates) {
    const result = spawnSync(candidate, ["--version"], {
      encoding: "utf8",
      timeout: 8000,
      maxBuffer: 1024 * 1024,
    });
    if (result.status === 0) {
      return {
        path: candidate,
        version: firstLine(result.stdout) || firstLine(result.stderr),
      };
    }
  }
  return {
    path: null,
    reason: args.sage
      ? `configured Sage runtime did not execute: ${args.sage}`
      : "Set SAGE_PATH or configure `sage.interpreter.path` in VS Code.",
  };
}

function resolveSourceRoot() {
  const candidates = [
    args.sourceRoot,
    process.env.SAGE_SOURCE_ROOT,
    path.resolve(repositoryRoot, "..", "sage", "src"),
    path.resolve(repositoryRoot, "sage", "src"),
  ].filter(Boolean);
  for (const candidate of candidates) {
    const resolved = path.resolve(candidate);
    if (isSageSourceRoot(resolved)) {
      return {
        path: resolved,
        source: candidate === args.sourceRoot ? "argument" : candidate === process.env.SAGE_SOURCE_ROOT ? "environment" : "nearby-checkout",
      };
    }
  }
  return {
    path: null,
    reason: "Set SAGE_SOURCE_ROOT or `sage.analysis.sourceRoots` to the Sage `src` directory for faster indexing.",
  };
}

function isSageSourceRoot(candidate) {
  return fs.existsSync(path.join(candidate, "sage", "all.py"))
    || fs.existsSync(path.join(candidate, "sage", "all.pyx"))
    || fs.existsSync(path.join(candidate, "sage", "__init__.py"));
}

function findCodeCli() {
  const candidates = [
    process.env.SAGE_CODE_CLI,
    "code",
    "/Applications/Visual Studio Code.app/Contents/Resources/app/bin/code",
    "/Applications/Visual Studio Code - Insiders.app/Contents/Resources/app/bin/code",
  ].filter(Boolean);
  for (const candidate of candidates) {
    const result = spawnSync(candidate, ["--version"], {
      encoding: "utf8",
      timeout: 5000,
      maxBuffer: 1024 * 1024,
    });
    if (result.status === 0) {
      return candidate;
    }
  }
  return null;
}

function nextCommands() {
  const commands = [];
  if (!fs.existsSync(packagedBinaryPath) || !fs.existsSync(vsixPath)) {
    commands.push("npm run package:vsix");
  }
  if (fs.existsSync(vsixPath)) {
    commands.push(`code --install-extension ${path.relative(repositoryRoot, vsixPath)} --force`);
  }
  commands.push("Sage: Open Getting Started");
  commands.push("Sage: Run UX Self Check");
  return commands;
}

function printHuman(report) {
  console.log(`Sage VS Code macOS doctor: ${report.status}`);
  console.log(`Extension: ${report.extension_id}${report.version ? ` ${report.version}` : ""}`);
  console.log(`Platform: ${report.platform.platform}-${report.platform.arch}`);
  console.log("");
  for (const severity of ["required", "recommended"]) {
    const rows = report.checks.filter((check) => check.severity === severity);
    console.log(`${capitalize(severity)} checks:`);
    for (const check of rows) {
      console.log(`${checkMarker(check)} ${check.name}`);
      if (!check.pass) {
        console.log(`  ${formatActual(check.actual)}`);
      }
    }
    console.log("");
  }
  console.log("Next commands:");
  for (const command of report.next_commands) {
    console.log(`- ${command}`);
  }
  console.log("");
  console.log("Use `npm run doctor:mac -- --json` for machine-readable details.");
}

function parseArgs(rawArgs) {
  const parsed = {
    json: false,
    strict: false,
    requireSage: false,
    requireVsCodeCli: false,
    sage: null,
    sourceRoot: null,
  };
  for (let index = 0; index < rawArgs.length; index += 1) {
    const item = rawArgs[index];
    if (item === "--json") {
      parsed.json = true;
      continue;
    }
    if (item === "--strict") {
      parsed.strict = true;
      continue;
    }
    if (item === "--require-sage") {
      parsed.requireSage = true;
      continue;
    }
    if (item === "--require-vscode-cli") {
      parsed.requireVsCodeCli = true;
      continue;
    }
    if (item === "--sage" || item === "--source-root") {
      const value = rawArgs[index + 1];
      if (!value) {
        throw new Error(`missing value for ${item}`);
      }
      if (item === "--sage") {
        parsed.sage = value;
      } else {
        parsed.sourceRoot = value;
      }
      index += 1;
      continue;
    }
    if (item === "--help" || item === "-h") {
      console.log(`Usage: node scripts/macos-doctor.mjs [--json] [--strict] [--sage PATH] [--source-root PATH]

Checks the local macOS VS Code extension package, staged Rust language server,
VS Code CLI, Sage runtime, and Sage source root.

Default mode reports findings without failing the shell. Use --strict to fail
when required packaged artifacts are missing. Use --require-sage or
--require-vscode-cli to make those optional checks hard requirements.`);
      process.exit(0);
    }
    throw new Error(`unknown argument: ${item}`);
  }
  return parsed;
}

function pushCheck(severity, name, pass, actual) {
  checks.push({
    severity,
    name,
    pass: Boolean(pass),
    actual,
  });
}

function platformSummary() {
  return {
    platform: process.platform,
    arch: normalizeArch(process.arch),
    release: os.release(),
  };
}

function normalizeArch(value) {
  if (value === "amd64") {
    return "x64";
  }
  return value;
}

function isExecutable(filePath) {
  try {
    fs.accessSync(filePath, fs.constants.X_OK);
    return true;
  } catch {
    return false;
  }
}

function sha256(filePath) {
  return crypto.createHash("sha256").update(fs.readFileSync(filePath)).digest("hex");
}

function readJsonIfExists(filePath) {
  try {
    return JSON.parse(fs.readFileSync(filePath, "utf8"));
  } catch {
    return null;
  }
}

function relativeOrAbsolute(filePath) {
  const relative = path.relative(repositoryRoot, filePath);
  return relative.startsWith("..") ? filePath : relative;
}

function firstLine(text) {
  return String(text ?? "").trim().split(/\r?\n/).find(Boolean) ?? "";
}

function formatActual(actual) {
  return typeof actual === "string" ? actual : JSON.stringify(actual);
}

function checkMarker(check) {
  if (check.pass) {
    return "PASS";
  }
  return check.severity === "required" ? "FAIL" : "WARN";
}

function capitalize(value) {
  return value.charAt(0).toUpperCase() + value.slice(1);
}
