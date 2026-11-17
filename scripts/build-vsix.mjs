#!/usr/bin/env node
// Propagate one selected target through every packaging stage.
import path from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";
import { selectedPackageTarget } from "./lib/package-targets.mjs";
import { spawnNpm } from "./lib/npm.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const target = selectedPackageTarget();
const environment = { ...process.env, SAGE_VSIX_TARGET: target.name };
function check(result) {
  if (result.error) throw result.error;
  if (result.status !== 0) process.exit(result.status ?? 1);
}
for (const args of [
  ["run", "check:package-toolchain"],
  ["run", "build", "--workspace", "@sage-vscode/syntax-pack"],
  ["run", "build", "--workspace", "sage-vscode-extension"],
  ["run", "test:generated-assets"],
  ["run", "package:rust-binary"],
  ["run", "test:vsix-contents"],
]) check(spawnNpm(args, { cwd: root, env: environment, stdio: "inherit" }));
check(spawnSync(process.execPath, [path.join(root, "scripts/package-vsix.mjs"), ...process.argv.slice(2)], {
  cwd: root, env: environment, stdio: "inherit",
}));
