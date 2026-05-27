#!/usr/bin/env node
import assert from "node:assert/strict";
import crypto from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const repositoryRoot = path.resolve(__dirname, "..");
const packageRoot = path.join(repositoryRoot, "packages", "extension-core");
const manifest = JSON.parse(fs.readFileSync(path.join(packageRoot, "package.json"), "utf8"));
const CRC32_TABLE = buildCrc32Table();
const tempRoot = fs.mkdtempSync(path.join(os.tmpdir(), "sage-vsix-package-smoke-"));
const packageResult = spawnSync(
  process.execPath,
  [path.join(repositoryRoot, "scripts", "package-vsix.mjs"), "--out-dir", tempRoot],
  {
    cwd: repositoryRoot,
    encoding: "utf8",
  },
);

if (packageResult.status !== 0) {
  process.stderr.write(packageResult.stdout);
  process.stderr.write(packageResult.stderr);
  process.exit(packageResult.status ?? 1);
}

const vsixPath = path.join(tempRoot, `${manifest.name}-${manifest.version}.vsix`);
const repeatTempRoot = fs.mkdtempSync(path.join(os.tmpdir(), "sage-vsix-package-repeat-smoke-"));
const repeatPackageResult = spawnSync(
  process.execPath,
  [path.join(repositoryRoot, "scripts", "package-vsix.mjs"), "--out-dir", repeatTempRoot],
  {
    cwd: repositoryRoot,
    encoding: "utf8",
  },
);
const repeatVsixPath = path.join(repeatTempRoot, `${manifest.name}-${manifest.version}.vsix`);
const sourceDateEpoch = "1700000000";
const sourceDateTempRoot = fs.mkdtempSync(path.join(os.tmpdir(), "sage-vsix-package-source-date-smoke-"));
const sourceDatePackageResult = spawnSync(
  process.execPath,
  [path.join(repositoryRoot, "scripts", "package-vsix.mjs"), "--out-dir", sourceDateTempRoot],
  {
    cwd: repositoryRoot,
    encoding: "utf8",
    env: {
      ...process.env,
      SOURCE_DATE_EPOCH: sourceDateEpoch,
    },
  },
);
const sourceDateVsixPath = path.join(sourceDateTempRoot, `${manifest.name}-${manifest.version}.vsix`);
const sourceDatePackageOutput = parseJsonOutput(sourceDatePackageResult.stdout);
const zip = readZip(vsixPath);
const entries = new Set(zip.entries.map((entry) => entry.name));
const checks = [];

