#!/usr/bin/env node
import fs from "node:fs/promises";
import fsSync from "node:fs";
import os from "node:os";
import path from "node:path";
import { execFile } from "node:child_process";
import { fileURLToPath } from "node:url";
import { promisify } from "node:util";

const execFileAsync = promisify(execFile);
const __dirname = path.dirname(fileURLToPath(import.meta.url));
const repositoryRoot = path.resolve(__dirname, "..");
const referenceViewerRoot = path.join(__dirname, "reference-viewer");
const args = parseArgs(process.argv.slice(2));
const workspaceRoot = path.resolve(args.workspace ?? process.cwd());
const outputRoot = path.resolve(args.out ?? path.join(workspaceRoot, ".sage-reference"));
const sourceRoots = args.sourceRoots.map((root) => path.resolve(root));
const debugInspector = path.join(
  repositoryRoot,
  "target",
  "debug",
  process.platform === "win32" ? "sage-debug-inspect.exe" : "sage-debug-inspect",
);
const cacheHome = path.join(os.tmpdir(), "sage-vscode-reference-export-cache");
const generatedAt = new Date().toISOString();

const projectFiles = await collectProjectFiles(workspaceRoot);
if (projectFiles.length === 0) {
  fail(`no Sage/Python/Cython files found under ${workspaceRoot}`);
}

const inspected = await inspectProjectFiles(projectFiles);
const bundle = await buildBundle(inspected);
await writeBundle(bundle);
printSummary(bundle);

async function inspectProjectFiles(files) {
  const results = [];
  let warmed = false;
  for (const file of files) {
    const source = await fs.readFile(file, "utf8");
    const candidates = candidateSymbolsFromSource(source);
    const batchPath = path.join(cacheHome, `reference-export-${process.pid}-${results.length}.json`);
    await fs.mkdir(cacheHome, { recursive: true });
    await fs.writeFile(
      batchPath,
      JSON.stringify(candidates.map((symbol) => ({ id: symbol, symbol }))),
      "utf8",
    );

    let inspectorPayload = null;
    if (sourceRoots.length > 0 && fsSync.existsSync(debugInspector)) {
      const inspectorArgs = [
        ...sourceRoots.flatMap((root) => ["--root", root]),
        "--root",
        workspaceRoot,
        "--editable-root",
        workspaceRoot,
        "--file",
        file,
        "--batch-file",
        batchPath,
        ...(warmed ? [] : ["--rebuild-index"]),
      ];
      try {
        const { stdout } = await execFileAsync(debugInspector, inspectorArgs, {
          cwd: repositoryRoot,
          env: { ...process.env, XDG_CACHE_HOME: cacheHome },
          maxBuffer: 80 * 1024 * 1024,
        });
        inspectorPayload = JSON.parse(stdout);
        warmed = true;
      } catch (error) {
        inspectorPayload = {
          diagnostics: [{ message: `inspector failed: ${String(error.message ?? error)}` }],
          parsed: parseSourceFallback(file, source),
          batchQueries: [],
        };
      }
    } else {
      inspectorPayload = {
        diagnostics: [],
        parsed: parseSourceFallback(file, source),
        batchQueries: [],
      };
    }
    results.push({ file, source, candidates, inspectorPayload });
  }
  return results;
}

