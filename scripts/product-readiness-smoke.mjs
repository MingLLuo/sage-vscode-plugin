#!/usr/bin/env node
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const repositoryRoot = path.resolve(__dirname, "..");
const packageRoot = path.join(repositoryRoot, "packages", "extension-core");
const args = parseArgs(process.argv.slice(2));

const rootPackage = readJson("package.json");
const manifest = readJson("packages/extension-core/package.json");
const checks = [];

const docs = {
  readme: readText("README.md"),
  quickStart: readText("docs/quick-start.md"),
  install: readText("docs/install-and-configure.md"),
  developer: readText("docs/developer-guide.md"),
  completeness: readText("docs/plugin-completeness.md"),
  releaseGates: readText("docs/process/ci-and-release-gates.md"),
  taskBoard: readText("docs/progress/task-board.md"),
  support: readText("SUPPORT.md"),
  security: readText("SECURITY.md"),
  featureTemplate: readText(".github/ISSUE_TEMPLATE/feature_request.yml"),
  performanceTemplate: readText(".github/ISSUE_TEMPLATE/performance_regression.yml"),
};
const combinedDocs = Object.values(docs).join("\n");
const scripts = rootPackage.scripts ?? {};
const contributes = manifest.contributes ?? {};
const commands = contributes.commands ?? [];
const commandIds = new Set(commands.map((command) => command.command));
const settings = contributes.configuration?.properties ?? {};
const settingIds = new Set(Object.keys(settings));
const activationEvents = new Set(manifest.activationEvents ?? []);

checkLanguageSurfaces();
checkInteractionReadiness();
checkLspFeatureParity();
checkPerformanceReadiness();
checkDiagnosticsAndDebuggability();
checkOfflineReferenceReadiness();
checkVisualPolishReadiness();
checkMacPackagingReadiness();
checkFutureSageUpdateReadiness();
checkMaintenanceReadiness();
checkModernPluginComparisonCoverage();

const failures = checks.filter((check) => !check.pass);
const report = {
  schema_version: 1,
  status: failures.length ? "failed" : "passed",
  summary: summarizeChecks(checks),
  checks,
};
if (args.json) {
  console.log(JSON.stringify(report, null, 2));
} else {
  printHuman(report);
}
if (failures.length > 0) {
  process.exitCode = 1;
}

function checkLanguageSurfaces() {
  const languages = contributes.languages ?? [];
  const grammars = contributes.grammars ?? [];
  const snippets = contributes.snippets ?? [];
  const languageIds = new Set(languages.map((language) => language.id));
  const grammarLanguages = new Set(grammars.map((grammar) => grammar.language));
  const snippetLanguages = new Set(snippets.map((snippet) => snippet.language));

  for (const id of ["sagemath", "sagemath-cython"]) {
    pushCheck("language-surface", `${id} language is contributed`, languageIds.has(id), [...languageIds]);
    pushCheck("language-surface", `${id} grammar is contributed`, grammarLanguages.has(id), [...grammarLanguages]);
    pushCheck("language-surface", `${id} snippets are contributed`, snippetLanguages.has(id), [...snippetLanguages]);
  }

  for (const fixture of [
    "examples/manual-smoke-workspace/src/01_hover_and_definition.sage",
    "examples/manual-smoke-workspace/src/08_highlighting_structures.sage",
    "examples/manual-smoke-workspace/src/10_sage_heavy_python.py",
    "examples/manual-smoke-workspace/src/11_sage_object_methods.py",
    "examples/manual-smoke-workspace/src/cythonish_bridge.pyx",
    "examples/manual-smoke-workspace/src/native_support.pxd",
    "examples/manual-smoke-workspace/src/native_include.pxi",
  ]) {
    pushCheck("language-surface", `manual smoke fixture exists: ${fixture}`, exists(fixture), fixture);
  }
}