pushCheck("VSIX file exists", fs.existsSync(vsixPath), vsixPath);
pushCheck(
  "repeat VSIX package succeeds",
  repeatPackageResult.status === 0 && fs.existsSync(repeatVsixPath),
  repeatPackageResult.status === 0 ? repeatVsixPath : repeatPackageResult.stderr || repeatPackageResult.stdout,
);
pushCheck(
  "VSIX package is reproducible across repeated runs",
  repeatPackageResult.status === 0 && fs.existsSync(repeatVsixPath) && sha256(vsixPath) === sha256(repeatVsixPath),
  repeatPackageResult.status === 0 && fs.existsSync(repeatVsixPath)
    ? { first: sha256(vsixPath), second: sha256(repeatVsixPath) }
    : null,
);
pushCheck(
  "SOURCE_DATE_EPOCH VSIX package succeeds",
  sourceDatePackageResult.status === 0 && fs.existsSync(sourceDateVsixPath),
  sourceDatePackageResult.status === 0 ? sourceDateVsixPath : sourceDatePackageResult.stderr || sourceDatePackageResult.stdout,
);
pushCheck(
  "SOURCE_DATE_EPOCH controls archive timestamp",
  sourceDatePackageOutput?.archiveTimestamp === new Date(Number(sourceDateEpoch) * 1000).toISOString(),
  sourceDatePackageOutput?.archiveTimestamp ?? sourceDatePackageResult.stdout,
);
pushCheck("VSIX CRC validates every archive entry", zip.integrityFailures.length === 0, zip.integrityFailures.slice(0, 10));
pushCheck("VSIX contains content types", entries.has("[Content_Types].xml"), "[Content_Types].xml");
pushCheck("VSIX contains extension manifest", entries.has("extension.vsixmanifest"), "extension.vsixmanifest");
pushCheck("VSIX contains package manifest", entries.has("extension/package.json"), "extension/package.json");
pushCheck("VSIX contains compiled extension entrypoint", entries.has("extension/out/src/extension.js"), "extension/out/src/extension.js");
pushCheck("VSIX contains generated Sage grammar", entries.has("extension/resources/generated/syntax/syntaxes/sagemath.tmLanguage.json"), "syntax");
const binaryEntry = platformBinaryEntry();
const binaryHashEntry = `${binaryEntry}.sha256`;
const binaryMetadataEntry = `${path.posix.dirname(binaryEntry)}/sage-ls.meta.json`;
pushCheck("VSIX contains packaged Rust binary", entries.has(binaryEntry), binaryEntry);
pushCheck("VSIX contains packaged Rust binary hash", entries.has(binaryHashEntry), binaryHashEntry);
pushCheck("VSIX contains packaged Rust binary metadata", entries.has(binaryMetadataEntry), binaryMetadataEntry);
if (entries.has(binaryEntry) && entries.has(binaryHashEntry)) {
  const archiveBinaryHash = sha256Buffer(zip.data(binaryEntry));
  const archiveHashText = zip.text(binaryHashEntry);
  pushCheck(
    "VSIX packaged Rust binary hash matches archive binary",
    archiveHashText.includes(archiveBinaryHash),
    archiveHashText.trim(),
  );
}
if (entries.has(binaryEntry) && entries.has(binaryMetadataEntry)) {
  const archiveBinaryHash = sha256Buffer(zip.data(binaryEntry));
  const metadata = JSON.parse(zip.text(binaryMetadataEntry));
  const expectedPlatform = process.env.SAGE_VSIX_PLATFORM ?? process.platform;
  const expectedArch = normalizeArch(process.env.SAGE_VSIX_ARCH ?? process.arch);
  pushCheck("VSIX packaged Rust metadata platform matches", metadata.platform === expectedPlatform, metadata.platform);
  pushCheck("VSIX packaged Rust metadata arch matches", metadata.arch === expectedArch, metadata.arch);
  pushCheck("VSIX packaged Rust metadata profile is release", metadata.profile === "release", metadata.profile);
  pushCheck("VSIX packaged Rust metadata hash matches archive binary", metadata.sha256 === archiveBinaryHash, metadata.sha256);
}
pushCheck("VSIX contains vscode-languageclient dependency", entries.has("extension/node_modules/vscode-languageclient/package.json"), "vscode-languageclient");
pushCheck("VSIX contains vscode-jsonrpc dependency", entries.has("extension/node_modules/vscode-jsonrpc/package.json"), "vscode-jsonrpc");
pushCheck("VSIX excludes TypeScript source", !entries.has("extension/src/extension.ts"), "extension/src/extension.ts");
pushCheck("VSIX excludes extension tests", ![...entries].some((entry) => entry.startsWith("extension/test/")), "extension/test/");
pushCheck("VSIX excludes extension-host tests", ![...entries].some((entry) => entry.startsWith("extension/test-host/")), "extension/test-host/");
pushCheck("VSIX excludes source maps", ![...entries].some((entry) => entry.endsWith(".map")), "*.map");

