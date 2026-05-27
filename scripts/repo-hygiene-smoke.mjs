#!/usr/bin/env node
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const repositoryRoot = path.resolve(__dirname, "..");
const checks = [];

const requiredFiles = [
  ".editorconfig",
  ".gitattributes",
  "CONTRIBUTING.md",
  "SECURITY.md",
  "SUPPORT.md",
  "docs/process/ci-and-release-gates.md",
  ".github/workflows/ci.yml",
  ".github/pull_request_template.md",
  ".github/ISSUE_TEMPLATE/config.yml",
  ".github/ISSUE_TEMPLATE/bug_report.yml",
  ".github/ISSUE_TEMPLATE/performance_regression.yml",
  ".github/ISSUE_TEMPLATE/feature_request.yml",
];

for (const relativePath of requiredFiles) {
  pushCheck(`required public maintenance file ${relativePath}`, exists(relativePath), relativePath);
}

const packageJson = readJson("package.json");
const scripts = packageJson.scripts ?? {};
pushCheck("test:repo-hygiene script is registered", scripts["test:repo-hygiene"] === "node scripts/repo-hygiene-smoke.mjs", scripts["test:repo-hygiene"]);
pushCheck(
  "test:generated-assets script checks syntax and icon drift",
  scripts["test:generated-assets"] === "node scripts/sync-syntax-assets.mjs --check && node scripts/generate-extension-icon.mjs --check",
  scripts["test:generated-assets"],
);
pushCheck("test:ci includes repo hygiene", includesScript("test:ci", "npm run test:repo-hygiene"), scripts["test:ci"]);
pushCheck("test:release includes repo hygiene", includesScript("test:release", "npm run test:repo-hygiene"), scripts["test:release"]);
pushCheck("test:ci includes generated asset drift check", includesScript("test:ci", "npm run test:generated-assets"), scripts["test:ci"]);
pushCheck("test:release includes generated asset drift check", includesScript("test:release", "npm run test:generated-assets"), scripts["test:release"]);

for (const localOnly of ["test:lsp-latency", "test:real-file-smoke", "test:native-smoke", "test:extension-host"]) {
  pushCheck(`test:ci excludes local-only ${localOnly}`, !includesScript("test:ci", localOnly), scripts["test:ci"]);
}

const workflow = readText(".github/workflows/ci.yml");
pushCheck("GitHub workflow installs Node dependencies without a tracked lockfile", /npm install/.test(workflow) && !/package-lock\.json/.test(workflow), ".github/workflows/ci.yml");
pushCheck("GitHub workflow runs portable CI gate", /npm run test:ci/.test(workflow), ".github/workflows/ci.yml");
pushCheck("GitHub workflow avoids local release gate", !/npm run test:release/.test(workflow), ".github/workflows/ci.yml");

const editorConfig = readText(".editorconfig");
pushCheck(".editorconfig is rooted", /^root\s*=\s*true/m.test(editorConfig), ".editorconfig");
pushCheck(".editorconfig enforces LF line endings", /end_of_line\s*=\s*lf/.test(editorConfig), ".editorconfig");
pushCheck(".editorconfig enforces final newlines", /insert_final_newline\s*=\s*true/.test(editorConfig), ".editorconfig");
pushCheck(".editorconfig declares Rust/Python/Sage indentation", /\[\*\.\{py,pyx,pxd,pxi,spyx,rs,sage\}\]/.test(editorConfig), ".editorconfig");

const gitAttributes = readText(".gitattributes");
for (const expected of [
  "* text=auto eol=lf",
  "*.rs text eol=lf",
  "*.ts text eol=lf",
  "*.py text eol=lf",
  "*.sage text eol=lf",
  "*.pyx text eol=lf",
  "*.pxd text eol=lf",
  "*.pxi text eol=lf",
  "*.spyx text eol=lf",
  "*.png binary",
  "*.vsix binary",
  "packages/extension-core/resources/bin/**/sage-ls binary",
]) {
  pushCheck(`.gitattributes contains ${expected}`, gitAttributes.includes(expected), expected);
}

const pullRequestTemplate = readText(".github/pull_request_template.md");
for (const expected of [
  "npm run test:ci",
  "npm run test:release",
  "npm run test:extension-host",
  "Future Sage compatibility",
]) {
  pushCheck(`PR template mentions ${expected}`, pullRequestTemplate.includes(expected), expected);
}

