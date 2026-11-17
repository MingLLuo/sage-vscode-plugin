#!/usr/bin/env node
// Records a machine-readable performance baseline for sage-ls against a full Sage source tree.
//
// Unlike the latency smoke (single samples, pass/fail budgets), this script measures what users
// feel while they work: latency during indexing and typing, large-file behaviour, memory, and
// disk. It never fails on a slow number; it writes JSON so runs can be compared.
//
// Usage: node scripts/perf-baseline.mjs [--profile release|debug] [--out file.json] [--samples N]
import { execFileSync } from "node:child_process";
import fsSync from "node:fs";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { LspProcess } from "./lib/lsp-process.mjs";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const repositoryRoot = path.resolve(__dirname, "..");
const args = parseArgs(process.argv.slice(2));
const profile = args.profile ?? "release";
const samples = Number(args.samples ?? 20);
const serverPath = path.join(
  repositoryRoot,
  "target",
  profile,
  process.platform === "win32" ? "sage-ls.exe" : "sage-ls",
);
const sageSourceRoot = path.resolve(
  process.env.SAGE_SOURCE_ROOT ?? path.join(repositoryRoot, "..", "sage", "src"),
);
const REQUEST_TIMEOUT_MS = 120_000;

function parseArgs(values) {
  const parsed = {};
  for (let index = 0; index < values.length; index += 1) {
    const match = /^--([a-z-]+)(?:=(.*))?$/.exec(values[index]);
    if (!match) continue;
    parsed[match[1]] = match[2] ?? values[++index];
  }
  return parsed;
}

function skip(reason) {
  console.log(JSON.stringify({ status: "skipped", reason }, null, 2));
  process.exit(0);
}
if (!fsSync.existsSync(serverPath)) skip(`missing sage-ls binary: ${serverPath}`);
if (!fsSync.existsSync(path.join(sageSourceRoot, "sage"))) skip(`missing Sage source root: ${sageSourceRoot}`);

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const round = (value) => Math.round(value * 10) / 10;

function summarize(values) {
  if (values.length === 0) return { count: 0 };
  const sorted = [...values].sort((left, right) => left - right);
  const at = (quantile) => sorted[Math.min(sorted.length - 1, Math.floor(quantile * sorted.length))];
  return {
    count: sorted.length,
    p50_ms: round(at(0.5)),
    p95_ms: round(at(0.95)),
    max_ms: round(sorted.at(-1)),
  };
}

function rssMb(pid) {
  if (process.platform === "win32") return null;
  try {
    const kb = Number(execFileSync("ps", ["-o", "rss=", "-p", String(pid)], { encoding: "utf8" }).trim());
    return Number.isFinite(kb) ? round(kb / 1024) : null;
  } catch {
    return null;
  }
}

async function directorySizeMb(root) {
  let total = 0;
  const walk = async (directory) => {
    for (const entry of await fs.readdir(directory, { withFileTypes: true }).catch(() => [])) {
      const target = path.join(directory, entry.name);
      if (entry.isDirectory()) await walk(target);
      // SQLite sidecars (-wal/-shm) can disappear between readdir and stat.
      else if (entry.isFile()) total += (await fs.stat(target).catch(() => ({ size: 0 }))).size;
    }
  };
  await walk(root);
  return round(total / (1024 * 1024));
}

const liveSessions = new Set();

class Session {
  constructor(context) {
    this.context = context;
    this.diagnosticWaiters = new Map();
    this.lastDiagnosticsAt = new Map();
    this.lastDiagnosticVersions = new Map();
    this.peakRssMb = 0;
  }