const packagedManifest = JSON.parse(zip.text("extension/package.json"));
pushCheck("packaged manifest id matches", packagedManifest.name === manifest.name, packagedManifest.name);
pushCheck("packaged manifest publisher matches", packagedManifest.publisher === manifest.publisher, packagedManifest.publisher);
pushCheck("packaged manifest version matches", packagedManifest.version === manifest.version, packagedManifest.version);
const vsixManifest = zip.text("extension.vsixmanifest");
pushCheck("VSIX identity includes extension name", vsixManifest.includes(`Id="${manifest.name}"`), manifest.name);
pushCheck("VSIX identity includes publisher", vsixManifest.includes(`Publisher="${manifest.publisher}"`), manifest.publisher);
pushCheck("VSIX marks preview gallery flag", vsixManifest.includes("<GalleryFlags>Preview</GalleryFlags>"), "Preview");
pushCheck(
  "VSIX exposes README as details asset",
  vsixManifest.includes('Type="Microsoft.VisualStudio.Services.Content.Details"')
    && vsixManifest.includes('Path="extension/README.md"'),
  "extension/README.md",
);
const contentTypes = parseContentTypes(zip.text("[Content_Types].xml"));
const missingContentTypes = zip.entries
  .map((entry) => entry.name)
  .filter((entry) => entry !== "[Content_Types].xml")
  .filter((entry) => !hasContentType(entry, contentTypes));
pushCheck(
  "VSIX content types cover every archive entry",
  missingContentTypes.length === 0,
  missingContentTypes.slice(0, 10),
);

const failures = checks.filter((check) => !check.pass);
console.log(JSON.stringify({
  status: failures.length ? "failed" : "passed",
  vsix: vsixPath,
  entryCount: entries.size,
  checks,
}, null, 2));
if (failures.length) {
  process.exitCode = 1;
}

function platformBinaryEntry() {
  const platform = process.env.SAGE_VSIX_PLATFORM ?? process.platform;
  const arch = normalizeArch(process.env.SAGE_VSIX_ARCH ?? process.arch);
  const binaryName = platform === "win32" ? "sage-ls.exe" : "sage-ls";
  return `extension/resources/bin/${platform}-${arch}/${binaryName}`;
}

function readZip(filePath) {
  const buffer = fs.readFileSync(filePath);
  const endOffset = findEndOfCentralDirectory(buffer);
  const entryCount = buffer.readUInt16LE(endOffset + 10);
  const centralOffset = buffer.readUInt32LE(endOffset + 16);
  const entries = [];
  const integrityFailures = [];
  let offset = centralOffset;
  for (let index = 0; index < entryCount; index += 1) {
    assert.equal(buffer.readUInt32LE(offset), 0x02014b50);
    const crc = buffer.readUInt32LE(offset + 16);
    const compressedSize = buffer.readUInt32LE(offset + 20);
    const uncompressedSize = buffer.readUInt32LE(offset + 24);
    const fileNameLength = buffer.readUInt16LE(offset + 28);
    const extraLength = buffer.readUInt16LE(offset + 30);
    const commentLength = buffer.readUInt16LE(offset + 32);
    const localOffset = buffer.readUInt32LE(offset + 42);
    const name = buffer.subarray(offset + 46, offset + 46 + fileNameLength).toString("utf8");
    entries.push({ name, crc, compressedSize, uncompressedSize, localOffset });
    offset += 46 + fileNameLength + extraLength + commentLength;
  }
  for (const entry of entries) {
    integrityFailures.push(...validateEntry(buffer, entry));
  }
  return {
    entries,
    integrityFailures,
    text(name) {
      const entry = entries.find((candidate) => candidate.name === name);
      assert.ok(entry, `missing zip entry ${name}`);
      const local = entry.localOffset;
      assert.equal(buffer.readUInt32LE(local), 0x04034b50);
      const fileNameLength = buffer.readUInt16LE(local + 26);
      const extraLength = buffer.readUInt16LE(local + 28);
      const dataStart = local + 30 + fileNameLength + extraLength;
      return buffer.subarray(dataStart, dataStart + entry.compressedSize).toString("utf8");
    },
    data(name) {
      const entry = entries.find((candidate) => candidate.name === name);
      assert.ok(entry, `missing zip entry ${name}`);
      const local = entry.localOffset;
      assert.equal(buffer.readUInt32LE(local), 0x04034b50);
      const fileNameLength = buffer.readUInt16LE(local + 26);
      const extraLength = buffer.readUInt16LE(local + 28);
      const dataStart = local + 30 + fileNameLength + extraLength;
      return buffer.subarray(dataStart, dataStart + entry.compressedSize);
    },
  };
}

