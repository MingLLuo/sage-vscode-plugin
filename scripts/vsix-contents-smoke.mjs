#!/usr/bin/env node
import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import {
  PACKAGE_TARGET_NAMES,
  selectedPackageTargetName,
  resolvePackageTarget,
  stagedPaths,
} from "./lib/package-targets.mjs";

// Verifies the extension package tree and the staged `sage-ls` for one VS Code
// target (`--target`, then SAGE_VSIX_TARGET, then the host platform).
const __dirname = path.dirname(fileURLToPath(import.meta.url));
const repositoryRoot = path.resolve(__dirname, "..");
const packageRoot = path.join(repositoryRoot, "packages", "extension-core");
const targetName = selectedPackageTargetName(process.argv.slice(2));
const packageTarget = PACKAGE_TARGET_NAMES.includes(targetName) ? resolvePackageTarget(targetName) : null;

const requiredFiles = [
  "package.json",
  "README.md",
  "CHANGELOG.md",
  "LICENSE",
  "out/src/extension.js",
  "resources/branding/icon.png",
  "resources/generated/syntax/language-configuration.json",
  "resources/generated/syntax/snippets/sagemath.json",
  "resources/generated/syntax/syntaxes/sagemath.tmLanguage.json",
  "resources/bin/README.md",
  "resources/walkthrough/install-sage.md",
  "resources/walkthrough/run-first-file.md",
  "resources/walkthrough/edit-and-explore.md",
  "resources/walkthrough/configure-workspace.md",
  "resources/walkthrough/troubleshoot.md",
  "resources/walkthrough/zh-cn/install-sage.md",
  "resources/walkthrough/zh-cn/run-first-file.md",
  "resources/walkthrough/zh-cn/edit-and-explore.md",
  "resources/walkthrough/zh-cn/configure-workspace.md",
  "resources/walkthrough/zh-cn/troubleshoot.md",
  "package.nls.json",
  "package.nls.zh-cn.json",
  "l10n/bundle.l10n.zh-cn.json",
];
const expectedIgnoredPatterns = [
  "src/**",
  "test/**",
  "test-host/**",
  "out/test/**",
  "out/test-host/**",
  "**/*.map",
];
const forbiddenRuntimeIgnorePatterns = [
  "out/src/**",
  "resources/**",
  "resources/bin/**",
  "package.json",
  "README.md",
  "CHANGELOG.md",
  "LICENSE",
];

const checks = [];
const manifest = JSON.parse(fs.readFileSync(path.join(packageRoot, "package.json"), "utf8"));
pushCheck(
  "VSIX target is a supported VS Code platform",
  Boolean(packageTarget),
  { target: targetName, supported: PACKAGE_TARGET_NAMES },
);
for (const relativePath of requiredFiles) {
  pushCheck(
    `required file ${relativePath}`,
    fs.existsSync(path.join(packageRoot, relativePath)),
    relativePath,
  );
}

