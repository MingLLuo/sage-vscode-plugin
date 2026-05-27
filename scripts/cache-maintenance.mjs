#!/usr/bin/env node
import fs from "node:fs";
import path from "node:path";
import os from "node:os";

const DATABASE_PATTERN = /^sage-index-([a-f0-9]{16})\.sqlite$/;
const SIDECAR_PATTERN = /^sage-index-([a-f0-9]{16})\.sqlite-(wal|shm)$/;
const DAY_MS = 24 * 60 * 60 * 1000;
const DEFAULT_MAX_TOTAL_BYTES = 2 * 1024 * 1024 * 1024;

const args = parseArgs(process.argv.slice(2));
const cacheDir = args.cacheDir ?? defaultCacheDir();
const maxAgeDays = args.maxAgeDays ?? 30;
const maxTotalBytes = args.maxTotalBytes ?? DEFAULT_MAX_TOTAL_BYTES;
const keepLatestDatabases = args.keepLatestDatabases ?? 2;
const orphanMaxAgeDays = args.orphanMaxAgeDays ?? 1;
const sizePruneMinAgeDays = args.sizePruneMinAgeDays ?? 0;
const prune = args.prune === true;
const dryRun = prune ? args.yes !== true : true;
const now = Date.now();

const report = buildReport(cacheDir, {
  maxAgeDays,
  maxTotalBytes,
  keepLatestDatabases,
  orphanMaxAgeDays,
  sizePruneMinAgeDays,
  prune,
  dryRun,
  now,
});

if (args.json === true) {
  console.log(JSON.stringify(report, null, 2));
} else {
  printHuman(report);
}

if (report.failures.length > 0) {
  process.exitCode = 1;
}

function buildReport(root, options) {
  const result = {
    schema_version: 1,
    cache_dir: root,
    exists: fs.existsSync(root),
    mode: options.prune ? "prune" : "inventory",
    dry_run: options.dryRun,
    max_age_days: options.maxAgeDays,
    max_total_bytes: options.maxTotalBytes,
    keep_latest_databases: options.keepLatestDatabases,
    orphan_max_age_days: options.orphanMaxAgeDays,
    size_prune_min_age_days: options.sizePruneMinAgeDays,
    entries: [],
    orphan_sidecars: [],
    actions: [],
    totals: {
      database_count: 0,
      sidecar_count: 0,
      orphan_sidecar_count: 0,
      total_bytes: 0,
      prunable_database_count: 0,
      prunable_orphan_sidecar_count: 0,
      prunable_bytes: 0,
      deleted_file_count: 0,
      deleted_bytes: 0,
    },
    failures: [],
  };

  if (!result.exists) {
    return result;
  }

  let names;
  try {
    names = fs.readdirSync(root);
  } catch (error) {
    result.failures.push(`cannot read cache dir ${root}: ${messageOf(error)}`);
    return result;
  }

  const sidecarsByDigest = new Map();
  for (const name of names) {
    const sidecarMatch = SIDECAR_PATTERN.exec(name);
    if (!sidecarMatch) {
      continue;
    }
    const digest = sidecarMatch[1];
    const record = fileRecord(root, name, options.now, result.failures);
    if (!record) {
      continue;
    }
    const list = sidecarsByDigest.get(digest) ?? [];
    list.push({
      ...record,
      kind: sidecarMatch[2],
      prunable: false,
      prune_reasons: [],
    });
    sidecarsByDigest.set(digest, list);
  }

  for (const name of names) {
    const databaseMatch = DATABASE_PATTERN.exec(name);
    if (!databaseMatch) {
      continue;
    }
    const digest = databaseMatch[1];
    const database = fileRecord(root, name, options.now, result.failures);
    if (!database) {
      continue;
    }
    const sidecars = (sidecarsByDigest.get(digest) ?? []).sort((a, b) => a.name.localeCompare(b.name));
    sidecarsByDigest.delete(digest);
    const totalBytes = database.size_bytes + sidecars.reduce((sum, item) => sum + item.size_bytes, 0);
    const entry = {
      ...database,
      digest,
      sidecars,
      total_bytes: totalBytes,
      protected: false,
      prunable: false,
      prune_reasons: [],
    };
    result.entries.push(entry);
    result.totals.database_count += 1;
    result.totals.sidecar_count += sidecars.length;
    result.totals.total_bytes += totalBytes;
  }

  result.entries.sort((a, b) => b.mtime_ms - a.mtime_ms);

  for (const sidecars of sidecarsByDigest.values()) {
    for (const sidecar of sidecars) {
      result.orphan_sidecars.push(sidecar);
      result.totals.orphan_sidecar_count += 1;
      result.totals.total_bytes += sidecar.size_bytes;
    }
  }
  result.orphan_sidecars.sort((a, b) => b.mtime_ms - a.mtime_ms);

  markPrunableEntries(result);

  if (options.prune) {
    pruneEntries(result, options.dryRun);
  }

  return result;
}

