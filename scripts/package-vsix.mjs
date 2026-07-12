#!/usr/bin/env node
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { deflateRawSync } from "node:zlib";
import { assertPinnedNodeVersion } from "./package-toolchain.mjs";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const repositoryRoot = path.resolve(__dirname, "..");
assertPinnedNodeVersion(repositoryRoot);
const packageRoot = path.join(repositoryRoot, "packages", "extension-core");
const outDir = path.resolve(argumentValue("--out-dir") ?? path.join(repositoryRoot, "dist"));
const manifest = JSON.parse(fs.readFileSync(path.join(packageRoot, "package.json"), "utf8"));
const vsixName = `${manifest.name}-${manifest.version}.vsix`;
const vsixPath = path.join(outDir, vsixName);
const CRC32_TABLE = buildCrc32Table();
const ARCHIVE_TIMESTAMP = archiveTimestampDate();
const ZIP_METHOD_STORE = 0;
const ZIP_METHOD_DEFLATE = 8;
const MAX_VSIX_BYTES = 6 * 1024 * 1024;

fs.mkdirSync(outDir, { recursive: true });

const runtimeEntries = [];

for (const file of collectExtensionFiles(packageRoot)) {
  const archivePath = slash(path.join("extension", file.relativePath));
  const data = fs.readFileSync(file.absolutePath);
  if (isPackagedSageBinary(archivePath)) {
    assertNoBuildMachinePaths(data, archivePath);
  }
  runtimeEntries.push({
    path: archivePath,
    data,
    mode: archiveMode(archivePath),
  });
}

for (const dependency of productionDependencyClosure(manifest)) {
  const dependencyRoot = path.join(repositoryRoot, "node_modules", dependency);
  for (const file of collectDependencyFiles(dependencyRoot)) {
    const archivePath = slash(path.join("extension", "node_modules", dependency, file.relativePath));
    runtimeEntries.push({
      path: archivePath,
      data: fs.readFileSync(file.absolutePath),
      mode: archiveMode(archivePath),
    });
  }
}

const manifestEntry = {
  path: "extension.vsixmanifest",
  data: Buffer.from(vsixManifestXml(manifest), "utf8"),
  mode: 0o100644,
};
const entries = [
  {
    path: "[Content_Types].xml",
    data: Buffer.from(contentTypesXml([manifestEntry, ...runtimeEntries]), "utf8"),
    mode: 0o100644,
  },
  manifestEntry,
  ...runtimeEntries,
];

const archiveStats = writeZip(vsixPath, entries.sort((left, right) => left.path.localeCompare(right.path)));
const vsixSize = fs.statSync(vsixPath).size;
if (vsixSize > MAX_VSIX_BYTES) {
  throw new Error(`VSIX size ${vsixSize} exceeds the ${MAX_VSIX_BYTES}-byte release budget`);
}
console.log(JSON.stringify({
  status: "packaged",
  vsix: vsixPath,
  entries: entries.length,
  size: vsixSize,
  maxSize: MAX_VSIX_BYTES,
  compressedPayloadSize: archiveStats.compressedPayloadSize,
  uncompressedPayloadSize: archiveStats.uncompressedPayloadSize,
  compressionRatio: Number(
    (archiveStats.compressedPayloadSize / Math.max(1, archiveStats.uncompressedPayloadSize)).toFixed(4),
  ),
  archiveTimestamp: ARCHIVE_TIMESTAMP.toISOString(),
}, null, 2));

function argumentValue(name) {
  const index = process.argv.indexOf(name);
  return index >= 0 ? process.argv[index + 1] : undefined;
}

function collectExtensionFiles(root) {
  const files = [];
  walk(root, "", (absolutePath, relativePath) => {
    if (isIgnoredExtensionFile(relativePath)) {
      return;
    }
    files.push({ absolutePath, relativePath });
  });
  return files;
}

function collectDependencyFiles(root) {
  if (!fs.existsSync(root)) {
    throw new Error(`Missing production dependency at ${root}`);
  }
  const files = [];
  walk(root, "", (absolutePath, relativePath) => {
    if (
      relativePath === ".package-lock.json"
      || relativePath.startsWith(".bin/")
      || relativePath.endsWith(".map")
    ) {
      return;
    }
    files.push({ absolutePath, relativePath });
  });
  return files;
}