async function buildBundle(inspections) {
  const sources = [];
  const sourceByVirtualPath = new Map();
  const symbols = [];
  const symbolByKey = new Map();
  const references = [];
  const relatedFiles = new Set();

  for (const inspection of inspections) {
    const projectSourceId = addSource(sources, sourceByVirtualPath, {
      virtualPath: virtualPathFor(inspection.file),
      language: languageForPath(inspection.file),
      origin: "project",
      text: inspection.source,
      snapshot: true,
    });

    const parsedSymbols = inspection.inspectorPayload?.parsed?.symbols ?? [];
    for (const symbol of parsedSymbols) {
      if (!symbol?.name || symbol.kind === "Module") {
        continue;
      }
      addSymbol(symbols, symbolByKey, {
        name: symbol.name,
        kind: symbol.kind ?? "Symbol",
        module: symbol.module ?? moduleNameForPath(inspection.file),
        origin: "project",
        sourceId: projectSourceId,
        signature: symbol.signature ?? null,
        summary: sanitizeText(firstDocLine(symbol.docstring) ?? symbol.detail ?? ""),
        doc: sanitizeText(symbol.docstring ?? ""),
        definition: {
          sourceId: projectSourceId,
          range: normalizeRange(symbol.range),
          virtualPath: sources[projectSourceId].virtualPath,
        },
        confidence: "high",
        reason: "project-symbol",
      });
    }

    for (const item of inspection.inspectorPayload?.batchQueries ?? []) {
      const query = item.query ?? {};
      const documentation = query.documentation ?? {};
      const definition = query.definition ?? {};
      const name = documentation.name ?? definition.name ?? query.target?.symbol ?? item.id;
      if (!name || query.fallback_reason) {
        continue;
      }
      let definitionSourceId = null;
      let definitionVirtualPath = null;
      if (definition.path) {
        definitionVirtualPath = virtualPathFor(definition.path);
        if (definitionVirtualPath.startsWith("sage://") || definitionVirtualPath.startsWith("project://")) {
          relatedFiles.add(definition.path);
          definitionSourceId = addSource(sources, sourceByVirtualPath, {
            virtualPath: definitionVirtualPath,
            language: languageForPath(definition.path),
            origin: definitionVirtualPath.startsWith("sage://") ? "sage" : "project",
            text: null,
            snapshot: false,
          });
        }
      }

      const symbolId = addSymbol(symbols, symbolByKey, {
        name,
        kind: documentation.kind ?? definition.kind ?? "Sage API",
        module: documentation.module_name ?? definition.module ?? "",
        origin: definitionVirtualPath?.startsWith("sage://") ? "sage" : "project",
        sourceId: definitionSourceId,
        signature: query.signature?.label ?? null,
        summary: sanitizeText(documentation.summary ?? definition.detail ?? query.hover?.markdown ?? ""),
        doc: sanitizeText(documentation.docstring ?? documentation.detail ?? query.hover?.markdown ?? ""),
        definition: definitionSourceId == null ? null : {
          sourceId: definitionSourceId,
          range: normalizeRange(definition.range),
          virtualPath: definitionVirtualPath,
        },
        confidence: query.resolutionConfidence ?? "unknown",
        reason: query.resolutionReason ?? query.fallback_reason ?? "indexed-query",
        ownerType: query.ownerType ?? null,
      });

      for (const reference of query.references ?? []) {
        if (!reference?.path) {
          continue;
        }
        const referenceVirtualPath = virtualPathFor(reference.path);
        const referenceSourceId = addSource(sources, sourceByVirtualPath, {
          virtualPath: referenceVirtualPath,
          language: languageForPath(reference.path),
          origin: referenceVirtualPath.startsWith("sage://") ? "sage" : "project",
          text: referenceVirtualPath.startsWith("project://") ? await safeReadText(reference.path) : null,
          snapshot: referenceVirtualPath.startsWith("project://"),
        });
        references.push({
          symbolId,
          sourceId: referenceSourceId,
          range: normalizeRange(reference.range),
          role: "usage",
        });
      }
    }
  }

  if (args.sourceMode !== "none") {
    for (const file of [...relatedFiles].sort()) {
      const virtualPath = virtualPathFor(file);
      const sourceId = sourceByVirtualPath.get(virtualPath);
      if (sourceId == null) {
        continue;
      }
      const source = sources[sourceId];
      if (source.text != null) {
        continue;
      }
      const text = await safeReadText(file);
      source.text = args.sourceMode === "snippets"
        ? snippetsForSource(text, symbols.filter((symbol) => symbol.definition?.sourceId === sourceId))
        : text;
      source.snapshot = args.sourceMode === "snapshot";
    }
  }

  for (const source of sources) {
    source.hash = hashString(source.text ?? "");
    source.lineCount = source.text ? source.text.split(/\r?\n/).length : 0;
  }
  symbols.sort((left, right) =>
    `${left.origin}:${left.name}:${left.module}`.localeCompare(`${right.origin}:${right.name}:${right.module}`),
  );
  references.sort((left, right) =>
    `${left.symbolId}:${left.sourceId}:${left.range?.startLine ?? 0}:${left.range?.startCharacter ?? 0}`.localeCompare(
      `${right.symbolId}:${right.sourceId}:${right.range?.startLine ?? 0}:${right.range?.startCharacter ?? 0}`,
    ),
  );

  const searchIndex = symbols.map((symbol) => ({
    id: symbol.id,
    text: [
      symbol.name,
      symbol.module,
      symbol.kind,
      symbol.summary,
      sources[symbol.sourceId]?.virtualPath,
    ].filter(Boolean).join(" ").toLowerCase(),
  }));
  return {
    manifest: {
      schemaVersion: 1,
      projectName: path.basename(workspaceRoot),
      generatedAt,
      generator: "sage-vscode-reference-export",
      sourceMode: args.sourceMode,
      stats: {
        symbols: symbols.length,
        sources: sources.length,
        references: references.length,
        projectFiles: projectFiles.length,
      },
    },
    symbols,
    searchIndex,
    sources,
    references,
  };
}

