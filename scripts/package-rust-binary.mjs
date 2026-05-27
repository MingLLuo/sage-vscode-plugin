#!/usr/bin/env node
import crypto from "node:crypto";
import fs from "node:fs/promises";
import fsSync from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const repositoryRoot = path.resolve(__dirname, "..");
const args = parseArgs(process.argv.slice(2));
const platform = args.platform ?? process.platform;
const arch = args.arch ?? process.arch;
const profile = args.profile ?? "release";
const dryRun = args.dryRun === true;

const target = packageTarget(platform, arch);
const executable = platform === "win32" ? "sage-ls.exe" : "sage-ls";
const source = args.source
  ? path.resolve(args.source)
  : path.join(repositoryRoot, "target", profile, executable);
const destinationDirectory = path.join(
  repositoryRoot,
  "packages",
  "extension-core",
  "resources",
  "bin",
  target.directory,
);
const destination = path.join(destinationDirectory, target.executable);
const sha256Path = `${destination}.sha256`;
const metadataPath = path.join(destinationDirectory, "sage-ls.meta.json");

if (!fsSync.existsSync(source)) {
  fail(`missing source binary: ${source}. Run cargo build --${profile} -p sage-ls first.`);
}

const binary = await fs.readFile(source);
const sha256 = crypto.createHash("sha256").update(binary).digest("hex");
const metadata = {
  name: "sage-ls",
  platform,
  arch,
  profile,
  source: path.relative(repositoryRoot, source),
  destination: path.relative(repositoryRoot, destination),
  sha256,
};

if (!dryRun) {
  await fs.mkdir(destinationDirectory, { recursive: true });
  await fs.copyFile(source, destination);
  if (platform !== "win32") {
    await fs.chmod(destination, 0o755);
  }
  await fs.writeFile(sha256Path, `${sha256}  ${target.executable}\n`, "utf8");
  await fs.writeFile(metadataPath, `${JSON.stringify(metadata, null, 2)}\n`, "utf8");
}

console.log(JSON.stringify({
  status: dryRun ? "dry-run" : "packaged",
  ...metadata,
}, null, 2));

function packageTarget(rawPlatform, rawArch) {
  const normalizedArch = normalizeArch(rawArch);
  const supported = new Set([
    "darwin-arm64",
    "darwin-x64",
    "linux-x64",
    "win32-x64",
  ]);
  const directory = `${rawPlatform}-${normalizedArch}`;
  if (!supported.has(directory)) {
    fail(`unsupported packaged binary target: ${directory}`);
  }
  return {
    directory,
    executable: rawPlatform === "win32" ? "sage-ls.exe" : "sage-ls",
  };
}

function normalizeArch(rawArch) {
  if (rawArch === "amd64") {
    return "x64";
  }
  return rawArch;
}

function parseArgs(rawArgs) {
  const parsed = {};
  for (let index = 0; index < rawArgs.length; index += 1) {
    const item = rawArgs[index];
    if (item === "--dry-run") {
      parsed.dryRun = true;
      continue;
    }
    if (item === "--profile" || item === "--platform" || item === "--arch" || item === "--source") {
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

function fail(message) {
  console.error(message);
  process.exit(1);
}
