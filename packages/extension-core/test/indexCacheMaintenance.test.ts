import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import fsSync from "node:fs";
import os from "node:os";
import path from "node:path";

import { maintainIndexCache } from "../src/indexCacheMaintenance";

const DAY_MS = 24 * 60 * 60 * 1000;

test("maintainIndexCache prunes old, oversized, and orphaned SQLite cache files", async () => {
  const nowMs = Date.UTC(2026, 4, 26, 12, 0, 0);
  const tempRoot = await fs.mkdtemp(path.join(os.tmpdir(), "sage-extension-cache-maintenance-"));
  const oldDigest = "1111111111111111";
  const freshDigest = "2222222222222222";
  const oversizedDigest = "3333333333333333";
  const orphanDigest = "4444444444444444";
  const oldFiles = [
    `sage-index-${oldDigest}.sqlite`,
    `sage-index-${oldDigest}.sqlite-wal`,
    `sage-index-${oldDigest}.sqlite-shm`,
  ];
  const freshFile = `sage-index-${freshDigest}.sqlite`;
  const oversizedFile = `sage-index-${oversizedDigest}.sqlite`;
  const orphanSidecar = `sage-index-${orphanDigest}.sqlite-wal`;

  try {
    for (const name of oldFiles) {
      await writeFixture(tempRoot, name, "old");
      await touchDaysAgo(tempRoot, name, nowMs, 45);
    }
    await writeFixture(tempRoot, freshFile, "new");
    await touchDaysAgo(tempRoot, freshFile, nowMs, 1);
    await writeFixture(tempRoot, oversizedFile, "oversized-size-cache");
    await touchDaysAgo(tempRoot, oversizedFile, nowMs, 7);
    await writeFixture(tempRoot, orphanSidecar, "orphan");
    await touchDaysAgo(tempRoot, orphanSidecar, nowMs, 2);
    await writeFixture(tempRoot, "notes.txt", "ignore");

    const dryRun = await maintainIndexCache({
      cacheDir: tempRoot,
      dryRun: true,
      maxAgeDays: 30,
      maxTotalBytes: 12,
      keepLatestDatabases: 1,
      orphanMaxAgeDays: 1,
      sizePruneMinAgeDays: 1,
      nowMs,
    });
    assert.equal(dryRun.totals.databaseCount, 3);
    assert.equal(dryRun.totals.prunableDatabaseCount, 2);
    assert.equal(dryRun.totals.prunableOrphanSidecarCount, 1);
    assert.equal(dryRun.actions.length, 5);
    assert.ok(dryRun.actions.every((action) => action.action === "would_delete"));
    for (const name of [...oldFiles, freshFile, oversizedFile, orphanSidecar]) {
      assertExists(path.join(tempRoot, name), `dry run should keep ${name}`);
    }

    const applied = await maintainIndexCache({
      cacheDir: tempRoot,
      maxAgeDays: 30,
      maxTotalBytes: 12,
      keepLatestDatabases: 1,
      orphanMaxAgeDays: 1,
      sizePruneMinAgeDays: 1,
      nowMs,
    });
    assert.equal(applied.totals.deletedFileCount, 5);
    for (const name of [...oldFiles, oversizedFile, orphanSidecar]) {
      assertMissing(path.join(tempRoot, name), `applied prune should remove ${name}`);
    }
    assertExists(path.join(tempRoot, freshFile), "applied prune should preserve the newest cache");
    assertExists(path.join(tempRoot, "notes.txt"), "applied prune should ignore non-cache files");
  } finally {
    await fs.rm(tempRoot, { recursive: true, force: true });
  }
});

test("maintainIndexCache leaves missing cache directories alone", async () => {
  const report = await maintainIndexCache({
    cacheDir: path.join(os.tmpdir(), `sage-missing-cache-${Date.now()}`),
    maxTotalBytes: 1,
  });
  assert.equal(report.exists, false);
  assert.equal(report.totals.databaseCount, 0);
  assert.deepEqual(report.failures, []);
});

test("maintainIndexCache does not size-prune fresh cache databases", async () => {
  const nowMs = Date.UTC(2026, 4, 26, 12, 0, 0);
  const tempRoot = await fs.mkdtemp(path.join(os.tmpdir(), "sage-extension-cache-fresh-"));
  const digest = "5555555555555555";
  const fileName = `sage-index-${digest}.sqlite`;
  try {
    await writeFixture(tempRoot, fileName, "larger-than-budget");
    await touchDaysAgo(tempRoot, fileName, nowMs, 0);
    const report = await maintainIndexCache({
      cacheDir: tempRoot,
      maxAgeDays: 30,
      maxTotalBytes: 1,
      keepLatestDatabases: 0,
      sizePruneMinAgeDays: 1,
      nowMs,
    });
    assert.equal(report.totals.deletedFileCount, 0);
    assertExists(path.join(tempRoot, fileName), "fresh cache should survive automatic size pruning");
  } finally {
    await fs.rm(tempRoot, { recursive: true, force: true });
  }
});

async function writeFixture(root: string, name: string, content: string): Promise<void> {
  await fs.writeFile(path.join(root, name), content, "utf8");
}

async function touchDaysAgo(root: string, name: string, nowMs: number, days: number): Promise<void> {
  const date = new Date(nowMs - days * DAY_MS);
  await fs.utimes(path.join(root, name), date, date);
}

function assertExists(filePath: string, message: string): void {
  if (!fsSync.existsSync(filePath)) {
    throw new Error(message);
  }
}

function assertMissing(filePath: string, message: string): void {
  if (fsSync.existsSync(filePath)) {
    throw new Error(message);
  }
}
