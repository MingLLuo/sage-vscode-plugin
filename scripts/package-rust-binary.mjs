#!/usr/bin/env node
import crypto from "node:crypto";
import fs from "node:fs/promises";
import fsSync from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import {
  cargoReleaseBinaryPath,
  resolvePackageTarget,
  selectedPackageTargetName,
  stagedPaths,
} from "./lib/package-targets.mjs";

// Stages an already built `sage-ls` for one VS Code target under
// packages/extension-core/resources/bin/<platform>-<arch>/ together with its
// SHA-256 and build metadata. Supported targets: darwin-arm64, darwin-x64,
// linux-x64, linux-arm64, alpine-x64, alpine-arm64, win32-x64, win32-arm64.
const __dirname = path.dirname(fileURLToPath(import.meta.url));
const repositoryRoot = path.resolve(__dirname, "..");
const args = parseArgs(process.argv.slice(2));
const profile = args.profile ?? "release";
const dryRun = args.dryRun === true;

const target = packageTarget();
const staged = stagedPaths(repositoryRoot, target);
const source = args.source
  ? path.resolve(args.source)
  : cargoReleaseBinaryPath(repositoryRoot, target, profile);

if (!fsSync.existsSync(source)) {
  fail(
    `missing source binary: ${source}. `
    + `Run npm run package:rust-binary -- --target ${target.name} to build and stage it.`,
  );
}

const binary = await fs.readFile(source);
const sha256 = crypto.createHash("sha256").update(binary).digest("hex");
// The metadata describes the binary, not the VSIX: linux-* and alpine-* share
// one musl binary, so staging either produces byte-identical files.
const metadata = {
  name: "sage-ls",
  platform: target.platform,
  arch: target.arch,
  rustTarget: target.rustTarget,
  profile,
  source: slash(path.relative(repositoryRoot, source)),
  destination: slash(path.relative(repositoryRoot, staged.binary)),
  sha256,
};

if (!dryRun) {
  await fs.mkdir(staged.directory, { recursive: true });
  await fs.copyFile(source, staged.binary);
  await fs.chmod(staged.binary, 0o755);
  await fs.writeFile(staged.sha256, `${sha256}  ${target.executable}\n`, "utf8");
  await fs.writeFile(staged.metadata, `${JSON.stringify(metadata, null, 2)}\n`, "utf8");
}

console.log(JSON.stringify({
  status: dryRun ? "dry-run" : "packaged",
  target: target.name,
  ...metadata,
}, null, 2));

function packageTarget() {
  try {
    return resolvePackageTarget(args.target ?? selectedPackageTargetName([]));
  } catch (error) {
    return fail(error.message);
  }
}

function parseArgs(rawArgs) {
  const parsed = {};
  for (let index = 0; index < rawArgs.length; index += 1) {
    const item = rawArgs[index];
    if (item === "--dry-run") {
      parsed.dryRun = true;
      continue;
    }
    if (item.startsWith("--target=")) {
      parsed.target = item.slice("--target=".length);
      continue;
    }
    if (item === "--profile" || item === "--target" || item === "--source") {
      const value = rawArgs[index + 1];
      if (!value) {
        fail(`missing value for ${item}`);
      }
      parsed[item.slice(2)] = value;
      index += 1;
      continue;
    }
    fail(`unknown argument: ${item}`);
  }
  return parsed;
}

function slash(value) {
  return value.split(path.sep).join("/");
}

function fail(message) {
  console.error(message);
  process.exit(1);
}