function markPrunableEntries(report) {
  const protectedDigests = new Set(
    report.entries
      .slice(0, report.keep_latest_databases)
      .map((entry) => entry.digest),
  );
  for (const entry of report.entries) {
    entry.protected = protectedDigests.has(entry.digest);
  }

  let projectedTotalBytes = report.totals.total_bytes;
  for (const entry of [...report.entries].reverse()) {
    if (entry.protected || entry.age_days < report.max_age_days) {
      continue;
    }
    markDatabasePrunable(entry, "age");
    projectedTotalBytes -= entry.total_bytes;
  }

  if (projectedTotalBytes > report.max_total_bytes) {
    for (const entry of [...report.entries].reverse()) {
      if (
        projectedTotalBytes <= report.max_total_bytes
        || entry.protected
        || entry.prunable
        || entry.age_days < report.size_prune_min_age_days
      ) {
        continue;
      }
      markDatabasePrunable(entry, "size-cap");
      projectedTotalBytes -= entry.total_bytes;
    }
  }

  for (const sidecar of report.orphan_sidecars) {
    if (sidecar.age_days >= report.orphan_max_age_days) {
      markSidecarPrunable(sidecar, "orphan");
      projectedTotalBytes -= sidecar.size_bytes;
    }
  }

  if (projectedTotalBytes > report.max_total_bytes) {
    for (const sidecar of [...report.orphan_sidecars].reverse()) {
      if (
        projectedTotalBytes <= report.max_total_bytes
        || sidecar.prunable
        || sidecar.age_days < report.size_prune_min_age_days
      ) {
        continue;
      }
      markSidecarPrunable(sidecar, "size-cap");
      projectedTotalBytes -= sidecar.size_bytes;
    }
  }

  for (const entry of report.entries) {
    if (entry.prunable) {
      report.totals.prunable_database_count += 1;
      report.totals.prunable_bytes += entry.total_bytes;
    }
  }
  for (const sidecar of report.orphan_sidecars) {
    if (sidecar.prunable) {
      report.totals.prunable_orphan_sidecar_count += 1;
      report.totals.prunable_bytes += sidecar.size_bytes;
    }
  }
}

function markDatabasePrunable(entry, reason) {
  entry.prunable = true;
  if (!entry.prune_reasons.includes(reason)) {
    entry.prune_reasons.push(reason);
  }
  for (const sidecar of entry.sidecars) {
    markSidecarPrunable(sidecar, reason);
  }
}

function markSidecarPrunable(sidecar, reason) {
  sidecar.prunable = true;
  if (!sidecar.prune_reasons.includes(reason)) {
    sidecar.prune_reasons.push(reason);
  }
}