function checkInteractionReadiness() {
  const requiredCommands = [
    "sage.openGettingStarted",
    "sage.selectInterpreter",
    "sage.configureWorkspace",
    "sage.showEnvironmentDetails",
    "sage.showIndexStatus",
    "sage.showDocsStatus",
    "sage.showDocumentation",
    "sage.findReferences",
    "sage.runUxSelfCheck",
    "sage.copySupportBundle",
    "sage.rebuildIndex",
    "sage.startRepl",
    "sage.runCurrentFile",
    "sage.runSelection",
    "sage.runCurrentCell",
  ];
  for (const command of requiredCommands) {
    pushCheck("interaction", `${command} is contributed`, commandIds.has(command), [...commandIds]);
    pushCheck("interaction", `${command} has activation event`, activationEvents.has(`onCommand:${command}`), [...activationEvents]);
  }
  for (const command of commands) {
    pushCheck("interaction", `${command.command} has Sage category`, command.category === "Sage", command);
    pushCheck("interaction", `${command.command} has a VS Code icon`, /^\$\([^)]+\)$/.test(command.icon ?? ""), command.icon);
  }
  const walkthroughs = contributes.walkthroughs ?? [];
  const gettingStarted = walkthroughs.find((walkthrough) => walkthrough.id === "gettingStarted");
  pushCheck("interaction", "getting started walkthrough exists", Boolean(gettingStarted), walkthroughs.map((walkthrough) => walkthrough.id));
  pushCheck("interaction", "walkthrough covers at least four first-run steps", (gettingStarted?.steps?.length ?? 0) >= 4, gettingStarted?.steps?.map((step) => step.id));
  pushCheck("interaction", "context menu exposes docs/references/UX self-check", hasMenuCommand("editor/context", "sage.showDocumentation")
    && hasMenuCommand("editor/context", "sage.findReferences")
    && hasMenuCommand("editor/context", "sage.runUxSelfCheck"), contributes.menus?.["editor/context"]);
}

function checkLspFeatureParity() {
  for (const feature of [
    "hover",
    "documentation",
    "definition",
    "completion",
    "signature help",
    "inlay hints",
    "diagnostics",
    "semantic tokens",
    "document symbols",
    "workspace symbols",
    "references",
    "rename",
  ]) {
    pushCheck("lsp-feature-parity", `README documents ${feature}`, includesCaseInsensitive(docs.readme, feature), "README.md");
  }
  for (const request of [
    "query_source_at",
    "query_source_at_navigation",
    "query_source_symbol",
    "query_source_symbol_with_options",
    "QueryFeatures::hover",
    "QueryFeatures::navigation",
    "documentation_for_symbol",
  ]) {
    pushCheck("lsp-feature-parity", `shared Rust query path includes ${request}`, sourceContains("crates", request), request);
  }
  for (const featureSwitch of ["completions", "references", "rename_preview", "signature", "diagnostics"]) {
    pushCheck("lsp-feature-parity", `query feature switch exists: ${featureSwitch}`, sourceContains("crates", featureSwitch), featureSwitch);
  }
}

function checkPerformanceReadiness() {
  for (const scriptName of ["test:performance", "test:lsp-latency", "test:real-file-smoke", "test:debug-web"]) {
    pushCheck("performance", `${scriptName} script exists`, Boolean(scripts[scriptName]), scripts[scriptName]);
  }
  pushCheck("performance", "release gate includes persistent LSP latency", includesScript("test:release", "npm run test:lsp-latency"), scripts["test:release"]);
  pushCheck("performance", "release gate includes real Sage-heavy smoke", includesScript("test:release", "npm run test:real-file-smoke"), scripts["test:release"]);
  pushCheck("performance", "performance regression template asks for latency evidence", docs.performanceTemplate.includes("observed latency")
    && docs.performanceTemplate.includes("npm run test:lsp-latency"), ".github/ISSUE_TEMPLATE/performance_regression.yml");
  pushCheck("performance", "task board tracks V2 latency budgets", docs.taskBoard.includes("V2-STABLE")
    && docs.taskBoard.includes("latency budgets"), "docs/progress/task-board.md");
}