function validateEntry(buffer, entry) {
  const failures = [];
  const local = entry.localOffset;
  if (buffer.readUInt32LE(local) !== 0x04034b50) {
    return [`${entry.name}: invalid local header`];
  }
  const localCrc = buffer.readUInt32LE(local + 14);
  const localCompressedSize = buffer.readUInt32LE(local + 18);
  const localUncompressedSize = buffer.readUInt32LE(local + 22);
  const fileNameLength = buffer.readUInt16LE(local + 26);
  const extraLength = buffer.readUInt16LE(local + 28);
  const localName = buffer.subarray(local + 30, local + 30 + fileNameLength).toString("utf8");
  const dataStart = local + 30 + fileNameLength + extraLength;
  const data = buffer.subarray(dataStart, dataStart + entry.compressedSize);
  if (localName !== entry.name) {
    failures.push(`${entry.name}: local name mismatch ${localName}`);
  }
  if (localCrc !== entry.crc || crc32(data) !== entry.crc) {
    failures.push(`${entry.name}: crc mismatch`);
  }
  if (localCompressedSize !== entry.compressedSize || localUncompressedSize !== entry.uncompressedSize) {
    failures.push(`${entry.name}: local size mismatch`);
  }
  if (entry.compressedSize !== entry.uncompressedSize) {
    failures.push(`${entry.name}: unexpected compressed entry`);
  }
  return failures;
}

function findEndOfCentralDirectory(buffer) {
  for (let offset = buffer.length - 22; offset >= 0; offset -= 1) {
    if (buffer.readUInt32LE(offset) === 0x06054b50) {
      return offset;
    }
  }
  throw new Error("end of central directory not found");
}

function pushCheck(name, pass, actual) {
  checks.push({ name, pass: Boolean(pass), actual });
}

function sha256(filePath) {
  return crypto.createHash("sha256").update(fs.readFileSync(filePath)).digest("hex");
}

function sha256Buffer(buffer) {
  return crypto.createHash("sha256").update(buffer).digest("hex");
}

function parseJsonOutput(stdout) {
  try {
    return JSON.parse(stdout);
  } catch {
    return null;
  }
}

function parseContentTypes(xml) {
  return {
    defaults: new Set([...xml.matchAll(/<Default\s+Extension="([^"]+)"/g)].map((match) => match[1])),
    overrides: new Set([...xml.matchAll(/<Override\s+PartName="\/([^"]+)"/g)].map((match) => match[1])),
  };
}

function hasContentType(entry, contentTypes) {
  const extension = path.posix.extname(entry).slice(1).toLowerCase();
  return extension ? contentTypes.defaults.has(extension) : contentTypes.overrides.has(entry);
}

function crc32(buffer) {
  let crc = 0xffffffff;
  for (const byte of buffer) {
    crc = CRC32_TABLE[(crc ^ byte) & 0xff] ^ (crc >>> 8);
  }
  return (crc ^ 0xffffffff) >>> 0;
}

function buildCrc32Table() {
  const table = new Uint32Array(256);
  for (let index = 0; index < 256; index += 1) {
    let value = index;
    for (let bit = 0; bit < 8; bit += 1) {
      value = value & 1 ? 0xedb88320 ^ (value >>> 1) : value >>> 1;
    }
    table[index] = value >>> 0;
  }
  return table;
}

function normalizeArch(rawArch) {
  return rawArch === "amd64" ? "x64" : rawArch;
}
