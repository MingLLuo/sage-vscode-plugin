#!/usr/bin/env node
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import {
  assertNpmSupportsNodeVersion,
  assertSupportedNodeVersion,
  assertSupportedNpmVersion,
  supportedNodeVersionRange,
  supportedNpmVersionRange,
} from "./package-toolchain.mjs";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const repositoryRoot = path.resolve(__dirname, "..");
const nodeVersion = assertSupportedNodeVersion(repositoryRoot);
const npmCommand = process.platform === "win32" ? "npm.cmd" : "npm";
const npmResult = spawnSync(npmCommand, ["--version"], { encoding: "utf8" });
if (npmResult.status !== 0) {
  throw new Error(
    `Could not read the npm version from ${npmCommand}: `
    + `${npmResult.error?.message || npmResult.stderr || npmResult.stdout || "unknown error"}`,
  );
}
const npmVersion = assertSupportedNpmVersion(repositoryRoot, npmResult.stdout.trim());
const npmRuntime = readNpmRuntimeMetadata(npmCommand, npmVersion);
const compatibility = assertNpmSupportsNodeVersion(
  nodeVersion,
  npmVersion,
  npmRuntime.engines?.node,
);

console.log(JSON.stringify({
  status: "passed",
  nodeVersion,
  npmVersion,
  supportedRanges: {
    node: supportedNodeVersionRange(repositoryRoot),
    npm: supportedNpmVersionRange(repositoryRoot),
  },
  runtimeCompatibility: {
    npmNode: compatibility.npmNodeRange,
  },
  sources: [
    "package.json#engines.node",
    "package.json#engines.npm",
    "installed npm package#engines.node",
  ],
}, null, 2));

function readNpmRuntimeMetadata(npmCommand, expectedVersion) {
  const manifests = [];
  if (process.env.npm_execpath) {
    manifests.push(...ancestorPackageManifests(process.env.npm_execpath));
  }

  const globalRootResult = spawnSync(npmCommand, ["root", "--global"], { encoding: "utf8" });
  if (globalRootResult.status === 0 && globalRootResult.stdout.trim()) {
    manifests.push(path.join(globalRootResult.stdout.trim(), "npm", "package.json"));
  }

  for (const manifestPath of new Set(manifests)) {
    try {
      const manifest = JSON.parse(fs.readFileSync(manifestPath, "utf8"));
      if (manifest.name === "npm" && manifest.version === expectedVersion) {
        return manifest;
      }
    } catch {
      // Try the next runtime metadata candidate.
    }
  }

  throw new Error(
    `Could not locate package metadata for the active npm ${expectedVersion}. `
    + "Run this check through the same npm executable used for packaging.",
  );
}

function ancestorPackageManifests(cliPath) {
  const manifests = [];
  let current = path.dirname(path.resolve(cliPath));
  while (true) {
    manifests.push(path.join(current, "package.json"));
    const parent = path.dirname(current);
    if (parent === current) {
      return manifests;
    }
    current = parent;
  }
}
