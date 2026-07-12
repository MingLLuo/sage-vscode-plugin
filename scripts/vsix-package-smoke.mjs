#!/usr/bin/env node
import assert from "node:assert/strict";
import crypto from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { inflateRawSync } from "node:zlib";
import { assertPinnedNodeVersion } from "./package-toolchain.mjs";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const repositoryRoot = path.resolve(__dirname, "..");
assertPinnedNodeVersion(repositoryRoot);
const packageRoot = path.join(repositoryRoot, "packages", "extension-core");
const manifest = JSON.parse(fs.readFileSync(path.join(packageRoot, "package.json"), "utf8"));
const CRC32_TABLE = buildCrc32Table();
const ZIP_METHOD_STORE = 0;
const ZIP_METHOD_DEFLATE = 8;
const MAX_VSIX_BYTES = 6 * 1024 * 1024;
const tempRoot = fs.mkdtempSync(path.join(os.tmpdir(), "sage-vsix-package-smoke-"));
stagePackagedRustBinary();
buildExtension(0o022);
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
buildExtension(0o077);
const restrictiveBuildMode = fs.statSync(path.join(packageRoot, "out", "src", "extension.js")).mode & 0o777;
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
const sourceDateZip = sourceDatePackageResult.status === 0 && fs.existsSync(sourceDateVsixPath)
  ? readZip(sourceDateVsixPath)
  : null;
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
  "repeat package rebuild uses a restrictive umask",
  restrictiveBuildMode === 0o600,
  restrictiveBuildMode.toString(8),
);
pushCheck(
  "SOURCE_DATE_EPOCH VSIX package succeeds",
  sourceDatePackageResult.status === 0 && fs.existsSync(sourceDateVsixPath),
  sourceDatePackageResult.status === 0 ? sourceDateVsixPath : sourceDatePackageResult.stderr || sourceDatePackageResult.stdout,
);
pushCheck(
  "SOURCE_DATE_EPOCH controls archive timestamp",
  sourceDatePackageOutput?.archiveTimestamp === new Date(Number(sourceDateEpoch) * 1000).toISOString()
    && sourceDateZip?.entries.every((entry) => {
      const expected = dosTimestamp(new Date(Number(sourceDateEpoch) * 1000));
      return entry.time === expected.time && entry.date === expected.date;
    }),
  {
    reported: sourceDatePackageOutput?.archiveTimestamp ?? sourceDatePackageResult.stdout,
    archiveEntryCount: sourceDateZip?.entries.length ?? 0,
  },
);
pushCheck(
  "VSIX stays within the release size budget",
  fs.statSync(vsixPath).size <= MAX_VSIX_BYTES,
  { size: fs.statSync(vsixPath).size, maxSize: MAX_VSIX_BYTES },
);
pushCheck("VSIX CRC validates every archive entry", zip.integrityFailures.length === 0, zip.integrityFailures.slice(0, 10));
const invalidArchiveModes = zip.entries
  .map((entry) => ({ name: entry.name, mode: entry.mode, expected: expectedArchiveMode(entry.name) }))
  .filter((entry) => entry.mode !== entry.expected);
