import { cpSync, existsSync, mkdirSync, readdirSync, readFileSync, rmSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(scriptDir, "..");
const sourceDir = path.join(root, "packages", "syntax-pack");
const targetDir = path.join(root, "packages", "extension-core", "resources", "generated", "syntax");
const checkOnly = process.argv.includes("--check");

const files = [
  "language-configuration.json",
  path.join("syntaxes", "sagemath.tmLanguage.json"),
  path.join("snippets", "sagemath.json")
];
const expectedFiles = new Set(files.map((relativePath) => slash(relativePath)));

function ensureTargetDir() {
  mkdirSync(path.join(targetDir, "syntaxes"), { recursive: true });
  mkdirSync(path.join(targetDir, "snippets"), { recursive: true });
}

function contentMatches(relativePath) {
  const sourcePath = path.join(sourceDir, relativePath);
  const targetPath = path.join(targetDir, relativePath);

  return existsSync(targetPath) && readFileSync(sourcePath, "utf8") === readFileSync(targetPath, "utf8");
}

if (checkOnly) {
  const mismatched = files.filter((relativePath) => !contentMatches(relativePath));
  const staleFiles = listFiles(targetDir).filter((relativePath) => !expectedFiles.has(relativePath));
  if (mismatched.length > 0 || staleFiles.length > 0) {
    console.error(JSON.stringify({
      status: "failed",
      reason: "generated syntax assets are out of sync",
      mismatched: mismatched.map((relativePath) => slash(relativePath)),
      staleFiles,
    }, null, 2));
    process.exit(1);
  }
  console.log(JSON.stringify({
    status: "passed",
    checkedFiles: [...expectedFiles].sort(),
  }, null, 2));
  process.exit(0);
}

rmSync(targetDir, { recursive: true, force: true });
ensureTargetDir();
for (const relativePath of files) {
  const fromPath = path.join(sourceDir, relativePath);
  const toPath = path.join(targetDir, relativePath);
  mkdirSync(path.dirname(toPath), { recursive: true });
  cpSync(fromPath, toPath);
}

const generatedEntries = listFiles(targetDir);
console.log(`Synced syntax assets into ${targetDir}: ${generatedEntries.join(", ")}`);

function listFiles(rootDir, prefix = "") {
  if (!existsSync(rootDir)) {
    return [];
  }
  return readdirSync(path.join(rootDir, prefix), { withFileTypes: true })
    .flatMap((entry) => {
      const relativePath = slash(path.join(prefix, entry.name));
      if (entry.isDirectory()) {
        return listFiles(rootDir, relativePath);
      }
      return entry.isFile() ? [relativePath] : [];
    })
    .sort();
}

function slash(value) {
  return value.split(path.sep).join("/");
}
