import { cpSync, existsSync, mkdirSync, readdirSync, readFileSync } from "node:fs";
import path from "node:path";

const root = path.resolve(import.meta.dirname, "..");
const sourceDir = path.join(root, "packages", "syntax-pack");
const targetDir = path.join(root, "packages", "extension-core", "resources", "generated", "syntax");
const checkOnly = process.argv.includes("--check");

const files = [
  "language-configuration.json",
  path.join("syntaxes", "sagemath.tmLanguage.json"),
  path.join("snippets", "sagemath.json")
];

function ensureTargetDir() {
  mkdirSync(path.join(targetDir, "syntaxes"), { recursive: true });
  mkdirSync(path.join(targetDir, "snippets"), { recursive: true });
}

function contentMatches(relativePath) {
  const sourcePath = path.join(sourceDir, relativePath);
  const targetPath = path.join(targetDir, relativePath);

  return existsSync(targetPath) && readFileSync(sourcePath, "utf8") === readFileSync(targetPath, "utf8");
}

ensureTargetDir();

if (checkOnly) {
  const mismatched = files.filter((relativePath) => !contentMatches(relativePath));
  if (mismatched.length > 0) {
    console.error(`Syntax assets are out of sync: ${mismatched.join(", ")}`);
    process.exit(1);
  }
  process.exit(0);
}

for (const relativePath of files) {
  const fromPath = path.join(sourceDir, relativePath);
  const toPath = path.join(targetDir, relativePath);
  mkdirSync(path.dirname(toPath), { recursive: true });
  cpSync(fromPath, toPath);
}

const generatedEntries = readdirSync(targetDir);
console.log(`Synced syntax assets into ${targetDir}: ${generatedEntries.join(", ")}`);

