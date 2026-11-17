#!/usr/bin/env node
import os from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { assertSupportedNodeVersion } from "./package-toolchain.mjs";
import { rustTargetStatus, selectedPackageTarget } from "./lib/package-targets.mjs";

// Builds the release `sage-ls` for one VS Code target and stages it, with its
// hash, metadata, and third-party notices, for `package-vsix.mjs`.
//
// Usage: node scripts/build-packaged-rust-binary.mjs [--target <vscode-target>]
// The target defaults to SAGE_VSIX_TARGET, then to the host platform.
const __dirname = path.dirname(fileURLToPath(import.meta.url));
const repositoryRoot = path.resolve(__dirname, "..");
assertSupportedNodeVersion(repositoryRoot);

const target = packageTarget();
assertRustTargetInstalled(target);

const environment = releaseBuildEnvironment(target);
run(
  "cargo",
  ["build", "--locked", "--release", "-p", "sage-ls", "--target", target.rustTarget],
  environment,
);
run(
  process.execPath,
  [path.join(__dirname, "package-rust-binary.mjs"), "--profile", "release", "--target", target.name],
  environment,
);
run(
  process.execPath,
  [path.join(__dirname, "generate-third-party-notices.mjs"), "--target", target.name],
  process.env,
);

function packageTarget() {
  try {
    return selectedPackageTarget(process.argv.slice(2));
  } catch (error) {
    console.error(error.message);
    return process.exit(1);
  }
}

function assertRustTargetInstalled(packageTarget) {
  const status = rustTargetStatus(repositoryRoot, packageTarget.rustTarget);
  if (status.installed) {
    return;
  }
  console.error(
    `Cannot build sage-ls for ${packageTarget.name}: ${status.reason}\n`
    + `Install it with: ${status.remedy}`
    + crossToolchainHint(packageTarget),
  );
  process.exit(1);
}

function crossToolchainHint(packageTarget) {
  if (packageTarget.rustTarget.endsWith("-linux-musl")) {
    return "\nThe musl targets also need a musl C toolchain for the bundled SQLite and tree-sitter sources "
      + "(for example `apt-get install musl-tools` on a runner of the same architecture).";
  }
  if (packageTarget.platform === "win32" && process.platform !== "win32") {
    return "\nWindows targets use the MSVC toolchain and are built on a Windows host.";
  }
  return "";
}

function releaseBuildEnvironment(packageTarget) {
  const environment = { ...process.env };
  if (packageTarget.rustTarget.endsWith("-linux-musl")) {
    const targetKey = packageTarget.rustTarget.replaceAll("-", "_");
    environment[`CC_${targetKey}`] ??= "musl-gcc";
    environment[`CARGO_TARGET_${targetKey.toUpperCase()}_LINKER`] ??= "musl-gcc";
  }
  const inheritedFlags = encodedRustFlags(environment);
  const cargoHome = environment.CARGO_HOME
    ? path.resolve(environment.CARGO_HOME)
    : path.join(os.homedir(), ".cargo");
  const remaps = uniqueRemapPrefixes([
    [os.homedir(), "/build-home"],
    [cargoHome, "/cargo-home"],
    [repositoryRoot, "/workspace/sage-vscode-plugin"],
  ]).flatMap(([source, destination]) => ["--remap-path-prefix", `${source}=${destination}`]);

  // Cargo applies these flags to the target only (not to build scripts or
  // proc macros) because the build always passes an explicit --target, which
  // is what `+crt-static` requires.
  delete environment.RUSTFLAGS;
  environment.CARGO_ENCODED_RUSTFLAGS = [
    ...inheritedFlags,
    ...packageTarget.rustFlags,
    ...remaps,
  ].join("\x1f");
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