function checkDiagnosticsAndDebuggability() {
  for (const command of ["sage.showEnvironmentDetails", "sage.showIndexStatus", "sage.showDocsStatus", "sage.copySupportBundle", "sage.runUxSelfCheck"]) {
    pushCheck("debuggability", `${command} is documented`, combinedDocs.includes(command) || commandDocsTitlePresent(command), command);
  }
  pushCheck("debuggability", "support bundle avoids source contents by policy", docs.support.includes("without source contents")
    || docs.completeness.includes("without source contents"), "SUPPORT.md / docs/plugin-completeness.md");
  pushCheck("debuggability", "browser workbench exposes UX matrix", readText("scripts/debug-workbench.mjs").includes("UX Defect Matrix")
    && readText("scripts/debug-workbench.mjs").includes("Run UX Matrix"), "scripts/debug-workbench.mjs");
  pushCheck("debuggability", "debug web smoke validates query documentation and latency", readText("scripts/debug-workbench.mjs").includes("expected documentation payload")
    && readText("scripts/debug-workbench.mjs").includes("latency budget exceeded"), "scripts/debug-workbench.mjs");
  pushCheck("debuggability", "documentation webview supports find without enabling scripts", readText("packages/extension-core/src/docsPanel.ts").includes("enableFindWidget: true")
    && readText("packages/extension-core/src/docsPanel.ts").includes("enableScripts: false"), "packages/extension-core/src/docsPanel.ts");
}

function checkOfflineReferenceReadiness() {
  const exporter = readText("scripts/export-reference.mjs");
  const viewerHtml = readText("scripts/reference-viewer/index.html");
  const viewerJs = readText("scripts/reference-viewer/viewer.js");
  const viewerCss = readText("scripts/reference-viewer/viewer.css");
  const smoke = readText("scripts/reference-export-smoke.mjs");
  pushCheck("offline-reference", "offline reference export script is registered", scripts["export:reference"] === "node scripts/export-reference.mjs"
    && exists("scripts/export-reference.mjs"), scripts["export:reference"]);
  pushCheck("offline-reference", "offline reference export smoke is registered", scripts["test:reference-export"] === "npm run build:debug-inspector && node scripts/reference-export-smoke.mjs"
    && exists("scripts/reference-export-smoke.mjs"), scripts["test:reference-export"]);
  pushCheck("offline-reference", "CI and release gates include offline reference export smoke", includesScript("test:ci", "npm run test:reference-export")
    && includesScript("test:release", "npm run test:reference-export"), {
    ci: scripts["test:ci"],
    release: scripts["test:release"],
  });
  pushCheck("offline-reference", "exporter writes a static no-server viewer", exporter.includes(".sage-reference")
    && exporter.includes("referenceViewerRoot")
    && exporter.includes("fs.copyFile")
    && exporter.includes("index.html")
    && exporter.includes("viewer.js")
    && exporter.includes("manifest.js")
    && exporter.includes("sources"), "scripts/export-reference.mjs");
  pushCheck("offline-reference", "viewer supports search, hash restore, theme, references, and source lazy loading", viewerJs.includes("searchIndex")
    && viewerJs.includes("restoreFromHash")
    && viewerJs.includes("themeButton")
    && viewerJs.includes("referenceList")
    && viewerJs.includes("loadSource"), "scripts/reference-viewer/viewer.js");
  pushCheck("offline-reference", "viewer assets are maintainable and responsive", exporter.includes("reference-viewer")
    && viewerHtml.includes("mobile-tabs")
    && viewerJs.includes("setPanel")
    && viewerCss.includes("@media (max-width: 1100px)"), "scripts/reference-viewer");
  pushCheck("offline-reference", "exporter strips private absolute paths", exporter.includes("sanitizeText")
    && exporter.includes("assertNoPrivatePaths")
    && smoke.includes("generated package avoids private paths"), "scripts/export-reference.mjs / scripts/reference-export-smoke.mjs");
  pushCheck("offline-reference", "public docs explain offline reference export", combinedDocs.includes("export:reference")
    && includesCaseInsensitive(combinedDocs, "offline reference"), "README.md / docs");
}