  async start() {
    liveSessions.add(this);
    this.server = new LspProcess(serverPath, {
      environment: { ...process.env, SAGE_LS_TEST_BACKGROUND_DELAY_MS: "0" },
      onNotification: (method, params) => this.handleNotification(method, params),
    });
    await this.server.start();
    this.rssTimer = setInterval(() => {
      this.peakRssMb = Math.max(this.peakRssMb, rssMb(this.server.child.pid) ?? 0);
    }, 200);
    const { workspaceRoot, cacheDir } = this.context;
    const workspaceUri = pathToFileURL(workspaceRoot).toString();
    const started = performance.now();
    await this.server.requestWithTimeout("initialize", {
      processId: process.pid,
      rootUri: workspaceUri,
      capabilities: { textDocument: { definition: { linkSupport: true } } },
      workspaceFolders: [{ uri: workspaceUri, name: "perf" }],
      initializationOptions: {
        analysis: {
          mode: "full",
          sourceRoots: [workspaceRoot, sageSourceRoot],
          extraPaths: [],
          enableDiagnostics: true,
          enableRuntimeIntrospection: false,
          enablePyxParsing: true,
          enablePythonFiles: true,
        },
        workspace: {
          rootUri: workspaceUri,
          folders: [workspaceUri],
          sourceRoots: [pathToFileURL(sageSourceRoot).toString()],
          exclude: ["**/.git/**", "**/__pycache__/**", "**/build/**"],
        },
        documentation: { preferredSource: "static", showOnHover: true },
        rust: { cacheDir },
      },
    }, REQUEST_TIMEOUT_MS, "initialize response");
    this.initializeMs = performance.now() - started;
    this.server.notify("initialized", {});
    this.startedAt = started;
  }

  handleNotification(method, params) {
    if (method !== "textDocument/publishDiagnostics") return;
    const at = performance.now();
    this.lastDiagnosticsAt.set(params.uri, at);
    this.lastDiagnosticVersions.set(params.uri, params.version);
    for (const waiter of [...(this.diagnosticWaiters.get(params.uri) ?? [])]) {
      if (waiter.version === undefined || waiter.version === params.version) waiter.finish(at);
    }
  }

  nextDiagnostics(uri, timeoutMs = 60_000, version) {
    if (version !== undefined && this.lastDiagnosticVersions.get(uri) === version) {
      return Promise.resolve(this.lastDiagnosticsAt.get(uri));
    }
    return new Promise((resolve) => {
      const waiters = this.diagnosticWaiters.get(uri) ?? [];
      const waiter = { version, finish: (at) => {
        clearTimeout(timer);
        this.diagnosticWaiters.set(uri, (this.diagnosticWaiters.get(uri) ?? []).filter((item) => item !== waiter));
        resolve(at);
      } };
      const timer = setTimeout(() => waiter.finish(null), timeoutMs);
      waiters.push(waiter);
      this.diagnosticWaiters.set(uri, waiters);
    });
  }

  status() {
    return this.server.requestWithTimeout(
      "workspace/executeCommand",
      { command: "sage.__rust.indexStatus", arguments: [] },
      REQUEST_TIMEOUT_MS,
      "index status",
    );
  }

  async waitForIndex(timeoutMs = 300_000) {
    const started = performance.now();
    for (;;) {
      const status = await this.status();
      if ((status?.pending_jobs ?? 0) === 0 && (status?.indexed_file_count ?? 0) > 1000) {
        return { ready_ms: round(performance.now() - this.startedAt), status };
      }
      if (performance.now() - started > timeoutMs) throw new Error("index did not become ready");
      await sleep(50);
    }
  }

  async timed(method, params) {
    const started = performance.now();
    let result;
    let error;
    try {
      result = await this.server.requestWithTimeout(method, params, REQUEST_TIMEOUT_MS, method);
    } catch (caught) {
      error = String(caught);
    }
    return { ms: performance.now() - started, result, error };
  }

  async sample(method, params, count = samples) {
    const values = [];
    let lastResult;
    for (let index = 0; index < count; index += 1) {
      const { ms, result, error } = await this.timed(method, params);
      if (error) return { error };
      values.push(ms);
      lastResult = result;
    }
    return { ...summarize(values), lastResult };
  }

  open(uri, text, languageId = "python", version = 1) {
    this.server.notify("textDocument/didOpen", { textDocument: { uri, languageId, version, text } });
  }

  rss() {
    return rssMb(this.server.child.pid);
  }

  async stop() {
    clearInterval(this.rssTimer);
    try {
      await this.server.requestWithTimeout("shutdown", undefined, 10_000, "shutdown");
      this.server.notify("exit");
      this.server.closeInput();
    } catch {
      // fall through to the forced exit below
    }
    await this.server.terminateAndWait(3_000);
    liveSessions.delete(this);
  }
}

