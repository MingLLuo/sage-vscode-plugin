import fs from "node:fs/promises";
import path from "node:path";

const DATABASE_PATTERN = /^sage-index-([a-f0-9]{16})\.sqlite$/;
const SIDECAR_PATTERN = /^sage-index-([a-f0-9]{16})\.sqlite-(wal|shm)$/;
const DAY_MS = 24 * 60 * 60 * 1000;

export const DEFAULT_INDEX_CACHE_MAX_AGE_DAYS = 30;
export const DEFAULT_INDEX_CACHE_MAX_TOTAL_BYTES = 2 * 1024 * 1024 * 1024;
export const DEFAULT_INDEX_CACHE_KEEP_LATEST_DATABASES = 2;
export const DEFAULT_INDEX_CACHE_ORPHAN_MAX_AGE_DAYS = 1;
export const DEFAULT_INDEX_CACHE_SIZE_PRUNE_MIN_AGE_DAYS = 0;

export interface IndexCacheMaintenanceOptions {
  cacheDir: string;
  dryRun?: boolean;
  maxAgeDays?: number;
  maxTotalBytes?: number;
  keepLatestDatabases?: number;
  orphanMaxAgeDays?: number;
  sizePruneMinAgeDays?: number;
  nowMs?: number;
}

export interface IndexCacheFileRecord {
  name: string;
  path: string;
  sizeBytes: number;
  mtimeMs: number;
  ageDays: number;
}

export interface IndexCacheSidecarRecord extends IndexCacheFileRecord {
  kind: "wal" | "shm";
  prunable: boolean;
  pruneReasons: string[];
}

export interface IndexCacheDatabaseRecord extends IndexCacheFileRecord {
  digest: string;
  sidecars: IndexCacheSidecarRecord[];
  totalBytes: number;
  protected: boolean;
  prunable: boolean;
  pruneReasons: string[];
}

export interface IndexCacheMaintenanceAction {
  action: "would_delete" | "delete";
  path: string;
  sizeBytes: number;
  reason: string;
  error?: string;
}

export interface IndexCacheMaintenanceReport {
  cacheDir: string;
  exists: boolean;
  dryRun: boolean;
  maxAgeDays: number;
  maxTotalBytes: number;
  keepLatestDatabases: number;
  orphanMaxAgeDays: number;
  sizePruneMinAgeDays: number;
  entries: IndexCacheDatabaseRecord[];
  orphanSidecars: IndexCacheSidecarRecord[];
  actions: IndexCacheMaintenanceAction[];
  totals: {
    databaseCount: number;
    sidecarCount: number;
    orphanSidecarCount: number;
    totalBytes: number;
    prunableDatabaseCount: number;
    prunableOrphanSidecarCount: number;
    prunableBytes: number;
    deletedFileCount: number;
    deletedBytes: number;
  };
  failures: string[];
}

