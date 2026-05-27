#!/usr/bin/env node
import { execFile } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { promisify } from "node:util";

const execFileAsync = promisify(execFile);
const __dirname = path.dirname(fileURLToPath(import.meta.url));
const repositoryRoot = path.resolve(__dirname, "..");
const cargoCommand = process.platform === "win32" ? "cargo.exe" : "cargo";
const npmCommand = process.platform === "win32" ? "npm.cmd" : "npm";
const benchBinary = path.join(
  repositoryRoot,
  "target",
  "release",
  process.platform === "win32" ? "sage-index-bench.exe" : "sage-index-bench",
);
const args = process.argv.slice(2);

const budgets = {
  parseMs: numberFromEnv("SAGE_PERF_PARSE_MS", 1500),
  rebuildElapsedMs: numberFromEnv("SAGE_PERF_REBUILD_ELAPSED_MS", 5000),
  rebuildInternalMs: numberFromEnv("SAGE_PERF_REBUILD_INTERNAL_MS", 1500),
  hydrateMs: numberFromEnv("SAGE_PERF_HYDRATE_MS", 300),
};

const sourceRoot = resolveSourceRoot();
const includeWorkbench = !args.includes("--skip-workbench") && process.env.SAGE_PERF_SKIP_WORKBENCH !== "1";
const performanceCacheHome = process.env.SAGE_PERF_CACHE_DIR
  ? path.resolve(process.env.SAGE_PERF_CACHE_DIR)
  : path.join(defaultPerformanceCacheRoot(), `sage-vscode-performance-smoke-${process.pid}`);

if (!sourceRoot || !fs.existsSync(path.join(sourceRoot, "sage"))) {
  console.log(JSON.stringify({
    schema_version: 1,
    skipped: true,
    reason: "missing Sage source root",
    expected: sourceRoot ?? "set SAGE_SOURCE_ROOT or pass --source-root <path>",
  }, null, 2));
  process.exit(0);
}

const result = {
  schema_version: 1,
  skipped: false,
  source_root: sourceRoot,
  cache_home: performanceCacheHome,
  budgets,
  checks: [],
  failures: [],
};

try {
  if (process.env.SAGE_PERF_KEEP_CACHE !== "1") {
    fs.rmSync(performanceCacheHome, { recursive: true, force: true });
  }
  await runCommand(cargoCommand, ["build", "--release", "-p", "sage-index", "--bin", "sage-index-bench"]);
  const parse = await runBench("parse", { SAGE_INDEX_BENCH_PARSE_ONLY: "1" });
  result.checks.push(check("parse_ms", parse.parse_ms, budgets.parseMs, parse));

  const rebuild = await runBench("rebuild");
  result.checks.push(check("rebuild_elapsed_ms", rebuild.elapsed_ms, budgets.rebuildElapsedMs, rebuild));
  result.checks.push(check("rebuild_internal_ms", rebuild.status?.last_index_ms, budgets.rebuildInternalMs, rebuild));

  const hydrate = await runBench("hydrate", { SAGE_INDEX_BENCH_HYDRATE_ONLY: "1" });
  result.checks.push(check("hydrate_ms", hydrate.hydrate_ms, budgets.hydrateMs, hydrate));

  if (includeWorkbench) {
    const started = Date.now();
    await runCommand(npmCommand, ["run", "test:debug-web"]);
    result.checks.push({
      name: "debug_workbench_smoke",
      actual: Date.now() - started,
      budget: null,
      pass: true,
    });
  } else {
    result.checks.push({
      name: "debug_workbench_smoke",
      skipped: true,
      reason: "disabled by --skip-workbench or SAGE_PERF_SKIP_WORKBENCH=1",
    });
  }
} catch (error) {
  result.failures.push(String(error instanceof Error ? error.message : error));
}

for (const item of result.checks) {
  if (item.pass === false) {
    result.failures.push(`${item.name}: ${item.actual} > ${item.budget}`);
  }
}

console.log(JSON.stringify(result, null, 2));
if (result.failures.length > 0) {
  process.exitCode = 1;
}

async function runBench(mode, extraEnv = {}) {
  const output = await runCommand(
    benchBinary,
    [sourceRoot],
    {
      XDG_CACHE_HOME: performanceCacheHome,
      ...extraEnv,
    },
  );
  return parseJsonOutput(output.stdout, mode);
}

async function runCommand(command, commandArgs, extraEnv = {}) {
  try {
    return await execFileAsync(command, commandArgs, {
      cwd: repositoryRoot,
      env: {
        ...process.env,
        ...extraEnv,
      },
      maxBuffer: 20 * 1024 * 1024,
    });
  } catch (error) {
    const stdout = error?.stdout ? `\nstdout:\n${error.stdout}` : "";
    const stderr = error?.stderr ? `\nstderr:\n${error.stderr}` : "";
    throw new Error(`${command} ${commandArgs.join(" ")} failed${stdout}${stderr}`);
  }
}

function parseJsonOutput(stdout, mode) {
  const trimmed = stdout.trim();
  const jsonStart = trimmed.indexOf("{");
  if (jsonStart < 0) {
    throw new Error(`bench ${mode} did not print JSON`);
  }
  return JSON.parse(trimmed.slice(jsonStart));
}

function check(name, actual, budget, payload) {
  return {
    name,
    actual,
    budget,
    pass: typeof actual === "number" && actual <= budget,
    mode: payload?.mode,
  };
}

function resolveSourceRoot() {
  const explicitIndex = args.indexOf("--source-root");
  if (explicitIndex >= 0) {
    return path.resolve(args[explicitIndex + 1] ?? "");
  }
  if (process.env.SAGE_SOURCE_ROOT) {
    return path.resolve(process.env.SAGE_SOURCE_ROOT);
  }
  return path.resolve(repositoryRoot, "..", "sage", "src");
}

function defaultPerformanceCacheRoot() {
  if (process.platform === "darwin" && fs.existsSync("/tmp")) {
    return "/tmp";
  }
  return os.tmpdir();
}

function numberFromEnv(name, fallback) {
  const raw = process.env[name];
  if (!raw) {
    return fallback;
  }
  const parsed = Number(raw);
  return Number.isFinite(parsed) ? parsed : fallback;
}
