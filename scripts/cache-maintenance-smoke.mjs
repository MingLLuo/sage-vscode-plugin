#!/usr/bin/env node
import { execFile } from "node:child_process";
import fs from "node:fs/promises";
import fsSync from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { promisify } from "node:util";

const execFileAsync = promisify(execFile);
const __dirname = path.dirname(fileURLToPath(import.meta.url));
const repositoryRoot = path.resolve(__dirname, "..");
const nodeCommand = process.execPath;
const scriptPath = path.join(repositoryRoot, "scripts", "cache-maintenance.mjs");
const tempRoot = await fs.mkdtemp(path.join(os.tmpdir(), "sage-cache-maintenance-smoke-"));
const oldDigest = "1111111111111111";
const newDigest = "2222222222222222";
const sizeDigest = "3333333333333333";
const orphanDigest = "4444444444444444";
const oldFiles = [
  `sage-index-${oldDigest}.sqlite`,
  `sage-index-${oldDigest}.sqlite-wal`,
  `sage-index-${oldDigest}.sqlite-shm`,
];
const newFile = `sage-index-${newDigest}.sqlite`;
const sizeFile = `sage-index-${sizeDigest}.sqlite`;
const orphanSidecar = `sage-index-${orphanDigest}.sqlite-wal`;
const ignoredFile = "notes.txt";

try {
  for (const name of oldFiles) {
    await writeFixture(name, "old");
    await touchDaysAgo(path.join(tempRoot, name), 45);
  }
  await writeFixture(newFile, "new");
  await touchDaysAgo(path.join(tempRoot, newFile), 1);
  await writeFixture(sizeFile, "oversized-size-cache");
  await touchDaysAgo(path.join(tempRoot, sizeFile), 7);
  await writeFixture(orphanSidecar, "orphan");
  await touchDaysAgo(path.join(tempRoot, orphanSidecar), 2);
  await writeFixture(ignoredFile, "keep");

  const inventory = await runMaintenance(["--cache-dir", tempRoot, "--json"]);
  assertEqual(inventory.totals.database_count, 3, "inventory should count only database files");
  assertEqual(inventory.totals.sidecar_count, 2, "inventory should associate SQLite sidecars");
  assertEqual(inventory.totals.orphan_sidecar_count, 1, "inventory should track orphan SQLite sidecars");
  assertEqual(inventory.totals.prunable_database_count, 1, "inventory should mark only old database prunable by default");

  const dryRun = await runMaintenance([
    "--cache-dir",
    tempRoot,
    "--prune",
    "--max-age-days",
    "30",
    "--max-total-bytes",
    "12",
    "--keep-latest",
    "1",
    "--size-prune-min-age-days",
    "1",
    "--json",
  ]);
  assertEqual(dryRun.dry_run, true, "prune without --yes must be dry-run");
  assertEqual(dryRun.actions.length, 5, "dry-run should plan old, over-budget, and orphan deletion");
  for (const name of oldFiles) {
    assertExists(path.join(tempRoot, name), `dry-run should keep ${name}`);
  }
  assertExists(path.join(tempRoot, sizeFile), "dry-run should keep over-budget cache database");
  assertExists(path.join(tempRoot, orphanSidecar), "dry-run should keep orphan sidecar");

  const applied = await runMaintenance([
    "--cache-dir",
    tempRoot,
    "--prune",
    "--max-age-days",
    "30",
    "--max-total-bytes",
    "12",
    "--keep-latest",
    "1",
    "--size-prune-min-age-days",
    "1",
    "--yes",
    "--json",
  ]);
  assertEqual(applied.dry_run, false, "prune with --yes should apply");
  assertEqual(applied.totals.deleted_file_count, 5, "applied prune should delete old, over-budget, and orphan files");
  for (const name of oldFiles) {
    assertMissing(path.join(tempRoot, name), `applied prune should remove ${name}`);
  }
  assertMissing(path.join(tempRoot, sizeFile), "applied prune should remove over-budget cache database");
  assertMissing(path.join(tempRoot, orphanSidecar), "applied prune should remove orphan sidecar");
  assertExists(path.join(tempRoot, newFile), "applied prune should keep newer cache database");
  assertExists(path.join(tempRoot, ignoredFile), "applied prune should ignore non-cache files");

  console.log(JSON.stringify({
    schema_version: 1,
    status: "passed",
    cache_dir: tempRoot,
    checked: ["inventory", "dry-run prune", "applied prune"],
  }, null, 2));
} finally {
  await fs.rm(tempRoot, { recursive: true, force: true });
}

async function writeFixture(name, content) {
  await fs.writeFile(path.join(tempRoot, name), content, "utf8");
}

async function touchDaysAgo(filePath, days) {
  const date = new Date(Date.now() - days * 24 * 60 * 60 * 1000);
  await fs.utimes(filePath, date, date);
}

async function runMaintenance(args) {
  const result = await execFileAsync(nodeCommand, [scriptPath, ...args], {
    cwd: repositoryRoot,
    maxBuffer: 1024 * 1024,
  });
  return JSON.parse(result.stdout);
}

function assertEqual(actual, expected, message) {
  if (actual !== expected) {
    throw new Error(`${message}: expected ${expected}, got ${actual}`);
  }
}

function assertExists(filePath, message) {
  if (!fsSync.existsSync(filePath)) {
    throw new Error(message);
  }
}

function assertMissing(filePath, message) {
  if (fsSync.existsSync(filePath)) {
    throw new Error(message);
  }
}
