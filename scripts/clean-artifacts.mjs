#!/usr/bin/env node
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const repositoryRoot = path.resolve(__dirname, "..");

const args = parseArgs(process.argv.slice(2));
const dryRun = args.yes !== true;
const includeDeps = args.deps === true;
const json = args.json === true;

const fixedPaths = [
  "target",
  "dist",
  "out",
  "build",
  "coverage",
  "htmlcov",
  "packages/extension-core/out",
  "packages/extension-core/.vscode-test",
  "packages/syntax-pack/out",
  "packages/sage-lsp/build",
  "packages/sage-lsp/.pytest_cache",
  ".pytest_cache",
  ".coverage",
];

const dependencyPaths = [
  "node_modules",
  "packages/extension-core/node_modules",
  "packages/syntax-pack/node_modules",
  ".venv",
  "venv",
];

const directoryNames = new Set([
  "__pycache__",
  ".pytest_cache",
  ".vscode-test",
]);

const filePatterns = [
  /\.py[cod]$/,
  /\.sage\.py$/,
  /\.log$/,
  /^npm-debug\.log/,
  /^yarn-debug\.log/,
  /^yarn-error\.log/,
  /^pnpm-debug\.log/,
  /\.vsix$/,
  /\.tgz$/,
];

const report = buildReport();

if (json) {
  console.log(JSON.stringify(report, null, 2));
} else {
  printHuman(report);
}

if (report.failures.length > 0) {
  process.exitCode = 1;
}

function buildReport() {
  const actions = [];
  const failures = [];
  const seen = new Set();

  for (const relativePath of fixedPaths) {
    collectPath(relativePath, "build-artifact");
  }

  if (includeDeps) {
    for (const relativePath of dependencyPaths) {
      collectPath(relativePath, "dependency");
    }
  }

  walk(repositoryRoot, (absolutePath, directoryEntry) => {
    const relativePath = toRelative(absolutePath);
    if (!relativePath || shouldSkipWalk(relativePath)) {
      return "skip";
    }
  if (directoryEntry.isDirectory() && directoryNames.has(directoryEntry.name)) {
      collectPath(relativePath, "generated-directory");
      return "skip";
    }
    if (isPackagedRustBinary(relativePath, directoryEntry.name)) {
      collectPath(relativePath, "packaged-rust-binary");
    }
    if (directoryEntry.isFile() && filePatterns.some((pattern) => pattern.test(directoryEntry.name))) {
      collectPath(relativePath, "generated-file");
    }
    return "continue";
  });

  for (const action of actions) {
    if (dryRun) {
      continue;
    }
    try {
      fs.rmSync(action.absolute_path, { recursive: true, force: true });
      action.deleted = true;
    } catch (error) {
      action.deleted = false;
      failures.push(`failed to remove ${action.relative_path}: ${messageOf(error)}`);
    }
  }

  return {
    schema_version: 1,
    mode: dryRun ? "dry-run" : "delete",
    repository_root: repositoryRoot,
    include_dependencies: includeDeps,
    actions,
    totals: {
      candidate_count: actions.length,
      candidate_bytes: actions.reduce((sum, action) => sum + action.size_bytes, 0),
      deleted_count: actions.filter((action) => action.deleted === true).length,
    },
    failures,
  };

  function collectPath(relativePath, reason) {
    const normalized = normalizeRelative(relativePath);
    const absolutePath = path.resolve(repositoryRoot, normalized);
    if (!isInsideRepository(absolutePath) || seen.has(normalized) || !fs.existsSync(absolutePath)) {
      return;
    }
    let stats;
    try {
      stats = fs.lstatSync(absolutePath);
    } catch (error) {
      failures.push(`failed to stat ${normalized}: ${messageOf(error)}`);
      return;
    }
    seen.add(normalized);
    actions.push({
      relative_path: normalized,
      absolute_path: absolutePath,
      reason,
      kind: stats.isDirectory() ? "directory" : stats.isSymbolicLink() ? "symlink" : "file",
      size_bytes: estimateSize(absolutePath, stats),
      deleted: false,
    });
  }
}

