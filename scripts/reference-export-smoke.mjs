#!/usr/bin/env node
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const repositoryRoot = path.resolve(__dirname, "..");
const tmpRoot = fs.mkdtempSync(path.join(os.tmpdir(), "sage-reference-export-"));
const workspaceRoot = path.join(tmpRoot, "workspace");
const outputRoot = path.join(workspaceRoot, ".sage-reference");
const sourceRoot = resolveSourceRoot();
const checks = [];

fs.mkdirSync(path.join(workspaceRoot, "src"), { recursive: true });
fs.writeFileSync(path.join(workspaceRoot, "src", "demo.py"), `"""Tiny public smoke fixture."""
from sage.all import GF, PolynomialRing, matrix, vector

def build_ring():
    field = GF(101)
    ring = PolynomialRing(field, ("x", "y"))
    return ring

def solve_demo():
    mat = matrix(GF(101), [[1, 2], [3, 4]])
    vec = vector(GF(101), [1, 0])
    return mat.rank(), mat.solve_right(vec), build_ring()
`, "utf8");
fs.writeFileSync(path.join(workspaceRoot, "src", "demo.sage"), `R.<x, y> = PolynomialRing(GF(101))
f = x^2 + y
`, "utf8");

const args = [
  path.join(repositoryRoot, "scripts", "export-reference.mjs"),
  "--workspace",
  workspaceRoot,
  "--out",
  outputRoot,
];
if (sourceRoot) {
  args.push("--source-root", sourceRoot);
}

const result = spawnSync(process.execPath, args, {
  cwd: repositoryRoot,
  encoding: "utf8",
  maxBuffer: 120 * 1024 * 1024,
});
pushCheck("export command exits successfully", result.status === 0, result.stderr || result.stdout);

for (const relativePath of [
  "index.html",
  "README.md",
  "assets/viewer.css",
  "assets/viewer.js",
  "data/manifest.js",
  "data/symbols.js",
]) {
  pushCheck(`generated ${relativePath}`, fs.existsSync(path.join(outputRoot, relativePath)), relativePath);
}

const sourceShardDir = path.join(outputRoot, "data", "sources");
const sourceShards = fs.existsSync(sourceShardDir)
  ? fs.readdirSync(sourceShardDir).filter((entry) => /^source-\d+\.js$/.test(entry))
  : [];
pushCheck("source shards are generated", sourceShards.length > 0, sourceShards);

const manifestText = readGenerated("data/manifest.js");
const symbolsText = readGenerated("data/symbols.js");
const viewerText = readGenerated("assets/viewer.js");
const cssText = readGenerated("assets/viewer.css");
const indexText = readGenerated("index.html");

pushCheck("manifest contains schema version", manifestText.includes("schemaVersion"), "data/manifest.js");
pushCheck("symbols data exposes search index", symbolsText.includes("searchIndex"), "data/symbols.js");
pushCheck("local function is searchable", symbolsText.includes("build_ring"), "build_ring");
pushCheck("Sage constructor candidate is searchable", symbolsText.includes("PolynomialRing"), "PolynomialRing");
pushCheck("viewer restores URL hash state", viewerText.includes("restoreFromHash"), "assets/viewer.js");
pushCheck("viewer supports keyboard search shortcut", viewerText.includes("event.key === \"/\""), "assets/viewer.js");
pushCheck("viewer supports reference navigation shortcuts", viewerText.includes("event.key === \"[\"") && viewerText.includes("event.key === \"]\""), "assets/viewer.js");
pushCheck("viewer renders detail documentation", viewerText.includes("symbol.doc") && viewerText.includes("full-doc"), "assets/viewer.js");
pushCheck("viewer initializes lazy source shard store", viewerText.includes("window.__SAGE_REFERENCE_SOURCES__ = window.__SAGE_REFERENCE_SOURCES__ || {}"), "assets/viewer.js");
pushCheck("viewer shows search result count", viewerText.includes("resultCount.textContent"), "assets/viewer.js");
pushCheck("viewer supports narrow-screen panel switching", viewerText.includes("setPanel") && indexText.includes("mobile-tabs"), "index.html / assets/viewer.js");
pushCheck("viewer shows source loading and error states", viewerText.includes("Loading source") && viewerText.includes("Failed to load source shard"), "assets/viewer.js");
pushCheck("viewer does not preload source shards in index.html", !indexText.includes("data/sources/source-"), "index.html");
pushCheck("viewer has light and dark themes", cssText.includes("html[data-theme=\"dark\"]") && cssText.includes("color-scheme"), "assets/viewer.css");
pushCheck("generated package avoids private paths", !containsPrivateHomePath(readAllGeneratedText(outputRoot)), outputRoot);

const failures = checks.filter((check) => !check.pass);
const report = {
  schema_version: 1,
  status: failures.length ? "failed" : "passed",
  output: outputRoot,
  source_root_used: sourceRoot !== null,
  checks,
};
console.log(JSON.stringify(report, null, 2));
if (failures.length > 0) {
  process.exitCode = 1;
}

function readGenerated(relativePath) {
  const filePath = path.join(outputRoot, relativePath);
  return fs.existsSync(filePath) ? fs.readFileSync(filePath, "utf8") : "";
}

function readAllGeneratedText(root) {
  const parts = [];
  const stack = [root];
  while (stack.length > 0) {
    const current = stack.pop();
    if (!fs.existsSync(current)) {
      continue;
    }
    const stat = fs.statSync(current);
    if (stat.isDirectory()) {
      for (const entry of fs.readdirSync(current)) {
        stack.push(path.join(current, entry));
      }
      continue;
    }
    if (!/\.(png|jpg|jpeg|gif|ico)$/i.test(current)) {
      parts.push(fs.readFileSync(current, "utf8"));
    }
  }
  return parts.join("\n");
}

function containsPrivateHomePath(text) {
  return /\/Users\/[^/\s]+\/|\/home\/[^/\s]+\/|[A-Za-z]:\\Users\\[^\\\s]+\\/u.test(text);
}

function pushCheck(name, pass, actual) {
  checks.push({
    name,
    pass: Boolean(pass),
    actual,
  });
}

function resolveSourceRoot() {
  const configured = (process.env.SAGE_SOURCE_ROOT ?? "")
    .split(path.delimiter)
    .map((entry) => entry.trim())
    .filter(Boolean);
  const candidates = [
    ...configured,
    path.resolve(repositoryRoot, "sage", "src"),
    path.resolve(repositoryRoot, "..", "sage", "src"),
  ];
  for (const candidate of candidates) {
    const resolved = path.resolve(candidate);
    if (fs.existsSync(path.join(resolved, "sage"))) {
      return resolved;
    }
  }
  return null;
}