function checkVisualPolishReadiness() {
  const iconPath = manifest.icon ? path.join(packageRoot, manifest.icon) : "";
  const iconDimensions = iconPath && fs.existsSync(iconPath) ? pngDimensions(iconPath) : { width: 0, height: 0 };
  pushCheck("visual-polish", "extension manifest has a packaged icon", manifest.icon === "resources/branding/icon.png"
    && fs.existsSync(iconPath), manifest.icon);
  pushCheck("visual-polish", "extension icon is 256x256 PNG", iconDimensions.width === 256 && iconDimensions.height === 256, iconDimensions);
  pushCheck("visual-polish", "gallery banner is configured with restrained dark theme", manifest.galleryBanner?.theme === "dark"
    && /^#[0-9a-f]{6}$/i.test(manifest.galleryBanner?.color ?? ""), manifest.galleryBanner);
  pushCheck("visual-polish", "all commands have VS Code product icons", commands.every((command) => /^\$\([^)]+\)$/.test(command.icon ?? "")), commands.map((command) => ({
    command: command.command,
    icon: command.icon,
  })));
  const walkthroughSteps = contributes.walkthroughs?.flatMap((walkthrough) => walkthrough.steps ?? []) ?? [];
  for (const step of walkthroughSteps) {
    const markdownPath = step.media?.markdown;
    const relativePath = markdownPath ? `packages/extension-core/${markdownPath.replace(/^\.\//, "")}` : "";
    pushCheck("visual-polish", `walkthrough step has markdown media: ${step.id}`, Boolean(markdownPath) && exists(relativePath), markdownPath ?? step.id);
  }
  pushCheck("visual-polish", "visual polish is part of public readiness docs", includesCaseInsensitive(combinedDocs, "visual polish")
    || includesCaseInsensitive(combinedDocs, "interface"), "docs/plugin-completeness.md");
}

function checkMacPackagingReadiness() {
  const packageRust = readText("scripts/package-rust-binary.mjs");
  const macDoctor = readText("scripts/macos-doctor.mjs");
  const workflow = readText(".github/workflows/ci.yml");
  pushCheck("mac-packaging", "GitHub CI uses macOS", workflow.includes("runs-on: macos-latest"), ".github/workflows/ci.yml");
  pushCheck("mac-packaging", "macOS doctor script is registered", scripts["doctor:mac"] === "node scripts/macos-doctor.mjs"
    && exists("scripts/macos-doctor.mjs"), scripts["doctor:mac"]);
  pushCheck("mac-packaging", "macOS doctor covers package, VS Code, Sage runtime, and source root", macDoctor.includes("VSIX package exists")
    && macDoctor.includes("packaged Rust language server")
    && macDoctor.includes("VS Code CLI")
    && macDoctor.includes("Sage runtime")
    && macDoctor.includes("Sage source root"), "scripts/macos-doctor.mjs");
  pushCheck("mac-packaging", "packaging supports Apple Silicon and Intel Mac", packageRust.includes("darwin-arm64")
    && packageRust.includes("darwin-x64"), "scripts/package-rust-binary.mjs");
  pushCheck("mac-packaging", "packaging rejects non-macOS release targets", packageRust.includes("This preview only stages macOS binaries")
    && !packageRust.includes("linux-x64")
    && !packageRust.includes("win32-x64"), "scripts/package-rust-binary.mjs");
  pushCheck("mac-packaging", "VSIX smokes require packaged macOS binary", readText("scripts/vsix-contents-smoke.mjs").includes("VSIX binary target is macOS")
    && readText("scripts/vsix-package-smoke.mjs").includes("unsupported-non-macos-platform"), "scripts/vsix-contents-smoke.mjs");
  pushCheck("mac-packaging", "cleanup script protects source and removes generated artifacts", Boolean(scripts.clean)
    && Boolean(scripts["clean:dry-run"])
    && readText("scripts/clean-artifacts.mjs").includes("Default mode is a dry run"), "scripts/clean-artifacts.mjs");
}

