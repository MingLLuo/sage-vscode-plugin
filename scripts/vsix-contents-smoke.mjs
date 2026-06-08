#!/usr/bin/env node
import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const repositoryRoot = path.resolve(__dirname, "..");
const packageRoot = path.join(repositoryRoot, "packages", "extension-core");
const platform = process.env.SAGE_VSIX_PLATFORM ?? process.platform;
const arch = normalizeArch(process.env.SAGE_VSIX_ARCH ?? process.arch);
const binaryDirectory = `${platform}-${arch}`;
const binaryName = "sage-ls";

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
  "resources/walkthrough/select-interpreter.md",
  "resources/walkthrough/configure-workspace.md",
  "resources/walkthrough/inspect-status.md",
  "resources/walkthrough/validate-edit-loop.md",
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
pushCheck("VSIX binary target is macOS", platform === "darwin", binaryDirectory);
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

const binaryRelativePath = path.join("resources", "bin", binaryDirectory, binaryName);
const binaryPath = path.join(packageRoot, binaryRelativePath);
if (fs.existsSync(binaryPath)) {
  const hashPath = `${binaryPath}.sha256`;
  const metadataPath = path.join(path.dirname(binaryPath), "sage-ls.meta.json");
  const hash = sha256(binaryPath);
  const hashText = fs.existsSync(hashPath) ? fs.readFileSync(hashPath, "utf8") : "";
  pushCheck(`packaged binary hash exists for ${binaryDirectory}`, fs.existsSync(hashPath), hashPath);
  pushCheck(`packaged binary hash matches for ${binaryDirectory}`, hashText.includes(hash), hashText.trim());
  pushCheck(`packaged binary metadata exists for ${binaryDirectory}`, fs.existsSync(metadataPath), metadataPath);
  if (fs.existsSync(metadataPath)) {
    const metadata = JSON.parse(fs.readFileSync(metadataPath, "utf8"));
    pushCheck(`packaged binary metadata platform ${platform}`, metadata.platform === platform, metadata.platform);
    pushCheck(`packaged binary metadata arch ${arch}`, metadata.arch === arch, metadata.arch);
    pushCheck(`packaged binary metadata sha256`, metadata.sha256 === hash, metadata.sha256);
  }
  const mode = fs.statSync(binaryPath).mode;
  pushCheck(`packaged binary executable for ${binaryDirectory}`, Boolean(mode & 0o111), mode.toString(8));
} else {
  pushCheck(
    `packaged macOS binary exists for ${binaryDirectory}`,
    false,
    "run npm run package:rust-binary before packaging",
  );
}

const failures = checks.filter((check) => !check.pass);
console.log(JSON.stringify({
  status: failures.length ? "failed" : "passed",
  packageRoot,
  binaryDirectory,
  checks,
}, null, 2));
if (failures.length) {
  process.exitCode = 1;
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

function normalizeArch(rawArch) {
  if (rawArch === "amd64") {
    return "x64";
  }
  return rawArch;
}
