#!/usr/bin/env node
// Compares two `sage-index-snapshot` outputs and reports which files changed in which analysis
// dimension. Exit code 1 when any difference exists, so it can gate parser/dependency upgrades.
//
// Usage: node scripts/index-snapshot-diff.mjs before.jsonl after.jsonl [--limit N]
import fs from "node:fs";

const [beforePath, afterPath, ...rest] = process.argv.slice(2);
if (!beforePath || !afterPath) {
  console.error("usage: index-snapshot-diff.mjs <before.jsonl> <after.jsonl> [--limit N]");
  process.exit(2);
}
const limitIndex = rest.indexOf("--limit");
const limit = limitIndex >= 0 ? Number(rest[limitIndex + 1]) : 40;

function load(file) {
  const entries = new Map();
  for (const line of fs.readFileSync(file, "utf8").split("\n")) {
    if (!line.trim()) continue;
    const entry = JSON.parse(line);
    entries.set(entry.path, entry);
  }
  return entries;
}

const before = load(beforePath);
const after = load(afterPath);
const dimensions = [
  ["symbols", "symbol_digest"],
  ["semantic_spans", "semantic_digest"],
  ["diagnostics", "diagnostic_digest"],
];
const report = {
  files_before: before.size,
  files_after: after.size,
  added: [...after.keys()].filter((file) => !before.has(file)),
  removed: [...before.keys()].filter((file) => !after.has(file)),
  changed: { symbols: [], semantic_spans: [], diagnostics: [] },
  totals: {},
};
for (const [count] of dimensions) {
  const sum = (entries) => [...entries.values()].reduce((total, entry) => total + entry[count], 0);
  report.totals[count] = { before: sum(before), after: sum(after) };
}
for (const [file, left] of before) {
  const right = after.get(file);
  if (!right) continue;
  for (const [count, digest] of dimensions) {
    if (left[digest] !== right[digest]) {
      report.changed[count].push({ path: file, before: left[count], after: right[count] });
    }
  }
}
const differences = report.added.length + report.removed.length
  + dimensions.reduce((total, [count]) => total + report.changed[count].length, 0);
const summary = {
  status: differences === 0 ? "identical" : "different",
  files_before: report.files_before,
  files_after: report.files_after,
  totals: report.totals,
  added: report.added.length,
  removed: report.removed.length,
  changed: Object.fromEntries(dimensions.map(([count]) => [count, report.changed[count].length])),
  examples: {
    added: report.added.slice(0, limit),
    removed: report.removed.slice(0, limit),
    ...Object.fromEntries(dimensions.map(([count]) => [count, report.changed[count].slice(0, limit)])),
  },
};
console.log(JSON.stringify(summary, null, 2));
process.exit(differences === 0 ? 0 : 1);