function checkFutureSageUpdateReadiness() {
  const configureWorkspace = readText("scripts/configure-workspace.mjs");
  pushCheck("future-sage-updates", "cross-platform workspace configure script is registered", scripts["configure:workspace"] === "node scripts/configure-workspace.mjs"
    && exists("scripts/configure-workspace.mjs"), scripts["configure:workspace"]);
  pushCheck("future-sage-updates", "workspace configure script can pin Sage runtime and source roots", configureWorkspace.includes("--sage")
    && configureWorkspace.includes("--source-root")
    && configureWorkspace.includes("sage.analysis.sourceRoots")
    && configureWorkspace.includes("sage.interpreter.path"), "scripts/configure-workspace.mjs");
  for (const setting of [
    "sage.analysis.sourceRoots",
    "sage.analysis.extraPaths",
    "sage.analysis.enableRuntimeIntrospection",
    "sage.docs.preferredSource",
    "sage.indexing.exclude",
  ]) {
    pushCheck("future-sage-updates", `${setting} setting exists`, settingIds.has(setting), [...settingIds]);
  }
  for (const doc of [
    "docs/design/runtime-source-root-discovery.md",
    "docs/design/runtime-introspection-fallback.md",
    "docs/design/source-mapping-strategy.md",
  ]) {
    pushCheck("future-sage-updates", `design note exists: ${doc}`, exists(doc), doc);
  }
  for (const fixture of [
    "examples/manual-smoke-workspace/src/02_star_import_and_completion.sage",
    "examples/manual-smoke-workspace/src/04_lazy_import_and_packages.sage",
    "examples/manual-smoke-workspace/src/06_runtime_graphs_and_number_theory.sage",
    "examples/manual-smoke-workspace/src/11_sage_object_methods.py",
  ]) {
    pushCheck("future-sage-updates", `dynamic Sage fixture exists: ${fixture}`, exists(fixture), fixture);
  }
}

function checkMaintenanceReadiness() {
  for (const scriptName of ["test:ci", "test:release", "test:repo-hygiene", "test:generated-assets", "test:vsix-package"]) {
    pushCheck("maintainability", `${scriptName} script exists`, Boolean(scripts[scriptName]), scripts[scriptName]);
  }
  pushCheck("maintainability", "release gate includes product readiness", includesScript("test:release", "npm run test:product-readiness"), scripts["test:release"]);
  pushCheck("maintainability", "CI gate includes product readiness", includesScript("test:ci", "npm run test:product-readiness"), scripts["test:ci"]);
  for (const doc of [
    "CONTRIBUTING.md",
    "SECURITY.md",
    "SUPPORT.md",
    "docs/developer-guide.md",
    "docs/plugin-completeness.md",
    "docs/progress/task-board.md",
  ]) {
    pushCheck("maintainability", `maintenance doc exists: ${doc}`, exists(doc), doc);
  }
}