function positionOf(text, needle, offsetInNeedle = 0) {
  const offset = text.indexOf(needle);
  if (offset < 0) throw new Error(`missing ${needle}`);
  const prefix = text.slice(0, offset + offsetInNeedle);
  const lines = prefix.split("\n");
  return { line: lines.length - 1, character: lines.at(-1).length };
}

const countOf = (result) => (Array.isArray(result) ? result.length : result ? 1 : 0);

async function main() {
  const tempRoot = await fs.realpath(await fs.mkdtemp(path.join(os.tmpdir(), "sage-perf-baseline-")));
  const workspaceRoot = path.join(tempRoot, "workspace");
  const cacheDir = path.join(tempRoot, "cache");
  await fs.mkdir(workspaceRoot, { recursive: true });
  await fs.mkdir(cacheDir, { recursive: true });
  const demoPath = path.join(workspaceRoot, "demo.sage");
  const demoText = [
    "from sage.all import matrix, QQ, PolynomialRing, Graph",
    "",
    "R = PolynomialRing(QQ, 'x')",
    "M = matrix(QQ, 2, 2, [1, 2, 3, 4])",
    "G = Graph()",
    "value = M.determinant()",
    "G.add_vertex(1)",
    "",
  ].join("\n");
  await fs.writeFile(demoPath, demoText);
  const demoUri = pathToFileURL(demoPath).toString();
  const largePath = path.join(sageSourceRoot, "sage", "graphs", "generic_graph.py");
  const largeText = await fs.readFile(largePath, "utf8");
  const largeUri = pathToFileURL(largePath).toString();
  const context = { workspaceRoot, cacheDir };
  const report = {
    status: "ok",
    recorded_at: new Date().toISOString(),
    profile,
    platform: `${process.platform}-${process.arch}`,
    cpus: os.cpus().length,
    samples,
    sage_source_root_files: null,
    large_file: { path: path.relative(sageSourceRoot, largePath), lines: largeText.split("\n").length, bytes: largeText.length },
  };
  const hoverParams = (uri, position) => ({ textDocument: { uri }, position });

  // --- 1. Cold start: empty cache, requests issued while the first index pass runs. -----------
  let session = new Session(context);
  await session.start();
  session.open(demoUri, demoText, "sagemath");
  const duringIndexHover = [];
  const duringIndexCompletion = [];
  let saveFreezeHoverMs = null;
  const coldLoop = (async () => {
    let iteration = 0;
    for (;;) {
      const status = await session.status();
      if ((status?.pending_jobs ?? 0) === 0 && (status?.indexed_file_count ?? 0) > 1000) break;
      duringIndexHover.push((await session.timed("textDocument/hover", hoverParams(demoUri, positionOf(demoText, "matrix(QQ", 2)))).ms);
      duringIndexCompletion.push((await session.timed("textDocument/completion", hoverParams(demoUri, positionOf(demoText, "M.determinant", 4)))).ms);
      if (iteration === 2) {
        // Saves while indexing used to occupy every handler slot until the index pass finished.
        for (let save = 0; save < 6; save += 1) {
          session.server.notify("textDocument/didSave", { textDocument: { uri: demoUri }, text: demoText });
        }
        saveFreezeHoverMs = (await session.timed("textDocument/hover", hoverParams(demoUri, positionOf(demoText, "matrix(QQ", 2)))).ms;
      }
      iteration += 1;
      await sleep(25);
    }
  })();
  await coldLoop;
  const cold = await session.waitForIndex();
  report.sage_source_root_files = cold.status.indexed_file_count;
  report.cold_start = {
    initialize_ms: round(session.initializeMs),
    index_ready_ms: cold.ready_ms,
    symbols: cold.status.symbol_count,
    rss_after_ready_mb: session.rss(),
    peak_rss_mb: round(session.peakRssMb),
    cache_dir_mb: await directorySizeMb(cacheDir),
    hover_during_indexing: summarize(duringIndexHover),
    completion_during_indexing: summarize(duringIndexCompletion),
    hover_after_six_saves_during_indexing_ms: saveFreezeHoverMs === null ? null : round(saveFreezeHoverMs),
  };
  await session.stop();

  // --- 2. Warm start: persistent cache already populated. --------------------------------------
  session = new Session(context);
  await session.start();
  session.open(demoUri, demoText, "sagemath");
  const firstHover = await session.timed("textDocument/hover", hoverParams(demoUri, positionOf(demoText, "matrix(QQ", 2)));
  const warm = await session.waitForIndex();
  report.warm_start = {
    initialize_ms: round(session.initializeMs),
    first_hover_ms: round(firstHover.ms),
    index_ready_ms: warm.ready_ms,
    rss_after_ready_mb: session.rss(),
  };

  // --- 3. Warm request latency on a small workspace file. --------------------------------------
  const warmRequests = {};
  const strip = ({ lastResult, ...rest }) => rest;
  warmRequests.hover = strip(await session.sample("textDocument/hover", hoverParams(demoUri, positionOf(demoText, "matrix(QQ", 2))));
  warmRequests.definition = strip(await session.sample("textDocument/definition", hoverParams(demoUri, positionOf(demoText, "PolynomialRing(QQ", 3))));
  warmRequests.completion = strip(await session.sample("textDocument/completion", hoverParams(demoUri, positionOf(demoText, "M.determinant", 4))));
  warmRequests.document_symbol = strip(await session.sample("textDocument/documentSymbol", { textDocument: { uri: demoUri } }));
  warmRequests.semantic_tokens_full = strip(await session.sample("textDocument/semanticTokens/full", { textDocument: { uri: demoUri } }));
  report.warm_requests_small_file = warmRequests;

  // --- 4. Project-wide references for common names. ---------------------------------------------
  report.references = {};
  for (const [name, needle, offset] of [["matrix", "matrix(QQ", 2], ["QQ", "QQ, 'x'", 1]]) {
    const rssBefore = session.rss();
    const run = await session.timed("textDocument/references", {
      ...hoverParams(demoUri, positionOf(demoText, needle, offset)),
      context: { includeDeclaration: false },
    });
    // A hover issued right behind the references request shows whether it blocks other requests.
    const pendingReferences = session.timed("textDocument/references", {
      ...hoverParams(demoUri, positionOf(demoText, needle, offset)),
      context: { includeDeclaration: false },
    });
    const concurrentHover = await session.timed("textDocument/hover", hoverParams(demoUri, positionOf(demoText, "Graph()", 2)));
    await pendingReferences;
    report.references[name] = {
      ms: round(run.ms),
      locations: countOf(run.result),
      error: run.error,
      hover_while_running_ms: round(concurrentHover.ms),
      rss_before_mb: rssBefore,
      rss_after_mb: session.rss(),
    };
  }

  // --- 5. Large file: open, semantic tokens, rapid typing. --------------------------------------
  const openDiagnostics = session.nextDiagnostics(largeUri);
  const openStarted = performance.now();
  session.open(largeUri, largeText, "python");
  const openDiagnosticsAt = await openDiagnostics;
  const large = {
    open_to_diagnostics_ms: openDiagnosticsAt === null ? null : round(openDiagnosticsAt - openStarted),
  };
  const tokens = await session.sample("textDocument/semanticTokens/full", { textDocument: { uri: largeUri } }, 5);
  large.semantic_tokens_full = strip(tokens);
  large.semantic_token_count = tokens.lastResult?.data ? tokens.lastResult.data.length / 5 : null;
  large.semantic_tokens_range = strip(await session.sample("textDocument/semanticTokens/range", {
    textDocument: { uri: largeUri },
    range: { start: { line: 12_000, character: 0 }, end: { line: 12_060, character: 0 } },
  }, 5));
  large.document_symbol = strip(await session.sample("textDocument/documentSymbol", { textDocument: { uri: largeUri } }, 5));
  large.folding_range = strip(await session.sample("textDocument/foldingRange", { textDocument: { uri: largeUri } }, 5));
  const largeHoverPosition = positionOf(largeText, "def add_vertex(self", 6);
  large.hover = strip(await session.sample("textDocument/hover", hoverParams(largeUri, largeHoverPosition)));

  // 100 single-character edits at 30 ms intervals in the middle of the file.
  const typingLine = 12_000;
  const typingHover = [];
  const typingCompletion = [];
  const inFlight = [];
  let version = 1;
  const typingStarted = performance.now();
  let lastEditAt = typingStarted;
  for (let edit = 0; edit < 100; edit += 1) {
    version += 1;
    lastEditAt = performance.now();
    session.server.notify("textDocument/didChange", {
      textDocument: { uri: largeUri, version },
      contentChanges: [{
        range: { start: { line: typingLine, character: 0 }, end: { line: typingLine, character: 0 } },
        text: "x",
      }],
    });
    if (edit % 10 === 5) {
      inFlight.push(session.timed("textDocument/hover", hoverParams(largeUri, largeHoverPosition)).then((r) => typingHover.push(r.ms)));
      inFlight.push(session.timed("textDocument/completion", hoverParams(largeUri, { line: typingLine, character: 1 })).then((r) => typingCompletion.push(r.ms)));
    }
    await sleep(30);
  }
  const lastPublishAt = await session.nextDiagnostics(largeUri, 5_000, version);
  await Promise.all(inFlight);
  const tokensAfterTyping = await session.timed("textDocument/semanticTokens/full", { textDocument: { uri: largeUri } });
  large.rapid_typing = {
    edits: 100,
    interval_ms: 30,
    typing_duration_ms: round(lastEditAt - typingStarted),
    final_diagnostics_version: session.lastDiagnosticVersions.get(largeUri),
    last_edit_to_final_diagnostics_ms: lastPublishAt === null ? null : round(lastPublishAt - lastEditAt),
    hover_while_typing: summarize(typingHover),
    completion_while_typing: summarize(typingCompletion),
    semantic_tokens_after_typing_ms: round(tokensAfterTyping.ms),
  };
  session.server.notify("textDocument/didClose", { textDocument: { uri: largeUri } });
  report.large_file = { ...report.large_file, ...large };

  // --- 6. Burst of watched-file events (e.g. `git checkout` inside the Sage tree). --------------
  const burstPaths = [];
  const graphsDir = path.join(sageSourceRoot, "sage", "graphs");
  for (const entry of await fs.readdir(graphsDir)) {
    if (entry.endsWith(".py") && burstPaths.length < 60) burstPaths.push(path.join(graphsDir, entry));
  }
  const burstStarted = performance.now();
  session.server.notify("workspace/didChangeWatchedFiles", {
    changes: burstPaths.map((file) => ({ uri: pathToFileURL(file).toString(), type: 2 })),
  });
  const burstHover = await session.timed("textDocument/hover", hoverParams(demoUri, positionOf(demoText, "matrix(QQ", 2)));
  await sleep(100);
  await session.waitForIndex();
  report.watched_file_burst = {
    files: burstPaths.length,
    hover_right_after_burst_ms: round(burstHover.ms),
    settle_ms: round(performance.now() - burstStarted),
    rss_after_mb: session.rss(),
  };

  // --- 7. Explicit rebuild: memory and duration. -------------------------------------------------
  const rssBeforeRebuild = session.rss();
  session.peakRssMb = 0;
  const rebuild = await session.timed("workspace/executeCommand", { command: "sage.__rust.rebuildIndex", arguments: [] });
  await sleep(100);
  await session.waitForIndex();
  const hoverAfterRebuild = strip(await session.sample("textDocument/hover", hoverParams(demoUri, positionOf(demoText, "matrix(QQ", 2))));
  report.rebuild = {
    command_ms: round(rebuild.ms),
    rss_before_mb: rssBeforeRebuild,
    rss_after_mb: session.rss(),
    peak_rss_mb: round(session.peakRssMb),
    hover_after_rebuild: hoverAfterRebuild,
    cache_dir_mb: await directorySizeMb(cacheDir),
  };
  const stopStarted = performance.now();
  await session.stop();
  report.shutdown_ms = round(performance.now() - stopStarted);

  await fs.rm(tempRoot, { recursive: true, force: true });
  const json = `${JSON.stringify(report, null, 2)}\n`;
  if (args.out) {
    await fs.mkdir(path.dirname(path.resolve(args.out)), { recursive: true });
    await fs.writeFile(path.resolve(args.out), json);
  }
  process.stdout.write(json);
}

main().catch((error) => {
  console.error(error);
  for (const session of liveSessions) session.server?.terminate();
  process.exit(1);
});
