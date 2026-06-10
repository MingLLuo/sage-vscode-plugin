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
  await fs.writeFile(path.join(outputRoot, "index.html"), htmlTemplate(), "utf8");
  await fs.writeFile(path.join(outputRoot, "assets", "viewer.css"), cssTemplate(), "utf8");
  await fs.writeFile(path.join(outputRoot, "assets", "viewer.js"), jsTemplate(), "utf8");
  await fs.writeFile(path.join(outputRoot, "README.md"), readmeTemplate(bundle.manifest), "utf8");
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

function htmlTemplate() {
  return `<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Sage Offline Reference</title>
  <link rel="stylesheet" href="./assets/viewer.css">
</head>
<body>
  <header class="topbar">
    <div>
      <h1 id="projectName">Sage Reference</h1>
      <p id="projectMeta">Loading...</p>
    </div>
    <div class="searchbox">
      <input id="searchInput" type="search" placeholder="Search symbols, modules, docs, paths" autocomplete="off" aria-label="Search symbols">
      <button id="themeButton" type="button" title="Toggle theme">Theme</button>
    </div>
  </header>
  <main class="layout">
    <aside class="sidebar">
      <div class="section-title">Symbols</div>
      <div id="symbolGroups" class="symbol-groups"></div>
    </aside>
    <section class="source-panel">
      <div class="panel-header">
        <span id="sourceTitle">Source</span>
        <span id="sourceHint"></span>
      </div>
      <pre id="sourceView" class="source-view"></pre>
    </section>
    <aside class="detail-panel">
      <div id="detailView" class="detail-view"></div>
      <div class="section-title">References</div>
      <div id="referenceList" class="reference-list"></div>
      <div class="section-title">Recent</div>
      <div id="recentList" class="recent-list"></div>
    </aside>
  </main>
  <script src="./data/manifest.js"></script>
  <script src="./data/symbols.js"></script>
  <script src="./assets/viewer.js"></script>
</body>
</html>
`;
}