function walk(root, visit) {
  let entries;
  try {
    entries = fs.readdirSync(root, { withFileTypes: true });
  } catch {
    return;
  }
  for (const entry of entries) {
    const absolutePath = path.join(root, entry.name);
    const decision = visit(absolutePath, entry);
    if (decision === "skip" || !entry.isDirectory()) {
      continue;
    }
    walk(absolutePath, visit);
  }
}

function isPackagedRustBinary(relativePath, name) {
  return (
    /^packages\/extension-core\/resources\/bin\/[^/]+\/sage-ls(?:\.meta\.json|\.sha256)?$/.test(relativePath)
    || (relativePath.startsWith("packages/extension-core/resources/bin/") && name === "sage-ls")
  );
}

function shouldSkipWalk(relativePath) {
  return (
    relativePath === ".git"
    || relativePath.startsWith(".git/")
    || relativePath === "node_modules"
    || relativePath.endsWith("/node_modules")
    || relativePath === "target"
    || relativePath.startsWith("target/")
    || relativePath === "dist"
    || relativePath.startsWith("dist/")
    || relativePath === ".venv"
    || relativePath === "venv"
  );
}

function estimateSize(absolutePath, stats) {
  if (!stats.isDirectory()) {
    return stats.size;
  }
  let total = 0;
  walk(absolutePath, (childPath, entry) => {
    try {
      const childStats = fs.lstatSync(childPath);
      total += childStats.size;
      return entry.isDirectory() && !entry.isSymbolicLink() ? "continue" : "skip";
    } catch {
      return "skip";
    }
  });
  return total;
}

function printHuman(report) {
  console.log(`Sage VS Code artifact cleanup (${report.mode})`);
  console.log(`Repository: ${report.repository_root}`);
  console.log(`Include dependencies: ${report.include_dependencies ? "yes" : "no"}`);
  console.log("");

  if (report.actions.length === 0) {
    console.log("No cleanup candidates found.");
  } else {
    for (const action of report.actions.sort((a, b) => a.relative_path.localeCompare(b.relative_path))) {
      const marker = report.mode === "dry-run" ? "would remove" : action.deleted ? "removed" : "failed";
      console.log(`${marker}: ${action.relative_path} (${formatBytes(action.size_bytes)}, ${action.reason})`);
    }
  }

  console.log("");
  console.log(`Candidates: ${report.totals.candidate_count}`);
  console.log(`Total size: ${formatBytes(report.totals.candidate_bytes)}`);
  if (report.mode === "dry-run") {
    console.log("Run `npm run clean -- --yes` to delete these artifacts.");
    console.log("Add `--deps` only when you also want to remove dependencies and virtual environments.");
  }
}

function parseArgs(values) {
  const result = {};
  for (const value of values) {
    if (value === "--yes") {
      result.yes = true;
    } else if (value === "--deps") {
      result.deps = true;
    } else if (value === "--json") {
      result.json = true;
    } else if (value === "--help" || value === "-h") {
      printUsageAndExit();
    } else {
      throw new Error(`Unknown argument: ${value}`);
    }
  }
  return result;
}

function printUsageAndExit() {
  console.log(`Usage: node scripts/clean-artifacts.mjs [--yes] [--deps] [--json]

Default mode is a dry run. Pass --yes to delete cleanup candidates.
Pass --deps to also include node_modules and virtualenvs.`);
  process.exit(0);
}

function normalizeRelative(relativePath) {
  return relativePath.split(path.sep).join("/");
}

function toRelative(absolutePath) {
  return normalizeRelative(path.relative(repositoryRoot, absolutePath));
}

function isInsideRepository(absolutePath) {
  const relativePath = path.relative(repositoryRoot, absolutePath);
  return relativePath === "" || (!relativePath.startsWith("..") && !path.isAbsolute(relativePath));
}

function formatBytes(bytes) {
  if (bytes < 1024) {
    return `${bytes} B`;
  }
  if (bytes < 1024 * 1024) {
    return `${(bytes / 1024).toFixed(1)} KiB`;
  }
  if (bytes < 1024 * 1024 * 1024) {
    return `${(bytes / 1024 / 1024).toFixed(1)} MiB`;
  }
  return `${(bytes / 1024 / 1024 / 1024).toFixed(1)} GiB`;
}

function messageOf(error) {
  return error instanceof Error ? error.message : String(error);
}