function checkModernPluginComparisonCoverage() {
  pushCheck("modern-plugin-comparison", "feature template asks for Pyright/rust-analyzer comparison", docs.featureTemplate.includes("Pyright")
    && docs.featureTemplate.includes("rust-analyzer"), ".github/ISSUE_TEMPLATE/feature_request.yml");
  for (const dimension of ["interaction", "latency", "status", "workspace", "diagnostics", "visual polish"]) {
    pushCheck("modern-plugin-comparison", `public docs mention ${dimension} dimension`, includesCaseInsensitive(combinedDocs, dimension), dimension);
  }
  pushCheck("modern-plugin-comparison", "extension uses workspace trust and virtual workspace capability limits", manifest.capabilities?.untrustedWorkspaces?.supported === "limited"
    && (typeof manifest.capabilities?.virtualWorkspaces === "object"
      ? manifest.capabilities.virtualWorkspaces.supported === "limited"
      : manifest.capabilities?.virtualWorkspaces === "limited"), manifest.capabilities);
}

function printHuman(report) {
  console.log(`Sage VS Code product readiness: ${report.status}`);
  console.log("");
  for (const [category, summary] of Object.entries(report.summary)) {
    const total = summary.passed + summary.failed;
    const marker = summary.failed === 0 ? "PASS" : "FAIL";
    console.log(`${marker} ${category}: ${summary.passed}/${total}`);
  }
  if (report.status === "passed") {
    console.log("");
    console.log("All product readiness checks passed.");
    console.log("Run `npm run test:product-readiness -- --json` for machine-readable details.");
    return;
  }
  console.log("");
  console.log("Failures:");
  for (const failure of report.checks.filter((check) => !check.pass)) {
    console.log(`- [${failure.category}] ${failure.name}`);
    console.log(`  actual: ${JSON.stringify(failure.actual)}`);
  }
}

function parseArgs(values) {
  const parsed = { json: false };
  for (const value of values) {
    if (value === "--json") {
      parsed.json = true;
    } else if (value === "--help" || value === "-h") {
      console.log(`Usage: node scripts/product-readiness-smoke.mjs [--json]

Default output is a compact human-readable product readiness matrix.
Use --json for complete machine-readable check details.`);
      process.exit(0);
    } else {
      throw new Error(`Unknown argument: ${value}`);
    }
  }
  return parsed;
}

function pushCheck(category, name, pass, actual) {
  checks.push({
    category,
    name,
    pass: Boolean(pass),
    actual,
  });
}

function summarizeChecks(items) {
  const summary = {};
  for (const item of items) {
    const current = summary[item.category] ?? { passed: 0, failed: 0 };
    if (item.pass) {
      current.passed += 1;
    } else {
      current.failed += 1;
    }
    summary[item.category] = current;
  }
  return summary;
}

function hasMenuCommand(menu, commandId) {
  return (contributes.menus?.[menu] ?? []).some((item) => item.command === commandId);
}

function includesScript(name, snippet) {
  return String(scripts[name] ?? "").includes(snippet);
}

function commandDocsTitlePresent(commandId) {
  const command = commands.find((candidate) => candidate.command === commandId);
  return command ? combinedDocs.includes(command.title) : false;
}

function includesCaseInsensitive(text, needle) {
  return text.toLowerCase().includes(needle.toLowerCase());
}

function sourceContains(relativeDirectory, needle) {
  const root = path.join(repositoryRoot, relativeDirectory);
  if (!fs.existsSync(root)) {
    return false;
  }
  let found = false;
  walkFiles(root, (filePath) => {
    if (!found && fs.readFileSync(filePath, "utf8").includes(needle)) {
      found = true;
    }
  });
  return found;
}

function walkFiles(root, visit) {
  for (const entry of fs.readdirSync(root, { withFileTypes: true })) {
    const absolutePath = path.join(root, entry.name);
    if (entry.isDirectory()) {
      walkFiles(absolutePath, visit);
    } else if (entry.isFile()) {
      visit(absolutePath);
    }
  }
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

function pngDimensions(filePath) {
  const buffer = fs.readFileSync(filePath);
  const signature = buffer.subarray(0, 8).toString("hex");
  if (signature !== "89504e470d0a1a0a") {
    return { width: 0, height: 0, error: "not a png" };
  }
  return {
    width: buffer.readUInt32BE(16),
    height: buffer.readUInt32BE(20),
  };
}