async function writeBundle(bundle) {
  await fs.rm(outputRoot, { recursive: true, force: true });
  await fs.mkdir(path.join(outputRoot, "assets"), { recursive: true });
  await fs.mkdir(path.join(outputRoot, "data", "sources"), { recursive: true });
  await fs.copyFile(path.join(referenceViewerRoot, "index.html"), path.join(outputRoot, "index.html"));
  await fs.copyFile(path.join(referenceViewerRoot, "viewer.css"), path.join(outputRoot, "assets", "viewer.css"));
  await fs.copyFile(path.join(referenceViewerRoot, "viewer.js"), path.join(outputRoot, "assets", "viewer.js"));
  await fs.writeFile(
    path.join(outputRoot, "README.md"),
    await renderReferenceViewerReadme(bundle.manifest),
    "utf8",
  );
  await fs.writeFile(
    path.join(outputRoot, "data", "manifest.js"),
    `window.__SAGE_REFERENCE_MANIFEST__ = ${stableJson(bundle.manifest)};\n`,
    "utf8",
  );
  await fs.writeFile(
    path.join(outputRoot, "data", "symbols.js"),
    `window.__SAGE_REFERENCE_SYMBOLS__ = ${stableJson({
      symbols: bundle.symbols,
      searchIndex: bundle.searchIndex,
      references: bundle.references,
      sources: bundle.sources.map(({ text, ...source }) => source),
    })};\n`,
    "utf8",
  );
  for (const source of bundle.sources) {
    await fs.writeFile(
      path.join(outputRoot, "data", "sources", `source-${source.id}.js`),
      `window.__SAGE_REFERENCE_SOURCES__ = window.__SAGE_REFERENCE_SOURCES__ || {};\nwindow.__SAGE_REFERENCE_SOURCES__[${JSON.stringify(source.id)}] = ${stableJson({
        id: source.id,
        text: source.text ?? "",
      })};\n`,
      "utf8",
    );
  }
  await assertNoPrivatePaths(outputRoot);
}

function addSource(sources, sourceByVirtualPath, input) {
  if (sourceByVirtualPath.has(input.virtualPath)) {
    const id = sourceByVirtualPath.get(input.virtualPath);
    if (sources[id].text == null && input.text != null) {
      sources[id].text = input.text;
      sources[id].snapshot = input.snapshot;
    }
    return id;
  }
  const id = sources.length;
  sourceByVirtualPath.set(input.virtualPath, id);
  sources.push({
    id,
    virtualPath: input.virtualPath,
    language: input.language,
    origin: input.origin,
    snapshot: Boolean(input.snapshot),
    text: sanitizeText(input.text),
    hash: "",
    lineCount: 0,
  });
  return id;
}

function addSymbol(symbols, symbolByKey, input) {
  const key = `${input.origin}:${input.name}:${input.module}:${input.definition?.sourceId ?? ""}`;
  if (symbolByKey.has(key)) {
    return symbolByKey.get(key);
  }
  const id = symbols.length;
  symbolByKey.set(key, id);
  symbols.push({
    id,
    name: input.name,
    kind: input.kind,
    module: input.module,
    origin: input.origin,
    sourceId: input.sourceId,
    signature: input.signature,
    summary: oneLine(sanitizeText(input.summary)),
    doc: sanitizeText(input.doc),
    definition: input.definition,
    confidence: input.confidence,
    reason: input.reason,
    ownerType: input.ownerType ?? null,
  });
  return id;
}