const bugTemplate = readText(".github/ISSUE_TEMPLATE/bug_report.yml");
const performanceTemplate = readText(".github/ISSUE_TEMPLATE/performance_regression.yml");
const featureTemplate = readText(".github/ISSUE_TEMPLATE/feature_request.yml");
pushCheck("bug template asks for support bundle", bugTemplate.includes("Sage: Copy Support Bundle"), "bug_report.yml");
pushCheck("bug template asks for affected editor surface", bugTemplate.includes("Affected surface"), "bug_report.yml");
pushCheck("performance template asks for index/docs status", performanceTemplate.includes("Sage: Show Index Status"), "performance_regression.yml");
pushCheck("performance template mentions release latency gates", performanceTemplate.includes("npm run test:lsp-latency"), "performance_regression.yml");
pushCheck("feature template asks for modern plugin comparison", featureTemplate.includes("Pyright") && featureTemplate.includes("rust-analyzer"), "feature_request.yml");

const security = readText("SECURITY.md");
pushCheck("security policy discourages public exploit details", /Do not post exploit details/.test(security), "SECURITY.md");
pushCheck("security policy calls out local process risk", /local processes/.test(security), "SECURITY.md");

const support = readText("SUPPORT.md");
pushCheck("support guide references UX self check", support.includes("Sage: Run UX Self Check"), "SUPPORT.md");
pushCheck("support guide references docs and index status", support.includes("Sage: Show Index Status") && support.includes("Sage: Show Docs Status"), "SUPPORT.md");

const docs = [
  readText("README.md"),
  readText("CONTRIBUTING.md"),
  readText("docs/process/ci-and-release-gates.md"),
  readText("docs/developer-guide.md"),
].join("\n");
for (const expected of ["test:repo-hygiene", "SECURITY.md", "SUPPORT.md", ".gitattributes", ".editorconfig"]) {
  pushCheck(`public docs mention ${expected}`, docs.includes(expected), expected);
}

for (const relativePath of requiredFiles) {
  const text = readText(relativePath);
  pushCheck(`${relativePath} avoids maintainer-private paths`, !containsPrivateHomePath(text), relativePath);
}

for (const relativePath of [
  ".vscode/launch.json",
  "README.md",
  "CONTRIBUTING.md",
  "SUPPORT.md",
  "SECURITY.md",
  "docs/developer-guide.md",
  "docs/install-and-configure.md",
  "docs/plugin-completeness.md",
  "docs/process/ci-and-release-gates.md",
  "docs/design/rust-lsp-v2.md",
  "packages/extension-core/README.md",
  "packages/extension-core/CHANGELOG.md",
  "scripts/lsp-latency-smoke.mjs",
  "scripts/real-file-smoke.mjs",
  "scripts/dev-vscode.sh",
]) {
  const text = readText(relativePath);
  pushCheck(`${relativePath} avoids maintainer-private paths`, !containsPrivateHomePath(text), relativePath);
}

const failures = checks.filter((check) => !check.pass);
console.log(JSON.stringify({
  schema_version: 1,
  status: failures.length ? "failed" : "passed",
  checks,
}, null, 2));
if (failures.length > 0) {
  process.exitCode = 1;
}

function containsPrivateHomePath(text) {
  return /\/Users\/(?!example\/|\.\.\.\/|<[^>]+>\/)[^/\s]+\/|\/home\/(?!example\/|\.\.\.\/|<[^>]+>\/)[^/\s]+\/|[A-Za-z]:\\Users\\(?!example\\|<[^>]+>\\)[^\\\s]+\\/u.test(text);
}

function includesScript(name, snippet) {
  return String(scripts[name] ?? "").includes(snippet);
}

function exists(relativePath) {
  return fs.existsSync(path.join(repositoryRoot, relativePath));
}

function readText(relativePath) {
  return fs.readFileSync(path.join(repositoryRoot, relativePath), "utf8");
}

function readJson(relativePath) {
  return JSON.parse(readText(relativePath));
}

function pushCheck(name, pass, actual) {
  checks.push({
    name,
    pass: Boolean(pass),
    actual,
  });
}