export async function maintainIndexCache(
  options: IndexCacheMaintenanceOptions,
): Promise<IndexCacheMaintenanceReport> {
  const resolved = resolveOptions(options);
  const report = emptyReport(resolved);

  let names: string[];
  try {
    names = await fs.readdir(resolved.cacheDir);
    report.exists = true;
  } catch (error) {
    if (isMissingPathError(error)) {
      return report;
    }
    report.failures.push(`cannot read cache dir ${resolved.cacheDir}: ${messageOf(error)}`);
    return report;
  }

  const sidecarsByDigest = new Map<string, IndexCacheSidecarRecord[]>();
  for (const name of names) {
    const match = SIDECAR_PATTERN.exec(name);
    if (!match) {
      continue;
    }
    const record = await fileRecord(resolved.cacheDir, name, resolved.nowMs, report.failures);
    if (!record) {
      continue;
    }
    const digest = match[1];
    const sidecars = sidecarsByDigest.get(digest) ?? [];
    sidecars.push({
      ...record,
      kind: match[2] as "wal" | "shm",
      prunable: false,
      pruneReasons: [],
    });
    sidecarsByDigest.set(digest, sidecars);
  }

  for (const name of names) {
    const match = DATABASE_PATTERN.exec(name);
    if (!match) {
      continue;
    }
    const database = await fileRecord(resolved.cacheDir, name, resolved.nowMs, report.failures);
    if (!database) {
      continue;
    }
    const digest = match[1];
    const sidecars = (sidecarsByDigest.get(digest) ?? []).sort((a, b) => a.name.localeCompare(b.name));
    sidecarsByDigest.delete(digest);
    const totalBytes = database.sizeBytes + sidecars.reduce((sum, item) => sum + item.sizeBytes, 0);
    report.entries.push({
      ...database,
      digest,
      sidecars,
      totalBytes,
      protected: false,
      prunable: false,
      pruneReasons: [],
    });
    report.totals.databaseCount += 1;
    report.totals.sidecarCount += sidecars.length;
    report.totals.totalBytes += totalBytes;
  }

  report.entries.sort((a, b) => b.mtimeMs - a.mtimeMs);
  for (const sidecars of sidecarsByDigest.values()) {
    for (const sidecar of sidecars) {
      report.orphanSidecars.push(sidecar);
      report.totals.orphanSidecarCount += 1;
      report.totals.totalBytes += sidecar.sizeBytes;
    }
  }
  report.orphanSidecars.sort((a, b) => b.mtimeMs - a.mtimeMs);

  markPrunableEntries(report);
  await applyPruneActions(report);
  return report;
}

function resolveOptions(options: IndexCacheMaintenanceOptions): Required<IndexCacheMaintenanceOptions> {
  return {
    cacheDir: path.resolve(options.cacheDir),
    dryRun: options.dryRun ?? false,
    maxAgeDays: options.maxAgeDays ?? DEFAULT_INDEX_CACHE_MAX_AGE_DAYS,
    maxTotalBytes: options.maxTotalBytes ?? DEFAULT_INDEX_CACHE_MAX_TOTAL_BYTES,
    keepLatestDatabases: options.keepLatestDatabases ?? DEFAULT_INDEX_CACHE_KEEP_LATEST_DATABASES,
    orphanMaxAgeDays: options.orphanMaxAgeDays ?? DEFAULT_INDEX_CACHE_ORPHAN_MAX_AGE_DAYS,
    sizePruneMinAgeDays: options.sizePruneMinAgeDays ?? DEFAULT_INDEX_CACHE_SIZE_PRUNE_MIN_AGE_DAYS,
    nowMs: options.nowMs ?? Date.now(),
  };
}

function emptyReport(options: Required<IndexCacheMaintenanceOptions>): IndexCacheMaintenanceReport {
  return {
    cacheDir: options.cacheDir,
    exists: false,
    dryRun: options.dryRun,
    maxAgeDays: options.maxAgeDays,
    maxTotalBytes: options.maxTotalBytes,
    keepLatestDatabases: options.keepLatestDatabases,
    orphanMaxAgeDays: options.orphanMaxAgeDays,
    sizePruneMinAgeDays: options.sizePruneMinAgeDays,
    entries: [],
    orphanSidecars: [],
    actions: [],
    totals: {
      databaseCount: 0,
      sidecarCount: 0,
      orphanSidecarCount: 0,
      totalBytes: 0,
      prunableDatabaseCount: 0,
      prunableOrphanSidecarCount: 0,
      prunableBytes: 0,
      deletedFileCount: 0,
      deletedBytes: 0,
    },
    failures: [],
  };
}

