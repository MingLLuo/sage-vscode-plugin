import fs from "node:fs";
import path from "node:path";
import { spawnSync } from "node:child_process";

// One row per VS Code target platform this repository ships a VSIX for.
//
// `binaryDirectory` is what the extension looks up at runtime:
// `resources/bin/${process.platform}-${process.arch}/`. Alpine reports
// `process.platform === "linux"`, so the Alpine VSIX reuses the Linux directory
// and the same statically linked musl binary; only `TargetPlatform` differs.
const STATIC_CRT = ["-C", "target-feature=+crt-static"];

export const PACKAGE_TARGETS = Object.freeze({
  "darwin-arm64": target("darwin", "arm64", "aarch64-apple-darwin"),
  "darwin-x64": target("darwin", "x64", "x86_64-apple-darwin"),
  "linux-x64": target("linux", "x64", "x86_64-unknown-linux-musl", STATIC_CRT),
  "linux-arm64": target("linux", "arm64", "aarch64-unknown-linux-musl", STATIC_CRT),
  "alpine-x64": target("linux", "x64", "x86_64-unknown-linux-musl", STATIC_CRT),
  "alpine-arm64": target("linux", "arm64", "aarch64-unknown-linux-musl", STATIC_CRT),
  "win32-x64": target("win32", "x64", "x86_64-pc-windows-msvc", STATIC_CRT),
  "win32-arm64": target("win32", "arm64", "aarch64-pc-windows-msvc", STATIC_CRT),
});

export const PACKAGE_TARGET_NAMES = Object.freeze(Object.keys(PACKAGE_TARGETS));
export const SERVER_NAME = "sage-ls";
export const METADATA_FILE = "sage-ls.meta.json";
export const STAGED_NOTICES_FILE = "sage-ls.THIRD_PARTY_NOTICES.txt";
export const PACKAGED_NOTICES_FILE = "THIRD_PARTY_NOTICES.txt";

function target(platform, arch, rustTarget, rustFlags = []) {
  return Object.freeze({
    platform,
    arch,
    rustTarget,
    rustFlags: Object.freeze([...rustFlags]),
    binaryDirectory: `${platform}-${arch}`,
    executable: platform === "win32" ? `sage-ls.exe` : "sage-ls",
  });
}

export function normalizeArch(rawArch) {
  return rawArch === "amd64" ? "x64" : rawArch;
}

export function hostPackageTarget(platform = process.platform, arch = process.arch) {
  return `${platform}-${normalizeArch(arch)}`;
}

/**
 * Resolves the VS Code target to package: `--target <name>` wins, then
 * `SAGE_VSIX_TARGET`, then the host platform.
 */
export function selectedPackageTargetName(argv = process.argv.slice(2), environment = process.env) {
  const index = argv.indexOf("--target");
  if (index >= 0) {
    const value = argv[index + 1];
    if (!value || value.startsWith("--")) {
      throw new Error("missing value for --target");
    }
    return value;
  }
  const inline = argv.find((item) => item.startsWith("--target="));
  if (inline) {
    return inline.slice("--target=".length);
  }
  return environment.SAGE_VSIX_TARGET?.trim() || hostPackageTarget();
}

export function resolvePackageTarget(name) {
  const entry = Object.hasOwn(PACKAGE_TARGETS, name) ? PACKAGE_TARGETS[name] : undefined;
  if (!entry) {
    throw new Error(
      `unsupported package target: ${name}. Supported targets: ${PACKAGE_TARGET_NAMES.join(", ")}.`,
    );
  }
  return { name, ...entry };
}

export function selectedPackageTarget(argv, environment) {
  return resolvePackageTarget(selectedPackageTargetName(argv, environment));
}

export function vsixFileName(manifest, targetName) {
  return `${manifest.name}-${manifest.version}-${targetName}.vsix`;
}

/** Absolute paths of everything `package:rust-binary` stages for one target. */
export function stagedPaths(repositoryRoot, packageTarget) {
  const directory = path.join(
    repositoryRoot,
    "packages",
    "extension-core",
    "resources",
    "bin",
    packageTarget.binaryDirectory,
  );
  const binary = path.join(directory, packageTarget.executable);
  return {
    directory,
    binary,
    sha256: `${binary}.sha256`,
    metadata: path.join(directory, METADATA_FILE),
    notices: path.join(directory, STAGED_NOTICES_FILE),
  };
}

/** Paths of the same files relative to the extension package root, with `/` separators. */
export function stagedRelativePaths(packageTarget) {
  const directory = `resources/bin/${packageTarget.binaryDirectory}`;
  const binary = `${directory}/${packageTarget.executable}`;
  return {
    directory,
    binary,
    sha256: `${binary}.sha256`,
    metadata: `${directory}/${METADATA_FILE}`,
    notices: `${directory}/${STAGED_NOTICES_FILE}`,
  };
}

export function cargoReleaseBinaryPath(repositoryRoot, packageTarget, profile = "release") {
  return path.join(repositoryRoot, "target", packageTarget.rustTarget, profile, packageTarget.executable);
}

/**
 * Reports whether the Rust standard library for `rustTarget` is present in the
 * toolchain selected for `repositoryRoot` (honours `rust-toolchain.toml`).
 * Works with and without rustup.
 */
export function rustTargetStatus(repositoryRoot, rustTarget) {
  const result = spawnSync("rustc", ["--print", "sysroot"], {
    cwd: repositoryRoot,
    encoding: "utf8",
  });
  if (result.error || result.status !== 0) {
    return {
      installed: false,
      reason: `could not run rustc: ${result.error?.message || (result.stderr ?? "").trim() || "unknown error"}`,
      remedy: "Install the Rust toolchain pinned in rust-toolchain.toml (https://rustup.rs).",
    };
  }
  const sysroot = result.stdout.trim();
  const libraryDirectory = path.join(sysroot, "lib", "rustlib", rustTarget, "lib");
  if (fs.existsSync(libraryDirectory)) {
    return { installed: true, sysroot };
  }
  return {
    installed: false,
    sysroot,
    reason: `Rust target ${rustTarget} is not installed in ${sysroot}.`,
    remedy: `rustup target add ${rustTarget}`,
  };
}