function walk(root, prefix, onFile) {
  const directory = path.join(root, prefix);
  for (const entry of fs.readdirSync(directory, { withFileTypes: true })) {
    const relativePath = slash(path.join(prefix, entry.name));
    const absolutePath = path.join(root, relativePath);
    if (entry.isDirectory()) {
      walk(root, relativePath, onFile);
    } else if (entry.isFile()) {
      onFile(absolutePath, relativePath);
    }
  }
}

function isIgnoredExtensionFile(relativePath) {
  return relativePath === "tsconfig.json"
    || relativePath.endsWith(".tsbuildinfo")
    || relativePath.endsWith(".map")
    || relativePath.startsWith("src/")
    || relativePath.startsWith("test/")
    || relativePath.startsWith("test-host/")
    || relativePath.startsWith("out/test/")
    || relativePath.startsWith("out/test-host/")
    || relativePath.startsWith("node_modules/");
}

function productionDependencyClosure(extensionManifest) {
  const seen = new Set();
  const queue = Object.keys(extensionManifest.dependencies ?? {});
  for (let index = 0; index < queue.length; index += 1) {
    const dependency = queue[index];
    if (seen.has(dependency)) {
      continue;
    }
    const packageJsonPath = path.join(repositoryRoot, "node_modules", dependency, "package.json");
    if (!fs.existsSync(packageJsonPath)) {
      throw new Error(`Dependency ${dependency} is not installed. Run npm install before packaging.`);
    }
    seen.add(dependency);
    const dependencyManifest = JSON.parse(fs.readFileSync(packageJsonPath, "utf8"));
    for (const transitive of Object.keys(dependencyManifest.dependencies ?? {})) {
      queue.push(transitive);
    }
  }
  return [...seen].sort();
}

function archiveMode(entryPath) {
  return isPackagedSageBinary(entryPath)
    ? 0o100755
    : 0o100644;
}

function isPackagedSageBinary(entryPath) {
  return /^extension\/resources\/bin\/[^/]+\/sage-ls$/.test(entryPath);
}

function assertNoBuildMachinePaths(binary, entryPath) {
  const text = binary.toString("latin1");
  const candidates = [
    repositoryRoot,
    os.homedir(),
    process.env.CARGO_HOME ? path.resolve(process.env.CARGO_HOME) : null,
    "/Users/",
    "/home/",
    "\\Users\\",
  ].filter(Boolean);
  const leaked = [...new Set(candidates.filter((candidate) => text.includes(candidate)))];
  if (leaked.length > 0) {
    throw new Error(`${entryPath} contains build-machine paths: ${leaked.join(", ")}`);
  }
}

function contentTypesXml(fileEntries) {
  const defaults = new Map([
    ["bnf", "text/plain"],
    ["cmd", "text/plain"],
    ["js", "application/javascript"],
    ["json", "application/json"],
    ["md", "text/markdown"],
    ["png", "image/png"],
    ["sha256", "text/plain"],
    ["sh", "text/x-shellscript"],
    ["svg", "image/svg+xml"],
    ["ts", "text/plain"],
    ["txt", "text/plain"],
    ["vsixmanifest", "text/xml"],
    ["xml", "text/xml"],
    ["yml", "text/yaml"],
  ]);
  const overrides = [];
  for (const entry of fileEntries) {
    const extension = path.posix.extname(entry.path).slice(1).toLowerCase();
    if (!extension) {
      overrides.push({ path: entry.path, contentType: contentTypeForExtensionlessEntry(entry.path) });
      continue;
    }
    if (!defaults.has(extension)) {
      defaults.set(extension, contentTypeForUnknownExtension(extension));
    }
  }
  const defaultXml = [...defaults.entries()]
    .sort(([left], [right]) => left.localeCompare(right))
    .map(([extension, contentType]) => `  <Default Extension="${xmlEscape(extension)}" ContentType="${xmlEscape(contentType)}"/>`)
    .join("\n");
  const overrideXml = overrides
    .sort((left, right) => left.path.localeCompare(right.path))
    .map((override) => `  <Override PartName="/${xmlEscape(override.path)}" ContentType="${xmlEscape(override.contentType)}"/>`)
    .join("\n");
  const overrideSection = overrideXml ? `\n${overrideXml}` : "";
  return `<?xml version="1.0" encoding="utf-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
${defaultXml}${overrideSection}
</Types>
`;
}