const iconPath = path.join(packageRoot, manifest.icon ?? "");
pushCheck("manifest icon points at packaged icon", manifest.icon === "resources/branding/icon.png", manifest.icon);
if (fs.existsSync(iconPath)) {
  const dimensions = pngDimensions(iconPath);
  pushCheck("packaged icon is a 256x256 PNG", dimensions.width === 256 && dimensions.height === 256, dimensions);
}
pushCheck("gallery banner uses dark theme", manifest.galleryBanner?.theme === "dark", manifest.galleryBanner?.theme);
pushCheck("gallery banner has a hex color", /^#[0-9a-f]{6}$/i.test(manifest.galleryBanner?.color ?? ""), manifest.galleryBanner?.color);
pushCheck("manifest marks the extension as preview", manifest.preview === true, manifest.preview);
pushCheck("manifest disables marketplace Q&A without a configured support URL", manifest.qna === false, manifest.qna);
pushCheck("manifest runs the extension in the workspace host", manifest.extensionKind?.includes("workspace"), manifest.extensionKind);
pushCheck("manifest declares MIT license", manifest.license === "MIT", manifest.license);
pushCheck("manifest is not marked npm-private", manifest.private !== true, manifest.private);

const licenseText = fs.readFileSync(path.join(packageRoot, "LICENSE"), "utf8");
const changelogText = fs.readFileSync(path.join(packageRoot, "CHANGELOG.md"), "utf8");
pushCheck("packaged license contains MIT permission grant", /MIT License/.test(licenseText) && /Permission is hereby granted/.test(licenseText), "LICENSE");
pushCheck("packaged changelog records current package version", changelogText.includes(`## ${manifest.version}`), manifest.version);
pushCheck("packaged changelog marks preview posture", /preview/i.test(changelogText), "preview");

const ignoreText = fs.readFileSync(path.join(packageRoot, ".vscodeignore"), "utf8");
for (const pattern of expectedIgnoredPatterns) {
  pushCheck(`ignore excludes ${pattern}`, ignoreText.includes(pattern), pattern);
}
for (const pattern of forbiddenRuntimeIgnorePatterns) {
  pushCheck(`ignore does not exclude runtime ${pattern}`, !ignoreText.includes(pattern), pattern);
}

if (packageTarget) {
  checkStagedServer(packageTarget);
}

const failures = checks.filter((check) => !check.pass);
console.log(JSON.stringify({
  status: failures.length ? "failed" : "passed",
  packageRoot,
  target: targetName,
  rustTarget: packageTarget?.rustTarget ?? null,
  binaryDirectory: packageTarget?.binaryDirectory ?? null,
  checks,
}, null, 2));
if (failures.length) {
  process.exitCode = 1;
}

function checkStagedServer(target) {
  const staged = stagedPaths(repositoryRoot, target);
  const label = target.name;
  if (!fs.existsSync(staged.binary)) {
    pushCheck(
      `packaged binary exists for ${label}`,
      false,
      `run npm run package:rust-binary -- --target ${label} before packaging`,
    );
    return;
  }
  const hash = sha256(staged.binary);
  const hashText = fs.existsSync(staged.sha256) ? fs.readFileSync(staged.sha256, "utf8") : "";
  pushCheck(`packaged binary exists for ${label}`, true, relative(staged.binary));
  pushCheck(`packaged binary hash exists for ${label}`, fs.existsSync(staged.sha256), relative(staged.sha256));
  pushCheck(`packaged binary hash matches for ${label}`, hashText.includes(hash), hashText.trim());
  pushCheck(
    `packaged binary hash names ${target.executable}`,
    hashText.trim().endsWith(`  ${target.executable}`),
    hashText.trim(),
  );
  pushCheck(`packaged binary metadata exists for ${label}`, fs.existsSync(staged.metadata), relative(staged.metadata));
  if (fs.existsSync(staged.metadata)) {
    const metadata = JSON.parse(fs.readFileSync(staged.metadata, "utf8"));
    pushCheck(`packaged binary metadata platform ${target.platform}`, metadata.platform === target.platform, metadata.platform);
    pushCheck(`packaged binary metadata arch ${target.arch}`, metadata.arch === target.arch, metadata.arch);
    pushCheck(
      `packaged binary metadata Rust target ${target.rustTarget}`,
      metadata.rustTarget === target.rustTarget,
      metadata.rustTarget,
    );
    pushCheck("packaged binary metadata profile is release", metadata.profile === "release", metadata.profile);
    pushCheck("packaged binary metadata sha256", metadata.sha256 === hash, metadata.sha256);
  }
  const format = executableFormat(staged.binary);
  pushCheck(
    `packaged binary is a ${expectedExecutableFormat(target)} executable`,
    format === expectedExecutableFormat(target),
    format,
  );
  // Windows has no executable bit; the mode check only applies on POSIX hosts.
  const mode = fs.statSync(staged.binary).mode;
  pushCheck(
    `packaged binary executable for ${label}`,
    process.platform === "win32" || Boolean(mode & 0o111),
    mode.toString(8),
  );
  const noticesText = fs.existsSync(staged.notices) ? fs.readFileSync(staged.notices, "utf8") : "";
  pushCheck(`third-party notices are staged for ${label}`, noticesText.length > 0, relative(staged.notices));
  pushCheck(
    "third-party notices describe the packaged Rust target",
    noticesText.startsWith("THIRD-PARTY SOFTWARE NOTICES") && noticesText.includes(`Rust target: ${target.rustTarget}`),
    noticesText.slice(0, 80),
  );
  pushCheck(
    "third-party notices include crate license texts",
    /^CRATES \([1-9]\d*\)$/m.test(noticesText) && noticesText.includes("LICENSE TEXTS") && noticesText.includes("Used by:"),
    noticesText.match(/^CRATES \(\d+\)$/m)?.[0] ?? null,
  );
}

function expectedExecutableFormat(target) {
  return { darwin: "Mach-O", linux: "ELF", win32: "PE" }[target.platform];
}

function executableFormat(filePath) {
  const header = Buffer.alloc(4);
  const descriptor = fs.openSync(filePath, "r");
  try {
    fs.readSync(descriptor, header, 0, 4, 0);
  } finally {
    fs.closeSync(descriptor);
  }
  const magic = header.toString("hex");
  if (magic === "7f454c46") {
    return "ELF";
  }
  if (["cffaedfe", "cefaedfe", "feedfacf", "feedface", "cafebabe"].includes(magic)) {
    return "Mach-O";
  }
  if (magic.startsWith("4d5a")) {
    return "PE";
  }
  return `unknown (${magic})`;
}

function relative(filePath) {
  return path.relative(repositoryRoot, filePath).split(path.sep).join("/");
}

function pushCheck(name, pass, actual) {
  checks.push({
    name,
    pass: Boolean(pass),
    actual,
  });
}

function sha256(filePath) {
  return crypto.createHash("sha256").update(fs.readFileSync(filePath)).digest("hex");
}

function pngDimensions(filePath) {
  const buffer = fs.readFileSync(filePath);
  const signature = buffer.subarray(0, 8).toString("hex");
  if (signature !== "89504e470d0a1a0a") {
    return { width: 0, height: 0, error: "not a png" };
  }
  return {
    width: buffer.readUInt32BE(16),
    height: buffer.readUInt32BE(20),
  };
}