function cssTemplate() {
  return `:root {
  color-scheme: light dark;
  --bg: #f6f8fb;
  --panel: #ffffff;
  --text: #17202a;
  --muted: #667085;
  --line: #d9e0ea;
  --accent: #0b6b6f;
  --accent-soft: #e6f3f4;
  --project: #155eef;
  --sage: #8f3c00;
  --warning: #b54708;
  --definition: #067647;
  --reference: #6941c6;
  --code-bg: #fbfcfe;
  --shadow: 0 8px 24px rgba(16, 24, 40, .08);
  font-family: Inter, ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
}
html[data-theme="dark"] {
  --bg: #0e1116;
  --panel: #161b22;
  --text: #e6edf3;
  --muted: #9aa7b5;
  --line: #303744;
  --accent: #3fb5b9;
  --accent-soft: #15383a;
  --project: #78a8ff;
  --sage: #d494ff;
  --warning: #ffb86b;
  --definition: #57d98f;
  --reference: #c7a5ff;
  --code-bg: #0b0f14;
  --shadow: none;
}
* { box-sizing: border-box; }
body { margin: 0; background: var(--bg); color: var(--text); }
.topbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
  padding: 14px 18px;
  border-bottom: 1px solid var(--line);
  background: var(--panel);
  position: sticky;
  top: 0;
  z-index: 5;
}
h1 { margin: 0; font-size: 18px; letter-spacing: 0; }
p { margin: 3px 0 0; color: var(--muted); font-size: 12px; }
.searchbox { display: flex; gap: 8px; min-width: min(560px, 48vw); }
input, button {
  border: 1px solid var(--line);
  background: var(--panel);
  color: var(--text);
  border-radius: 6px;
  padding: 9px 10px;
  font: inherit;
}
input { width: 100%; }
button { cursor: pointer; color: var(--accent); }
.layout {
  display: grid;
  grid-template-columns: minmax(260px, 330px) minmax(480px, 1fr) minmax(320px, 420px);
  gap: 12px;
  padding: 12px;
  min-height: calc(100vh - 70px);
}
.sidebar, .source-panel, .detail-panel {
  background: var(--panel);
  border: 1px solid var(--line);
  border-radius: 8px;
  box-shadow: var(--shadow);
  min-width: 0;
  overflow: hidden;
}
.sidebar, .detail-panel { max-height: calc(100vh - 94px); overflow: auto; }
.section-title {
  padding: 10px 12px;
  border-bottom: 1px solid var(--line);
  color: var(--muted);
  font-size: 11px;
  font-weight: 700;
  text-transform: uppercase;
  letter-spacing: 0;
}
.group-title { padding: 10px 12px 6px; color: var(--muted); font-size: 12px; font-weight: 700; }
.symbol-item, .reference-item, .recent-item {
  display: block;
  width: 100%;
  border: 0;
  border-bottom: 1px solid var(--line);
  border-radius: 0;
  padding: 9px 12px;
  text-align: left;
  background: transparent;
  color: var(--text);
}
.symbol-item:hover, .symbol-item.active, .reference-item:hover, .recent-item:hover { background: var(--accent-soft); }
.symbol-name { display: block; font-weight: 700; color: var(--text); overflow-wrap: anywhere; }
.symbol-meta { display: block; margin-top: 2px; color: var(--muted); font-size: 12px; overflow-wrap: anywhere; }
.origin-project { color: var(--project); }
.origin-sage { color: var(--sage); }
.panel-header {
  display: flex;
  justify-content: space-between;
  gap: 10px;
  padding: 10px 12px;
  border-bottom: 1px solid var(--line);
  color: var(--muted);
  font-size: 12px;
}
.source-view {
  margin: 0;
  padding: 10px 0 18px;
  overflow: auto;
  height: calc(100vh - 140px);
  background: var(--code-bg);
  font: 13px/1.55 "SFMono-Regular", Menlo, Consolas, monospace;
  white-space: pre;
}
.line { display: block; min-height: 20px; padding-right: 14px; }
.line-number { display: inline-block; width: 58px; padding-right: 12px; text-align: right; color: var(--muted); user-select: none; }
.line.definition { background: color-mix(in srgb, var(--definition) 14%, transparent); }
.line.reference { background: color-mix(in srgb, var(--reference) 12%, transparent); }
.detail-view { padding: 14px; }
.detail-view h2 { margin: 0 0 6px; font-size: 20px; overflow-wrap: anywhere; }
.badge-row { display: flex; flex-wrap: wrap; gap: 6px; margin: 8px 0 12px; }
.badge { border: 1px solid var(--line); border-radius: 999px; padding: 3px 8px; color: var(--muted); font-size: 12px; }
.badge.warn { color: var(--warning); border-color: var(--warning); }
.signature, .full-doc {
  border: 1px solid var(--line);
  border-radius: 6px;
  padding: 10px;
  background: var(--code-bg);
  font: 12px/1.5 "SFMono-Regular", Menlo, Consolas, monospace;
  white-space: pre-wrap;
  overflow-wrap: anywhere;
}
.full-doc { max-height: 300px; overflow: auto; margin-top: 10px; }
.action-row { display: flex; flex-wrap: wrap; gap: 8px; margin: 12px 0; }
.empty { color: var(--muted); padding: 12px; }
@media (max-width: 1100px) {
  .layout { grid-template-columns: 1fr; }
  .sidebar, .detail-panel, .source-view { max-height: none; height: auto; }
  .source-view { min-height: 360px; }
  .topbar { align-items: stretch; flex-direction: column; }
  .searchbox { min-width: 0; width: 100%; }
}
`;
}