function contentTypeForExtensionlessEntry(entryPath) {
  const baseName = path.posix.basename(entryPath);
  if (baseName === "sage-ls") {
    return "application/octet-stream";
  }
  return "text/plain";
}

function contentTypeForUnknownExtension(extension) {
  if (extension === "exe" || extension === "node") {
    return "application/octet-stream";
  }
  return "text/plain";
}

function vsixManifestXml(extensionManifest) {
  const categories = (extensionManifest.categories ?? []).join(",");
  const tags = (extensionManifest.keywords ?? []).join(" ");
  const extensionKind = (extensionManifest.extensionKind ?? []).join(",");
  const galleryFlags = extensionManifest.preview ? "Preview" : "";
  const iconAsset = extensionManifest.icon
    ? `    <Asset Type="Microsoft.VisualStudio.Services.Icons.Default" Path="extension/${xmlEscape(extensionManifest.icon)}" Addressable="true"/>\n`
    : "";
  return `<?xml version="1.0" encoding="utf-8"?>
<PackageManifest Version="2.0.0" xmlns="http://schemas.microsoft.com/developer/vsx-schema/2011">
  <Metadata>
    <Identity Language="en-US" Id="${xmlEscape(extensionManifest.name)}" Version="${xmlEscape(extensionManifest.version)}" Publisher="${xmlEscape(extensionManifest.publisher)}"/>
    <DisplayName>${xmlEscape(extensionManifest.displayName ?? extensionManifest.name)}</DisplayName>
    <Description xml:space="preserve">${xmlEscape(extensionManifest.description ?? "")}</Description>
    <Tags>${xmlEscape(tags)}</Tags>
    <Categories>${xmlEscape(categories)}</Categories>
    <GalleryFlags>${xmlEscape(galleryFlags)}</GalleryFlags>
    <Properties>
      <Property Id="Microsoft.VisualStudio.Code.Engine" Value="${xmlEscape(extensionManifest.engines?.vscode ?? "")}"/>
      <Property Id="Microsoft.VisualStudio.Code.ExtensionKind" Value="${xmlEscape(extensionKind)}"/>
    </Properties>
  </Metadata>
  <Installation>
    <InstallationTarget Id="Microsoft.VisualStudio.Code"/>
  </Installation>
  <Dependencies/>
  <Assets>
    <Asset Type="Microsoft.VisualStudio.Code.Manifest" Path="extension/package.json" Addressable="true"/>
${iconAsset}    <Asset Type="Microsoft.VisualStudio.Services.Content.License" Path="extension/LICENSE" Addressable="true"/>
    <Asset Type="Microsoft.VisualStudio.Services.Content.Details" Path="extension/README.md" Addressable="true"/>
    <Asset Type="Microsoft.VisualStudio.Services.Content.Changelog" Path="extension/CHANGELOG.md" Addressable="true"/>
  </Assets>
</PackageManifest>
`;
}

function xmlEscape(value) {
  return String(value)
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll("\"", "&quot;")
    .replaceAll("'", "&apos;");
}