function pruneEntries(report, dryRun) {
  for (const entry of report.entries) {
    if (!entry.prunable) {
      continue;
    }
    const reason = entry.prune_reasons.join(",") || "prune";
    for (const item of [entry, ...entry.sidecars]) {
      recordDeleteAction(report, item, item.prune_reasons?.join(",") || reason, dryRun);
    }
  }
  for (const sidecar of report.orphan_sidecars) {
    if (!sidecar.prunable) {
      continue;
    }
    recordDeleteAction(report, sidecar, sidecar.prune_reasons.join(",") || "orphan", dryRun);
  }
}

function recordDeleteAction(report, item, reason, dryRun) {
  const action = {
    action: dryRun ? "would_delete" : "delete",
    path: item.path,
    size_bytes: item.size_bytes,
    reason,
  };
  if (!dryRun) {
    try {
      fs.unlinkSync(item.path);
      report.totals.deleted_file_count += 1;
      report.totals.deleted_bytes += item.size_bytes;
    } catch (error) {
      action.error = messageOf(error);
      report.failures.push(`cannot delete ${item.path}: ${action.error}`);
    }
  }
  report.actions.push(action);
}

function fileRecord(root, name, nowMs, failures) {
  const absolutePath = path.join(root, name);
  let stats;
  try {
    stats = fs.lstatSync(absolutePath);
  } catch (error) {
    failures.push(`cannot stat ${absolutePath}: ${messageOf(error)}`);
    return undefined;
  }
  if (!stats.isFile()) {
    failures.push(`refusing non-file cache entry ${absolutePath}`);
    return undefined;
  }
  return {
    name,
    path: absolutePath,
    size_bytes: stats.size,
    mtime_ms: Math.trunc(stats.mtimeMs),
    mtime: stats.mtime.toISOString(),
    age_days: Math.max(0, Math.floor((nowMs - stats.mtimeMs) / DAY_MS)),
  };
}

function printHuman(report) {
  console.log(`Sage Rust index cache: ${report.cache_dir}`);
  if (!report.exists) {
    console.log("status: missing");
    return;
  }
  console.log(`status: ${report.mode}${report.mode === "prune" && report.dry_run ? " dry-run" : ""}`);
  console.log(
    `databases: ${report.totals.database_count}, sidecars: ${report.totals.sidecar_count}, ` +
      `orphan sidecars: ${report.totals.orphan_sidecar_count}, total: ${formatBytes(report.totals.total_bytes)}`,
  );
  console.log(
    `policy: keep latest ${report.keep_latest_databases}, max age ${report.max_age_days}d, ` +
      `max total ${formatBytes(report.max_total_bytes)}, orphan sidecars ${report.orphan_max_age_days}d`,
  );
  console.log(
    `prunable: ${report.totals.prunable_database_count} database(s), ` +
      `${report.totals.prunable_orphan_sidecar_count} orphan sidecar(s), ` +
      `${formatBytes(report.totals.prunable_bytes)}`,
  );
  if (report.mode === "prune") {
    if (report.dry_run) {
      console.log("no files were deleted; pass --yes with --prune to apply");
    } else {
      console.log(`deleted: ${report.totals.deleted_file_count} file(s), ${formatBytes(report.totals.deleted_bytes)}`);
    }
  }
  for (const entry of report.entries.slice(0, 12)) {
    const marker = entry.prunable ? `prunable:${entry.prune_reasons.join(",")}` : "keep";
    const protectedMarker = entry.protected ? " protected" : "";
    console.log(
      `- ${entry.name} ${formatBytes(entry.total_bytes)} age=${entry.age_days}d ` +
        `sidecars=${entry.sidecars.length}${protectedMarker} ${marker}`,
    );
  }
  if (report.entries.length > 12) {
    console.log(`... ${report.entries.length - 12} more database(s)`);
  }
  if (report.orphan_sidecars.length > 0) {
    console.log(`orphan sidecars: ${report.orphan_sidecars.length}`);
  }
  for (const failure of report.failures) {
    console.error(`error: ${failure}`);
  }
}

function defaultCacheDir() {
  const base = process.env.XDG_CACHE_HOME
    ? path.resolve(process.env.XDG_CACHE_HOME)
    : process.env.HOME
      ? path.join(process.env.HOME, ".cache")
      : os.tmpdir();
  return path.join(base, "sage-vscode-plugin", "rust-index-v2");
}