function candidateSymbolsFromSource(source) {
  const names = new Set();
  for (const match of source.matchAll(/\bfrom\s+sage\.all\s+import\s+([^\n#]+)/g)) {
    for (const part of match[1].split(",")) {
      const name = part.trim().split(/\s+as\s+/)[0]?.trim();
      if (/^[A-Za-z_]\w*$/.test(name)) {
        names.add(name);
      }
    }
  }
  for (const match of source.matchAll(/\b(?:def|class)\s+([A-Za-z_]\w*)/g)) {
    names.add(match[1]);
  }
  for (const match of source.matchAll(/\b([A-Z][A-Za-z0-9_]{2,})\b/g)) {
    names.add(match[1]);
  }
  return [...names].sort().slice(0, 120);
}

function parseSourceFallback(file, source) {
  const symbols = [];
  for (const [lineIndex, line] of source.split(/\r?\n/).entries()) {
    const match = /^(?:async\s+)?(?:def|class)\s+([A-Za-z_]\w*)\s*(\([^)]*\))?/.exec(line.trim());
    if (!match) {
      continue;
    }
    symbols.push({
      name: match[1],
      kind: line.trim().startsWith("class ") ? "Class" : "Function",
      module: moduleNameForPath(file),
      path: file,
      detail: match[0],
      signature: match[2] ? `${match[1]}${match[2]}` : match[1],
      docstring: null,
      range: {
        start_line: lineIndex,
        start_character: line.indexOf(match[1]),
        end_line: lineIndex,
        end_character: line.indexOf(match[1]) + match[1].length,
      },
    });
  }
  return { module: moduleNameForPath(file), path: file, symbols };
}

async function collectProjectFiles(root) {
  const files = [];
  const stack = [root];
  const ignored = new Set([".git", ".sage-reference", "__pycache__", ".venv", "venv", "node_modules", "target", "dist", "build"]);
  while (stack.length > 0) {
    const current = stack.pop();
    let stat;
    try {
      stat = await fs.stat(current);
    } catch {
      continue;
    }
    if (stat.isDirectory()) {
      if (ignored.has(path.basename(current))) {
        continue;
      }
      for (const entry of await fs.readdir(current)) {
        stack.push(path.join(current, entry));
      }
      continue;
    }
    if (stat.isFile() && /\.(sage|py|pyx|pxd|pxi|spyx)$/i.test(current)) {
      files.push(path.resolve(current));
    }
  }
  return files.sort();
}

function virtualPathFor(filePath) {
  const resolved = path.resolve(String(filePath));
  if (isPathInsideOrEqual(resolved, workspaceRoot)) {
    return `project://${normalizeVirtual(path.relative(workspaceRoot, resolved) || path.basename(resolved))}`;
  }
  for (const sourceRoot of sourceRoots) {
    if (isPathInsideOrEqual(resolved, sourceRoot)) {
      return `sage://${normalizeVirtual(path.relative(sourceRoot, resolved) || path.basename(resolved))}`;
    }
  }
  return `external://${path.basename(resolved)}`;
}

function languageForPath(filePath) {
  const ext = path.extname(String(filePath)).toLowerCase();
  if (ext === ".sage") {
    return "sagemath";
  }
  if ([".pyx", ".pxd", ".pxi", ".spyx"].includes(ext)) {
    return "cython";
  }
  return "python";
}

function normalizeRange(range) {
  if (!range) {
    return null;
  }
  return {
    startLine: Number(range.start_line ?? range.startLine ?? 0),
    startCharacter: Number(range.start_character ?? range.startCharacter ?? 0),
    endLine: Number(range.end_line ?? range.endLine ?? 0),
    endCharacter: Number(range.end_character ?? range.endCharacter ?? 0),
  };
}

function snippetsForSource(text, symbols) {
  const lines = text.split(/\r?\n/);
  const included = new Set();
  for (const symbol of symbols) {
    const line = symbol.definition?.range?.startLine ?? 0;
    for (let index = Math.max(0, line - 8); index <= Math.min(lines.length - 1, line + 30); index += 1) {
      included.add(index);
    }
  }
  return [...included].sort((a, b) => a - b).map((line) => lines[line]).join("\n");
}

async function safeReadText(filePath) {
  try {
    return sanitizeText(await fs.readFile(filePath, "utf8"));
  } catch {
    return "";
  }
}

function sanitizeText(value) {
  if (value == null) {
    return value;
  }
  return String(value)
    .replace(/\/Users\/[^/\s]+/g, "<home>")
    .replace(/\/home\/[^/\s]+/g, "<home>")
    .replace(/[A-Za-z]:\\Users\\[^\\\s]+/g, "<home>");
}

async function assertNoPrivatePaths(root) {
  const files = [];
  const stack = [root];
  while (stack.length > 0) {
    const current = stack.pop();
    const stat = await fs.stat(current);
    if (stat.isDirectory()) {
      for (const entry of await fs.readdir(current)) {
        stack.push(path.join(current, entry));
      }
    } else {
      files.push(current);
    }
  }
  const privatePathPattern = /\/Users\/[^/\s]+\/|\/home\/[^/\s]+\/|[A-Za-z]:\\Users\\[^\\\s]+\\/;
  for (const file of files) {
    if (/\.(png|jpg|jpeg|gif|ico)$/i.test(file)) {
      continue;
    }
    const text = await fs.readFile(file, "utf8");
    if (privatePathPattern.test(text)) {
      fail(`private local path leaked into ${file}`);
    }
  }
}

async function renderReferenceViewerReadme(manifest) {
  const template = await fs.readFile(path.join(referenceViewerRoot, "README.md"), "utf8");
  return template
    .replace(/{{projectName}}/g, manifest.projectName)
    .replace(/{{generatedAt}}/g, manifest.generatedAt)
    .replace(/{{symbolCount}}/g, String(manifest.stats.symbols))
    .replace(/{{sourceCount}}/g, String(manifest.stats.sources))
    .replace(/{{referenceCount}}/g, String(manifest.stats.references));
}

function printSummary(bundle) {
  console.log(JSON.stringify({
    status: "exported",
    out: outputRoot,
    stats: bundle.manifest.stats,
  }, null, 2));
}

function parseArgs(rawArgs) {
  const parsed = {
    workspace: null,
    out: null,
    sourceRoots: [],
    sourceMode: "snapshot",
  };
  for (let index = 0; index < rawArgs.length; index += 1) {
    const item = rawArgs[index];
    if (["--workspace", "--out", "--source-root", "--source-mode"].includes(item)) {
      const value = rawArgs[index + 1];
      if (!value) fail(`missing value for ${item}`);
      if (item === "--source-root") parsed.sourceRoots.push(value);
      else parsed[item.slice(2).replace(/-([a-z])/g, (_, letter) => letter.toUpperCase())] = value;
      index += 1;
      continue;
    }
    if (item === "--help" || item === "-h") {
      console.log("Usage: node scripts/export-reference.mjs --workspace PATH --source-root PATH [--out DIR] [--source-mode snapshot|snippets|none]");
      process.exit(0);
    }
    fail(`unknown argument: ${item}`);
  }
  if (!["snapshot", "snippets", "none"].includes(parsed.sourceMode)) fail(`invalid --source-mode ${parsed.sourceMode}`);
  return parsed;
}

function moduleNameForPath(filePath) { return path.basename(filePath).replace(/\.(sage|py|pyx|pxd|pxi|spyx)$/i, ""); }
function firstDocLine(value) { return value?.split(/\r?\n/).map((line) => line.trim()).find(Boolean) ?? null; }
function oneLine(value) { return String(value ?? "").replace(/\s+/g, " ").trim().slice(0, 500); }
function normalizeVirtual(value) { return value.split(path.sep).join("/"); }
function isPathInsideOrEqual(targetPath, folder) { const relative = path.relative(folder, targetPath); return relative === "" || (!relative.startsWith("..") && !path.isAbsolute(relative)); }
function hashString(value) { let hash = 2166136261; for (let i = 0; i < value.length; i += 1) { hash ^= value.charCodeAt(i); hash = Math.imul(hash, 16777619); } return (hash >>> 0).toString(16).padStart(8, "0"); }
function stableJson(value) { return `${JSON.stringify(value, null, 2)}\n`; }
function fail(message) { console.error(message); process.exit(1); }
