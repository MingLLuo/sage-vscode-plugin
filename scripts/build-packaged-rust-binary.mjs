#!/usr/bin/env node
import os from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { assertPinnedNodeVersion } from "./package-toolchain.mjs";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const repositoryRoot = path.resolve(__dirname, "..");
assertPinnedNodeVersion(repositoryRoot);

const environment = releaseBuildEnvironment();
run("cargo", ["build", "--locked", "--release", "-p", "sage-ls"], environment);
run(process.execPath, [path.join(__dirname, "package-rust-binary.mjs"), "--profile", "release"], environment);

function releaseBuildEnvironment() {
  const environment = { ...process.env };
  const inheritedFlags = encodedRustFlags(environment);
  const cargoHome = environment.CARGO_HOME
    ? path.resolve(environment.CARGO_HOME)
    : path.join(os.homedir(), ".cargo");
  const remaps = uniqueRemapPrefixes([
    [os.homedir(), "/build-home"],
    [cargoHome, "/cargo-home"],
    [repositoryRoot, "/workspace/sage-vscode-plugin"],
  ]).flatMap(([source, destination]) => ["--remap-path-prefix", `${source}=${destination}`]);

  delete environment.RUSTFLAGS;
  environment.CARGO_ENCODED_RUSTFLAGS = [...inheritedFlags, ...remaps].join("\x1f");
  return environment;
}

function encodedRustFlags(environment) {
  if (environment.CARGO_ENCODED_RUSTFLAGS) {
    return environment.CARGO_ENCODED_RUSTFLAGS.split("\x1f").filter(Boolean);
  }
  if (environment.RUSTFLAGS?.trim()) {
    return environment.RUSTFLAGS.trim().split(/\s+/);
  }
  return [];
}

function uniqueRemapPrefixes(entries) {
  const seen = new Set();
  return entries.filter(([source]) => {
    const normalized = path.resolve(source);
    if (seen.has(normalized)) {
      return false;
    }
    seen.add(normalized);
    return true;
  });
}

function run(command, args, environment) {
  const result = spawnSync(command, args, {
    cwd: repositoryRoot,
    env: environment,
    encoding: "utf8",
    stdio: "inherit",
  });
  if (result.error) {
    throw result.error;
  }
  if (result.status !== 0) {
    process.exit(result.status ?? 1);
  }
}