function parseArgs(rawArgs) {
  const parsed = {};
  for (let index = 0; index < rawArgs.length; index += 1) {
    const item = rawArgs[index];
    if (item === "--help" || item === "-h") {
      printUsage();
      process.exit(0);
    }
    if (item === "--json") {
      parsed.json = true;
      continue;
    }
    if (item === "--prune") {
      parsed.prune = true;
      continue;
    }
    if (item === "--yes") {
      parsed.yes = true;
      continue;
    }
    if (item === "--cache-dir") {
      parsed.cacheDir = path.resolve(requiredArg(rawArgs, index, item));
      index += 1;
      continue;
    }
    if (item === "--max-age-days") {
      parsed.maxAgeDays = parseNonNegativeNumber(requiredArg(rawArgs, index, item), item);
      index += 1;
      continue;
    }
    if (item === "--max-total-bytes") {
      parsed.maxTotalBytes = parseNonNegativeNumber(requiredArg(rawArgs, index, item), item);
      index += 1;
      continue;
    }
    if (item === "--keep-latest") {
      parsed.keepLatestDatabases = parseNonNegativeInteger(requiredArg(rawArgs, index, item), item);
      index += 1;
      continue;
    }
    if (item === "--orphan-max-age-days") {
      parsed.orphanMaxAgeDays = parseNonNegativeNumber(requiredArg(rawArgs, index, item), item);
      index += 1;
      continue;
    }
    if (item === "--size-prune-min-age-days") {
      parsed.sizePruneMinAgeDays = parseNonNegativeNumber(requiredArg(rawArgs, index, item), item);
      index += 1;
      continue;
    }
    fail(`unknown argument: ${item}`);
  }
  return parsed;
}

function requiredArg(rawArgs, index, item) {
  const value = rawArgs[index + 1];
  if (!value) {
    fail(`missing value for ${item}`);
  }
  return value;
}

function parseNonNegativeNumber(value, label) {
  const numberValue = Number(value);
  if (!Number.isFinite(numberValue) || numberValue < 0) {
    fail(`invalid ${label} value: ${value}`);
  }
  return numberValue;
}

function parseNonNegativeInteger(value, label) {
  const numberValue = parseNonNegativeNumber(value, label);
  if (!Number.isInteger(numberValue)) {
    fail(`invalid ${label} value: ${value}`);
  }
  return numberValue;
}

function printUsage() {
  console.log(`Usage: node scripts/cache-maintenance.mjs [options]

Options:
  --json                         Print machine-readable JSON.
  --cache-dir <path>             Inspect a specific rust-index-v2 cache directory.
  --prune                        Plan deletion of old or over-budget Sage Rust index databases.
  --max-age-days <days>          Age threshold for --prune. Defaults to 30.
  --max-total-bytes <bytes>      Total cache budget. Defaults to 2 GiB.
  --keep-latest <count>          Always keep this many newest database namespaces. Defaults to 2.
  --orphan-max-age-days <days>   Age threshold for orphan SQLite sidecars. Defaults to 1.
  --size-prune-min-age-days <d>  Minimum age before size-cap pruning. Defaults to 0.
  --yes                          Apply --prune. Without this, prune mode is dry-run.
`);
}

function formatBytes(value) {
  if (value < 1024) {
    return `${value} B`;
  }
  if (value < 1024 * 1024) {
    return `${(value / 1024).toFixed(1)} KiB`;
  }
  if (value < 1024 * 1024 * 1024) {
    return `${(value / (1024 * 1024)).toFixed(1)} MiB`;
  }
  return `${(value / (1024 * 1024 * 1024)).toFixed(2)} GiB`;
}

function messageOf(error) {
  return error instanceof Error ? error.message : String(error);
}

function fail(message) {
  console.error(message);
  process.exit(1);
}