function markPrunableEntries(report: IndexCacheMaintenanceReport): void {
  const protectedDigests = new Set(
    report.entries
      .slice(0, report.keepLatestDatabases)
      .map((entry) => entry.digest),
  );
  for (const entry of report.entries) {
    entry.protected = protectedDigests.has(entry.digest);
  }

  let projectedTotalBytes = report.totals.totalBytes;
  for (const entry of [...report.entries].reverse()) {
    if (entry.protected || entry.ageDays < report.maxAgeDays) {
      continue;
    }
    markDatabasePrunable(entry, "age");
    projectedTotalBytes -= entry.totalBytes;
  }

  if (projectedTotalBytes > report.maxTotalBytes) {
    for (const entry of [...report.entries].reverse()) {
      if (
        projectedTotalBytes <= report.maxTotalBytes
        || entry.protected
        || entry.prunable
        || entry.ageDays < report.sizePruneMinAgeDays
      ) {
        continue;
      }
      markDatabasePrunable(entry, "size-cap");
      projectedTotalBytes -= entry.totalBytes;
    }
  }

  for (const sidecar of report.orphanSidecars) {
    if (sidecar.ageDays >= report.orphanMaxAgeDays) {
      markSidecarPrunable(sidecar, "orphan");
      projectedTotalBytes -= sidecar.sizeBytes;
    }
  }

  if (projectedTotalBytes > report.maxTotalBytes) {
    for (const sidecar of [...report.orphanSidecars].reverse()) {
      if (
        projectedTotalBytes <= report.maxTotalBytes
        || sidecar.prunable
        || sidecar.ageDays < report.sizePruneMinAgeDays
      ) {
        continue;
      }
      markSidecarPrunable(sidecar, "size-cap");
      projectedTotalBytes -= sidecar.sizeBytes;
    }
  }

  for (const entry of report.entries) {
    if (entry.prunable) {
      report.totals.prunableDatabaseCount += 1;
      report.totals.prunableBytes += entry.totalBytes;
    }
  }
  for (const sidecar of report.orphanSidecars) {
    if (sidecar.prunable) {
      report.totals.prunableOrphanSidecarCount += 1;
      report.totals.prunableBytes += sidecar.sizeBytes;
    }
  }
}

function markDatabasePrunable(entry: IndexCacheDatabaseRecord, reason: string): void {
  entry.prunable = true;
  if (!entry.pruneReasons.includes(reason)) {
    entry.pruneReasons.push(reason);
  }
  for (const sidecar of entry.sidecars) {
    markSidecarPrunable(sidecar, reason);
  }
}

function markSidecarPrunable(sidecar: IndexCacheSidecarRecord, reason: string): void {
  sidecar.prunable = true;
  if (!sidecar.pruneReasons.includes(reason)) {
    sidecar.pruneReasons.push(reason);
  }
}

async function applyPruneActions(report: IndexCacheMaintenanceReport): Promise<void> {
  for (const entry of report.entries) {
    if (!entry.prunable) {
      continue;
    }
    const reason = entry.pruneReasons.join(",") || "prune";
    await recordDeleteAction(report, entry, reason);
    for (const sidecar of entry.sidecars) {
      await recordDeleteAction(report, sidecar, sidecar.pruneReasons.join(",") || reason);
    }
  }
  for (const sidecar of report.orphanSidecars) {
    if (!sidecar.prunable) {
      continue;
    }
    await recordDeleteAction(report, sidecar, sidecar.pruneReasons.join(",") || "orphan");
  }
}

async function recordDeleteAction(
  report: IndexCacheMaintenanceReport,
  item: IndexCacheFileRecord,
  reason: string,
): Promise<void> {
  const action: IndexCacheMaintenanceAction = {
    action: report.dryRun ? "would_delete" : "delete",
    path: item.path,
    sizeBytes: item.sizeBytes,
    reason,
  };
  if (!report.dryRun) {
    try {
      await fs.unlink(item.path);
      report.totals.deletedFileCount += 1;
      report.totals.deletedBytes += item.sizeBytes;
    } catch (error) {
      action.error = messageOf(error);
      report.failures.push(`cannot delete ${item.path}: ${action.error}`);
    }
  }
  report.actions.push(action);
}

async function fileRecord(
  root: string,
  name: string,
  nowMs: number,
  failures: string[],
): Promise<IndexCacheFileRecord | undefined> {
  const absolutePath = path.join(root, name);
  let stats;
  try {
    stats = await fs.lstat(absolutePath);
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
    sizeBytes: stats.size,
    mtimeMs: Math.trunc(stats.mtimeMs),
    ageDays: Math.max(0, Math.floor((nowMs - stats.mtimeMs) / DAY_MS)),
  };
}

function isMissingPathError(error: unknown): boolean {
  return typeof error === "object" && error !== null && "code" in error && error.code === "ENOENT";
}

function messageOf(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