function writeZip(filePath, fileEntries) {
  const localParts = [];
  const centralParts = [];
  let offset = 0;
  let compressedPayloadSize = 0;
  let uncompressedPayloadSize = 0;
  for (const entry of fileEntries) {
    const name = Buffer.from(entry.path, "utf8");
    const data = Buffer.isBuffer(entry.data) ? entry.data : Buffer.from(entry.data);
    const deflated = deflateRawSync(data, { level: 9 });
    const compressionMethod = deflated.length < data.length ? ZIP_METHOD_DEFLATE : ZIP_METHOD_STORE;
    const payload = compressionMethod === ZIP_METHOD_DEFLATE ? deflated : data;
    compressedPayloadSize += payload.length;
    uncompressedPayloadSize += data.length;
    const crc = crc32(data);
    const { time, date } = dosTimestamp(ARCHIVE_TIMESTAMP);
    const localHeader = Buffer.alloc(30);
    localHeader.writeUInt32LE(0x04034b50, 0);
    localHeader.writeUInt16LE(20, 4);
    localHeader.writeUInt16LE(0x0800, 6);
    localHeader.writeUInt16LE(compressionMethod, 8);
    localHeader.writeUInt16LE(time, 10);
    localHeader.writeUInt16LE(date, 12);
    localHeader.writeUInt32LE(crc, 14);
    localHeader.writeUInt32LE(payload.length, 18);
    localHeader.writeUInt32LE(data.length, 22);
    localHeader.writeUInt16LE(name.length, 26);
    localHeader.writeUInt16LE(0, 28);
    localParts.push(localHeader, name, payload);

    const centralHeader = Buffer.alloc(46);
    centralHeader.writeUInt32LE(0x02014b50, 0);
    centralHeader.writeUInt16LE(0x031e, 4);
    centralHeader.writeUInt16LE(20, 6);
    centralHeader.writeUInt16LE(0x0800, 8);
    centralHeader.writeUInt16LE(compressionMethod, 10);
    centralHeader.writeUInt16LE(time, 12);
    centralHeader.writeUInt16LE(date, 14);
    centralHeader.writeUInt32LE(crc, 16);
    centralHeader.writeUInt32LE(payload.length, 20);
    centralHeader.writeUInt32LE(data.length, 24);
    centralHeader.writeUInt16LE(name.length, 28);
    centralHeader.writeUInt16LE(0, 30);
    centralHeader.writeUInt16LE(0, 32);
    centralHeader.writeUInt16LE(0, 34);
    centralHeader.writeUInt16LE(0, 36);
    centralHeader.writeUInt32LE((((entry.mode ?? 0o100644) & 0xffff) * 0x10000) >>> 0, 38);
    centralHeader.writeUInt32LE(offset, 42);
    centralParts.push(centralHeader, name);
    offset += localHeader.length + name.length + payload.length;
  }

  const centralDirectory = Buffer.concat(centralParts);
  const end = Buffer.alloc(22);
  end.writeUInt32LE(0x06054b50, 0);
  end.writeUInt16LE(0, 4);
  end.writeUInt16LE(0, 6);
  end.writeUInt16LE(fileEntries.length, 8);
  end.writeUInt16LE(fileEntries.length, 10);
  end.writeUInt32LE(centralDirectory.length, 12);
  end.writeUInt32LE(offset, 16);
  end.writeUInt16LE(0, 20);
  fs.writeFileSync(filePath, Buffer.concat([...localParts, centralDirectory, end]));
  return { compressedPayloadSize, uncompressedPayloadSize };
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

function dosTimestamp(value) {
  const year = Math.max(1980, value.getUTCFullYear());
  const date = ((year - 1980) << 9) | ((value.getUTCMonth() + 1) << 5) | value.getUTCDate();
  const time = (value.getUTCHours() << 11) | (value.getUTCMinutes() << 5) | Math.floor(value.getUTCSeconds() / 2);
  return { date, time };
}

function archiveTimestampDate() {
  const raw = process.env.SOURCE_DATE_EPOCH;
  if (raw === undefined || raw === "") {
    return new Date(Date.UTC(1980, 0, 1, 0, 0, 0));
  }
  if (!/^\d+$/.test(raw)) {
    throw new Error(`SOURCE_DATE_EPOCH must be a non-negative integer, got ${raw}`);
  }
  const seconds = Number(raw);
  if (!Number.isSafeInteger(seconds)) {
    throw new Error(`SOURCE_DATE_EPOCH is too large: ${raw}`);
  }
  return new Date(seconds * 1000);
}

function slash(value) {
  return value.split(path.sep).join("/");
}