pushCheck(
  "VSIX normalizes regular files to 0644 and sage-ls to 0755",
  invalidArchiveModes.length === 0,
  invalidArchiveModes.slice(0, 10),
);
const compressedEntries = zip.entries.filter((entry) => entry.compressionMethod === ZIP_METHOD_DEFLATE);
const compressedPayloadSize = zip.entries.reduce((total, entry) => total + entry.compressedSize, 0);
const uncompressedPayloadSize = zip.entries.reduce((total, entry) => total + entry.uncompressedSize, 0);
pushCheck("VSIX uses deterministic deflate compression", compressedEntries.length > 0, compressedEntries.length);
pushCheck(
  "VSIX compression materially reduces the runtime payload",
  compressedPayloadSize < uncompressedPayloadSize * 0.75,
  { compressedPayloadSize, uncompressedPayloadSize },
);
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
  const archiveBinary = zip.data(binaryEntry);
  const archiveBinaryHash = sha256Buffer(archiveBinary);
  const archiveHashText = zip.text(binaryHashEntry);
  pushCheck(
    "VSIX packaged Rust binary hash matches archive binary",
    archiveHashText.includes(archiveBinaryHash),
    archiveHashText.trim(),
  );
  const leakedMachinePaths = findBuildMachinePaths(archiveBinary);
  pushCheck(
    "VSIX packaged Rust binary excludes build-machine home and repository paths",
    leakedMachinePaths.length === 0,
    leakedMachinePaths,
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
pushCheck("VSIX contains jsonc-parser dependency", entries.has("extension/node_modules/jsonc-parser/package.json"), "jsonc-parser");
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

buildExtension(0o022);

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
  if (platform !== "darwin") {
    return `unsupported-non-macos-platform/${platform}-${arch}/sage-ls`;
  }
  return `extension/resources/bin/${platform}-${arch}/sage-ls`;
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
    const time = buffer.readUInt16LE(offset + 12);
    const date = buffer.readUInt16LE(offset + 14);
    const crc = buffer.readUInt32LE(offset + 16);
    const compressionMethod = buffer.readUInt16LE(offset + 10);
    const compressedSize = buffer.readUInt32LE(offset + 20);
    const uncompressedSize = buffer.readUInt32LE(offset + 24);
    const fileNameLength = buffer.readUInt16LE(offset + 28);
    const extraLength = buffer.readUInt16LE(offset + 30);
    const commentLength = buffer.readUInt16LE(offset + 32);
    const externalAttributes = buffer.readUInt32LE(offset + 38);
    const localOffset = buffer.readUInt32LE(offset + 42);
    const name = buffer.subarray(offset + 46, offset + 46 + fileNameLength).toString("utf8");
    entries.push({
      name,
      time,
      date,
      crc,
      compressionMethod,
      compressedSize,
      uncompressedSize,
      mode: externalAttributes >>> 16,
      localOffset,
    });
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
      return readEntryData(buffer, entry).toString("utf8");
    },
    data(name) {
      const entry = entries.find((candidate) => candidate.name === name);
      assert.ok(entry, `missing zip entry ${name}`);
      return readEntryData(buffer, entry);
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
  const localCompressionMethod = buffer.readUInt16LE(local + 8);
  const localTime = buffer.readUInt16LE(local + 10);
  const localDate = buffer.readUInt16LE(local + 12);
  const localCompressedSize = buffer.readUInt32LE(local + 18);
  const localUncompressedSize = buffer.readUInt32LE(local + 22);
  const fileNameLength = buffer.readUInt16LE(local + 26);
  const extraLength = buffer.readUInt16LE(local + 28);
  const localName = buffer.subarray(local + 30, local + 30 + fileNameLength).toString("utf8");
  const dataStart = local + 30 + fileNameLength + extraLength;
  const payload = buffer.subarray(dataStart, dataStart + entry.compressedSize);
  if (localName !== entry.name) {
    failures.push(`${entry.name}: local name mismatch ${localName}`);
  }
  if (localCompressionMethod !== entry.compressionMethod) {
    failures.push(`${entry.name}: compression method mismatch`);
  }
  if (localTime !== entry.time || localDate !== entry.date) {
    failures.push(`${entry.name}: local timestamp mismatch`);
  }
  if (![ZIP_METHOD_STORE, ZIP_METHOD_DEFLATE].includes(entry.compressionMethod)) {
    failures.push(`${entry.name}: unsupported compression method ${entry.compressionMethod}`);
    return failures;
  }
  let data;
  try {
    data = entry.compressionMethod === ZIP_METHOD_DEFLATE ? inflateRawSync(payload) : payload;
  } catch (error) {
    failures.push(`${entry.name}: decompression failed: ${String(error)}`);
    return failures;
  }
  if (localCrc !== entry.crc || crc32(data) !== entry.crc) {
    failures.push(`${entry.name}: crc mismatch`);
  }
  if (localCompressedSize !== entry.compressedSize || localUncompressedSize !== entry.uncompressedSize) {
    failures.push(`${entry.name}: local size mismatch`);
  }
  if (data.length !== entry.uncompressedSize) {
    failures.push(`${entry.name}: uncompressed size mismatch`);
  }
  return failures;
}

function readEntryData(buffer, entry) {
  const local = entry.localOffset;
  assert.equal(buffer.readUInt32LE(local), 0x04034b50);
  const fileNameLength = buffer.readUInt16LE(local + 26);
  const extraLength = buffer.readUInt16LE(local + 28);
  const dataStart = local + 30 + fileNameLength + extraLength;
  const payload = buffer.subarray(dataStart, dataStart + entry.compressedSize);
  if (entry.compressionMethod === ZIP_METHOD_STORE) {
    return payload;
  }
  assert.equal(entry.compressionMethod, ZIP_METHOD_DEFLATE);
  return inflateRawSync(payload);
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

function buildExtension(creationMask) {
  const npmCommand = process.platform === "win32" ? "npm.cmd" : "npm";
  const previousMask = process.umask(creationMask);
  let result;
  try {
    result = spawnSync(npmCommand, ["run", "build", "--workspace", "sage-vscode-extension"], {
      cwd: repositoryRoot,
      encoding: "utf8",
    });
  } finally {
    process.umask(previousMask);
  }
  if (result.status !== 0) {
    process.stderr.write(result.stdout);
    process.stderr.write(result.stderr);
    process.exit(result.status ?? 1);
  }
}

function expectedArchiveMode(entryName) {
  return /^extension\/resources\/bin\/[^/]+\/sage-ls$/.test(entryName)
    ? 0o100755
    : 0o100644;
}

function findBuildMachinePaths(binary) {
  const text = binary.toString("latin1");
  const candidates = [
    repositoryRoot,
    os.homedir(),
    process.env.CARGO_HOME ? path.resolve(process.env.CARGO_HOME) : null,
    "/Users/",
    "/home/",
    "\\Users\\",
  ].filter(Boolean);
  return [...new Set(candidates.filter((candidate) => text.includes(candidate)))];
}

function stagePackagedRustBinary() {
  const npmCommand = process.platform === "win32" ? "npm.cmd" : "npm";
  const result = spawnSync(npmCommand, ["run", "package:rust-binary"], {
    cwd: repositoryRoot,
    encoding: "utf8",
  });
  if (result.status !== 0) {
    process.stderr.write(result.stdout);
    process.stderr.write(result.stderr);
    process.exit(result.status ?? 1);
  }
}

function dosTimestamp(value) {
  const year = Math.max(1980, value.getUTCFullYear());
  const date = ((year - 1980) << 9) | ((value.getUTCMonth() + 1) << 5) | value.getUTCDate();
  const time = (value.getUTCHours() << 11) | (value.getUTCMinutes() << 5) | Math.floor(value.getUTCSeconds() / 2);
  return { date, time };
}

function normalizeArch(rawArch) {
  return rawArch === "amd64" ? "x64" : rawArch;
}
