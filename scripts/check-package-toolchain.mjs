#!/usr/bin/env node
import { spawnSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";
import {
  assertPinnedNodeVersion,
  assertPinnedNpmVersion,
} from "./package-toolchain.mjs";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const repositoryRoot = path.resolve(__dirname, "..");
const nodeVersion = assertPinnedNodeVersion(repositoryRoot);
const npmCommand = process.platform === "win32" ? "npm.cmd" : "npm";
const npmResult = spawnSync(npmCommand, ["--version"], { encoding: "utf8" });
if (npmResult.status !== 0) {
  throw new Error(
    `Could not read the npm version from ${npmCommand}: ${npmResult.stderr || npmResult.stdout}`,
  );
}
const npmVersion = assertPinnedNpmVersion(repositoryRoot, npmResult.stdout.trim());

console.log(JSON.stringify({
  status: "passed",
  nodeVersion,
  npmVersion,
  sources: [".node-version", "package.json#packageManager"],
}, null, 2));
