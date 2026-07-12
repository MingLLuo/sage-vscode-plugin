#!/usr/bin/env node
import path from "node:path";
import { fileURLToPath } from "node:url";
import { assertPinnedNodeVersion } from "./package-toolchain.mjs";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const repositoryRoot = path.resolve(__dirname, "..");
const nodeVersion = assertPinnedNodeVersion(repositoryRoot);

console.log(JSON.stringify({
  status: "passed",
  nodeVersion,
  source: ".node-version",
}, null, 2));
