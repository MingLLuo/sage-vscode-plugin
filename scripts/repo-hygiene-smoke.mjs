#!/usr/bin/env node
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import {
  assertPinnedNodeVersion,
  assertPinnedNpmVersion,
  pinnedNodeVersion,
  pinnedNpmVersion,
} from "./package-toolchain.mjs";

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
  ".node-version",
  "package-lock.json",
  "rust-toolchain.toml",
  "scripts/build-packaged-rust-binary.mjs",
  "scripts/check-package-toolchain.mjs",
  "scripts/package-toolchain.mjs",
];

for (const relativePath of requiredFiles) {
  pushCheck(`required public maintenance file ${relativePath}`, exists(relativePath), relativePath);
}

const packageJson = readJson("package.json");
const scripts = packageJson.scripts ?? {};
pushCheck("Node package manager is pinned", packageJson.packageManager === "npm@11.17.0", packageJson.packageManager);
const expectedNpmVersion = pinnedNpmVersion(repositoryRoot);
let rejectsMismatchedNpm = false;
try {
  assertPinnedNpmVersion(repositoryRoot, "0.0.0");
} catch (error) {
  rejectsMismatchedNpm = String(error).includes(`requires npm ${expectedNpmVersion}`);
}
pushCheck("packaging rejects a mismatched npm runtime", rejectsMismatchedNpm, expectedNpmVersion);
let acceptsPinnedNpm = true;
try {
  assertPinnedNpmVersion(repositoryRoot, expectedNpmVersion);
} catch {
  acceptsPinnedNpm = false;
}
pushCheck("packaging accepts the exact pinned npm runtime", acceptsPinnedNpm, expectedNpmVersion);
const expectedNodeVersion = pinnedNodeVersion(repositoryRoot);
pushCheck("Node runtime is pinned", expectedNodeVersion === "22.23.1", expectedNodeVersion);
let rejectsMismatchedNode = false;
try {
  assertPinnedNodeVersion(repositoryRoot, "0.0.0");
} catch (error) {
  rejectsMismatchedNode = String(error).includes(`requires Node ${expectedNodeVersion}`);
}
pushCheck("packaging rejects a mismatched Node runtime", rejectsMismatchedNode, expectedNodeVersion);
let acceptsPinnedNode = true;
try {
  assertPinnedNodeVersion(repositoryRoot, expectedNodeVersion);
} catch {
  acceptsPinnedNode = false;
}
pushCheck("packaging accepts the exact pinned Node runtime", acceptsPinnedNode, expectedNodeVersion);
const rustToolchain = readText("rust-toolchain.toml");
pushCheck("Rust runtime is pinned", /channel\s*=\s*["']1\.92\.0["']/.test(rustToolchain), "rust-toolchain.toml");
pushCheck(
  "Rust lint and format components are pinned",
  rustToolchain.includes('components = ["clippy", "rustfmt"]'),
  "rust-toolchain.toml",
);
const packageLock = readJson("package-lock.json");
const remoteLockEntries = Object.values(packageLock.packages ?? {}).filter(
  (entry) => typeof entry?.resolved === "string" && entry.resolved.startsWith("https://"),
);
pushCheck(
  "remote Node dependencies have integrity hashes",
  remoteLockEntries.length > 0 && remoteLockEntries.every((entry) => typeof entry.integrity === "string"),
  { remoteEntries: remoteLockEntries.length, hashedEntries: remoteLockEntries.filter((entry) => entry.integrity).length },
);
pushCheck("test:repo-hygiene script is registered", scripts["test:repo-hygiene"] === "node scripts/repo-hygiene-smoke.mjs", scripts["test:repo-hygiene"]);
pushCheck("configure:workspace script is registered", scripts["configure:workspace"] === "node scripts/configure-workspace.mjs", scripts["configure:workspace"]);
pushCheck("doctor:mac script is registered", scripts["doctor:mac"] === "node scripts/macos-doctor.mjs", scripts["doctor:mac"]);
pushCheck("export:reference script is registered", scripts["export:reference"] === "node scripts/export-reference.mjs", scripts["export:reference"]);
pushCheck("test:reference-export script is registered", scripts["test:reference-export"] === "npm run build:debug-inspector && node scripts/reference-export-smoke.mjs", scripts["test:reference-export"]);
pushCheck("test:lsp-shutdown script is registered", scripts["test:lsp-shutdown"] === "npm run build:rust && node scripts/lsp-shutdown-smoke.mjs", scripts["test:lsp-shutdown"]);
pushCheck(
  "packaging toolchain check is registered",
  scripts["check:package-toolchain"] === "node scripts/check-package-toolchain.mjs",
  scripts["check:package-toolchain"],
);
pushCheck(
  "Rust release packaging uses the locked reproducible builder",
  includesScript("package:rust-binary", "npm run check:package-toolchain")
    && includesScript("package:rust-binary", "node scripts/build-packaged-rust-binary.mjs"),
  scripts["package:rust-binary"],
);
pushCheck(
  "VSIX packaging checks the pinned toolchain before building",
  scripts["package:vsix"]?.startsWith("npm run check:package-toolchain &&") === true,
  scripts["package:vsix"],
);
pushCheck(
  "test:generated-assets script checks syntax and icon drift",
  scripts["test:generated-assets"] === "node scripts/sync-syntax-assets.mjs --check && node scripts/generate-extension-icon.mjs --check",
  scripts["test:generated-assets"],
);
pushCheck("test:ci includes repo hygiene", includesScript("test:ci", "npm run test:repo-hygiene"), scripts["test:ci"]);
pushCheck("test:release includes repo hygiene", includesScript("test:release", "npm run test:repo-hygiene"), scripts["test:release"]);
pushCheck("test:ci includes generated asset drift check", includesScript("test:ci", "npm run test:generated-assets"), scripts["test:ci"]);
pushCheck("test:release includes generated asset drift check", includesScript("test:release", "npm run test:generated-assets"), scripts["test:release"]);
pushCheck("test:ci includes product readiness smoke", includesScript("test:ci", "npm run test:product-readiness"), scripts["test:ci"]);
pushCheck("test:release includes product readiness smoke", includesScript("test:release", "npm run test:product-readiness"), scripts["test:release"]);
pushCheck("test:ci includes offline reference export smoke", includesScript("test:ci", "npm run test:reference-export"), scripts["test:ci"]);
pushCheck("test:release includes offline reference export smoke", includesScript("test:release", "npm run test:reference-export"), scripts["test:release"]);
pushCheck("test:ci includes LSP shutdown smoke", includesScript("test:ci", "npm run test:lsp-shutdown"), scripts["test:ci"]);
pushCheck("test:release includes LSP shutdown smoke", includesScript("test:release", "npm run test:lsp-shutdown"), scripts["test:release"]);

for (const localOnly of ["test:lsp-latency", "test:real-file-smoke", "test:native-smoke", "test:extension-host"]) {
  pushCheck(`test:ci excludes local-only ${localOnly}`, !includesScript("test:ci", localOnly), scripts["test:ci"]);
}

const workflow = readText(".github/workflows/ci.yml");
pushCheck("GitHub workflow installs locked Node dependencies", /npm ci/.test(workflow), ".github/workflows/ci.yml");
pushCheck("GitHub workflow pins the declared npm version", /npm install --global npm@11\.17\.0/.test(workflow), ".github/workflows/ci.yml");
pushCheck("GitHub workflow uses the pinned Node runtime", /node-version-file:\s*["']?\.node-version["']?/.test(workflow), ".github/workflows/ci.yml");
pushCheck("GitHub workflow uses the pinned Rust runtime", /toolchain:\s*["']?1\.92\.0["']?/.test(workflow), ".github/workflows/ci.yml");
pushCheck("GitHub workflow enables setup-node npm cache", /cache:\s*["']?npm["']?/.test(workflow)
  && /cache-dependency-path:\s*package-lock\.json/.test(workflow), ".github/workflows/ci.yml");
pushCheck("GitHub workflow runs on macOS for the maintained release target", /runs-on:\s*macos-latest/.test(workflow), ".github/workflows/ci.yml");
pushCheck("GitHub workflow caches Rust dependencies", /Swatinem\/rust-cache@v2/.test(workflow)
  && /cache-on-failure:\s*true/.test(workflow), ".github/workflows/ci.yml");
pushCheck("GitHub workflow prefetches locked Rust dependencies", /cargo fetch --locked/.test(workflow), ".github/workflows/ci.yml");
pushCheck("GitHub workflow runs macOS CI gate", /npm run test:ci/.test(workflow), ".github/workflows/ci.yml");
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
for (const expected of ["configure:workspace", "doctor:mac", "export:reference", "test:reference-export", "test:repo-hygiene", "SECURITY.md", "SUPPORT.md", ".gitattributes", ".editorconfig"]) {
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
  "scripts/configure-workspace.mjs",
  "scripts/lsp-latency-smoke.mjs",
  "scripts/real-file-smoke.mjs",
  "scripts/macos-doctor.mjs",
  "scripts/export-reference.mjs",
  "scripts/reference-export-smoke.mjs",
  "scripts/reference-viewer/index.html",
  "scripts/reference-viewer/viewer.css",
  "scripts/reference-viewer/viewer.js",
  "scripts/reference-viewer/README.md",
  "scripts/dev-vscode.sh",
]) {
  const text = readText(relativePath);
  pushCheck(`${relativePath} avoids maintainer-private paths`, !containsPrivateHomePath(text), relativePath);
}

const referenceExporter = readText("scripts/export-reference.mjs");
const referenceViewerHtml = readText("scripts/reference-viewer/index.html");
const referenceViewerJs = readText("scripts/reference-viewer/viewer.js");
const referenceViewerCss = readText("scripts/reference-viewer/viewer.css");
pushCheck("reference exporter writes static viewer entrypoint", referenceExporter.includes("referenceViewerRoot")
  && referenceExporter.includes("fs.copyFile")
  && referenceExporter.includes("index.html")
  && referenceExporter.includes("viewer.js")
  && referenceExporter.includes("symbols.js"), "scripts/export-reference.mjs");
pushCheck("reference exporter strips local home paths", referenceExporter.includes("sanitizeText")
  && referenceExporter.includes("assertNoPrivatePaths"), "scripts/export-reference.mjs");
pushCheck("reference viewer is kept in maintainable static assets", referenceExporter.includes("reference-viewer")
  && referenceViewerHtml.includes("mobile-tabs")
  && referenceViewerJs.includes("restoreFromHash")
  && referenceViewerCss.includes("html[data-theme=\"dark\"]"), "scripts/reference-viewer");
pushCheck("reference viewer stores URL-shareable viewer state", referenceViewerJs.includes("restoreFromHash")
  && referenceViewerJs.includes("history.replaceState"), "scripts/reference-viewer/viewer.js");

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