function jsTemplate() {
  return `"use strict";
const manifest = window.__SAGE_REFERENCE_MANIFEST__;
const data = window.__SAGE_REFERENCE_SYMBOLS__;
window.__SAGE_REFERENCE_SOURCES__ = window.__SAGE_REFERENCE_SOURCES__ || {};
const state = { symbolId: null, sourceId: null, line: null, recent: [] };
const byId = new Map(data.symbols.map((symbol) => [symbol.id, symbol]));
const sources = new Map(data.sources.map((source) => [source.id, source]));
const referencesBySymbol = new Map();
for (const reference of data.references) {
  if (!referencesBySymbol.has(reference.symbolId)) referencesBySymbol.set(reference.symbolId, []);
  referencesBySymbol.get(reference.symbolId).push(reference);
}

document.querySelector("#projectName").textContent = manifest.projectName + " Sage Reference";
document.querySelector("#projectMeta").textContent =
  manifest.stats.symbols + " symbols | " + manifest.stats.sources + " sources | generated " + manifest.generatedAt;
const searchInput = document.querySelector("#searchInput");
searchInput.addEventListener("input", () => renderSymbolGroups(searchInput.value));
searchInput.addEventListener("keydown", (event) => {
  if (event.key === "Enter") {
    const first = document.querySelector(".symbol-item");
    if (first) first.click();
  }
  if (event.key === "Escape") {
    searchInput.value = "";
    renderSymbolGroups("");
  }
});
document.querySelector("#themeButton").addEventListener("click", () => {
  const next = document.documentElement.dataset.theme === "dark" ? "light" : "dark";
  document.documentElement.dataset.theme = next;
  localStorage.setItem("sage-reference-theme", next);
});
document.addEventListener("keydown", (event) => {
  if (event.key === "/" && document.activeElement !== searchInput) {
    event.preventDefault();
    searchInput.focus();
  }
  if (event.key === "[" || event.key === "]") {
    moveReference(event.key === "]" ? 1 : -1);
  }
});
window.addEventListener("hashchange", restoreFromHash);
document.documentElement.dataset.theme = localStorage.getItem("sage-reference-theme") || (matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light");
renderSymbolGroups("");
restoreFromHash();
if (state.symbolId == null && data.symbols[0]) selectSymbol(data.symbols[0].id, false);

function renderSymbolGroups(filter) {
  const needle = filter.trim().toLowerCase();
  const ids = needle
    ? data.searchIndex.filter((entry) => fuzzy(entry.text, needle)).slice(0, 160).map((entry) => entry.id)
    : data.symbols.slice(0, 160).map((symbol) => symbol.id);
  const groups = new Map();
  for (const id of ids) {
    const symbol = byId.get(id);
    const group = symbol.origin === "sage" ? "Sage API" : symbol.kind === "Module" ? "Modules" : symbol.kind === "Class" ? "Classes / Types" : symbol.origin === "project" ? "Project" : "Functions";
    if (!groups.has(group)) groups.set(group, []);
    groups.get(group).push(symbol);
  }
  const root = document.querySelector("#symbolGroups");
  root.innerHTML = "";
  if (ids.length === 0) {
    root.innerHTML = '<div class="empty">No symbols matched this search.</div>';
    return;
  }
  for (const [group, symbols] of groups) {
    const title = document.createElement("div");
    title.className = "group-title";
    title.textContent = group + " (" + symbols.length + ")";
    root.append(title);
    for (const symbol of symbols) {
      const button = document.createElement("button");
      button.type = "button";
      button.className = "symbol-item" + (symbol.id === state.symbolId ? " active" : "");
      button.innerHTML = '<span class="symbol-name origin-' + escapeAttr(symbol.origin) + '">' + escapeHtml(symbol.name) + '</span><span class="symbol-meta">' + escapeHtml([symbol.kind, symbol.module].filter(Boolean).join(" | ")) + '</span>';
      button.addEventListener("click", () => selectSymbol(symbol.id, true));
      root.append(button);
    }
  }
}

async function selectSymbol(id, updateHash) {
  const symbol = byId.get(id);
  if (!symbol) return;
  state.symbolId = id;
  state.sourceId = symbol.definition?.sourceId ?? symbol.sourceId;
  state.line = symbol.definition?.range?.startLine ?? null;
  state.recent = [id, ...state.recent.filter((entry) => entry !== id)].slice(0, 8);
  if (updateHash) setHash("symbol=" + encodeURIComponent(symbol.name));
  renderDetail(symbol);
  renderReferences(symbol);
  renderRecent();
  await renderSource(state.sourceId, state.line, id);
  renderSymbolGroups(searchInput.value);
}

function renderDetail(symbol) {
  const target = document.querySelector("#detailView");
  const definition = symbol.definition;
  target.innerHTML =
    '<h2>' + escapeHtml(symbol.name) + '</h2>' +
    '<div class="badge-row">' +
    badge(symbol.origin) + badge(symbol.kind) + badge(symbol.confidence, symbol.confidence !== "high") + badge(symbol.ownerType || "") +
    '</div>' +
    '<p>' + escapeHtml(symbol.module || "module unavailable") + '</p>' +
    '<div class="action-row">' +
    actionButton("Copy symbol", () => copyText(symbol.name)) +
    actionButton("Copy path", () => copyText(definition?.virtualPath || "")) +
    actionButton("Copy definition", () => copyText(definition ? definition.virtualPath + ":" + (definition.range.startLine + 1) : "")) +
    '</div>' +
    (symbol.signature ? '<div class="signature">' + escapeHtml(symbol.signature) + '</div>' : '<p>No signature available.</p>') +
    '<div class="full-doc">' + escapeHtml(symbol.doc || symbol.summary || "No documentation was exported for this symbol.") + '</div>' +
    '<p>Resolution: ' + escapeHtml(symbol.reason || "none") + '</p>';
  for (const button of target.querySelectorAll("button[data-action]")) {
    button.addEventListener("click", () => copyText(button.dataset.value || ""));
  }
}

function renderReferences(symbol) {
  const refs = referencesBySymbol.get(symbol.id) || [];
  const root = document.querySelector("#referenceList");
  if (refs.length === 0) {
    root.innerHTML = '<div class="empty">No references were exported for this symbol.</div>';
    return;
  }
  root.innerHTML = "";
  refs.slice(0, 200).forEach((reference, index) => {
    const source = sources.get(reference.sourceId);
    const button = document.createElement("button");
    button.type = "button";
    button.className = "reference-item";
    button.textContent = source.virtualPath + ":" + (reference.range.startLine + 1);
    button.addEventListener("click", async () => {
      state.sourceId = reference.sourceId;
      state.line = reference.range.startLine;
      setHash("source=" + encodeURIComponent(source.virtualPath) + "&line=" + (reference.range.startLine + 1));
      await renderSource(reference.sourceId, reference.range.startLine, symbol.id, index);
    });
    root.append(button);
  });
}

async function renderSource(sourceId, focusLine, symbolId) {
  const sourceMeta = sources.get(sourceId);
  if (!sourceMeta) {
    document.querySelector("#sourceView").textContent = "No source selected.";
    return;
  }
  document.querySelector("#sourceTitle").textContent = sourceMeta.virtualPath;
  document.querySelector("#sourceHint").textContent = sourceMeta.snapshot ? "snapshot" : "snippet or metadata";
  const text = await loadSource(sourceId);
  const definitionLine = byId.get(symbolId)?.definition?.range?.startLine;
  const referenceLines = new Set((referencesBySymbol.get(symbolId) || []).filter((ref) => ref.sourceId === sourceId).map((ref) => ref.range.startLine));
  document.querySelector("#sourceView").innerHTML = text.split(/\\r?\\n/).map((line, index) => {
    const classes = ["line"];
    if (index === definitionLine) classes.push("definition");
    if (referenceLines.has(index)) classes.push("reference");
    return '<span class="' + classes.join(" ") + '" id="L' + (index + 1) + '"><span class="line-number">' + (index + 1) + '</span>' + escapeHtml(line) + '</span>';
  }).join("");
  if (typeof focusLine === "number") {
    document.querySelector("#L" + (focusLine + 1))?.scrollIntoView({ block: "center" });
  }
}

function renderRecent() {
  const root = document.querySelector("#recentList");
  root.innerHTML = "";
  for (const id of state.recent) {
    const symbol = byId.get(id);
    if (!symbol) continue;
    const button = document.createElement("button");
    button.type = "button";
    button.className = "recent-item";
    button.textContent = symbol.name;
    button.addEventListener("click", () => selectSymbol(id, true));
    root.append(button);
  }
}

function restoreFromHash() {
  const params = new URLSearchParams(location.hash.replace(/^#/, ""));
  const symbolName = params.get("symbol");
  if (symbolName) {
    const symbol = data.symbols.find((entry) => entry.name === symbolName);
    if (symbol) selectSymbol(symbol.id, false);
    return;
  }
  const sourcePath = params.get("source");
  if (sourcePath) {
    const source = data.sources.find((entry) => entry.virtualPath === sourcePath);
    const line = Math.max(0, Number(params.get("line") || "1") - 1);
    if (source) renderSource(source.id, line, state.symbolId);
  }
}

function loadSource(id) {
  if (window.__SAGE_REFERENCE_SOURCES__[id]) return Promise.resolve(window.__SAGE_REFERENCE_SOURCES__[id].text || "");
  return new Promise((resolve, reject) => {
    const script = document.createElement("script");
    script.src = "./data/sources/source-" + id + ".js";
    script.onload = () => resolve(window.__SAGE_REFERENCE_SOURCES__[id]?.text || "");
    script.onerror = () => reject(new Error("failed to load source shard " + id));
    document.head.append(script);
  });
}

function moveReference(delta) {
  const refs = referencesBySymbol.get(state.symbolId) || [];
  if (!refs.length) return;
  const current = refs.findIndex((ref) => ref.sourceId === state.sourceId && ref.range.startLine === state.line);
  const next = refs[(Math.max(0, current) + delta + refs.length) % refs.length];
  const source = sources.get(next.sourceId);
  setHash("source=" + encodeURIComponent(source.virtualPath) + "&line=" + (next.range.startLine + 1));
  renderSource(next.sourceId, next.range.startLine, state.symbolId);
}

function setHash(value) {
  history.replaceState(null, "", "#" + value);
}

function actionButton(label, valueFactory) {
  const value = valueFactory();
  return '<button type="button" data-action="copy" data-value="' + escapeAttr(value) + '">' + escapeHtml(label) + '</button>';
}
function badge(value, warn) { return value ? '<span class="badge' + (warn ? " warn" : "") + '">' + escapeHtml(value) + '</span>' : ""; }
function fuzzy(text, needle) { return needle.split(/\\s+/).every((part) => text.includes(part)); }
function copyText(value) { navigator.clipboard?.writeText(value); }
function escapeHtml(value) { return String(value ?? "").replace(/[&<>"']/g, (char) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[char])); }
function escapeAttr(value) { return escapeHtml(value); }
`;
}

function readmeTemplate(manifest) {
  return `# Sage Offline Reference

Open \`index.html\` in a browser. This reference bundle is static and does not require Sage, VS Code, or the Sage VS Code extension.

- Project: ${manifest.projectName}
- Generated: ${manifest.generatedAt}
- Symbols: ${manifest.stats.symbols}
- Sources: ${manifest.stats.sources}
- References: ${manifest.stats.references}

Regenerate from the extension repository with:

\`\`\`bash
npm run export:reference -- --workspace /path/to/project --source-root /path/to/sage/src
\`\`\`
`;
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
