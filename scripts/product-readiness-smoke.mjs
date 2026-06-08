#!/usr/bin/env node
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const repositoryRoot = path.resolve(__dirname, "..");
const packageRoot = path.join(repositoryRoot, "packages", "extension-core");

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
checkMacPackagingReadiness();
checkFutureSageUpdateReadiness();
checkMaintenanceReadiness();
checkModernPluginComparisonCoverage();

const failures = checks.filter((check) => !check.pass);
console.log(JSON.stringify({
  schema_version: 1,
  status: failures.length ? "failed" : "passed",
  summary: summarizeChecks(checks),
  checks,
}, null, 2));
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
}

function checkMacPackagingReadiness() {
  const packageRust = readText("scripts/package-rust-binary.mjs");
  const workflow = readText(".github/workflows/ci.yml");
  pushCheck("mac-packaging", "GitHub CI uses macOS", workflow.includes("runs-on: macos-latest"), ".github/workflows/ci.yml");
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
  for (const dimension of ["interaction", "latency", "status", "workspace", "diagnostics"]) {
    pushCheck("modern-plugin-comparison", `public docs mention ${dimension} dimension`, includesCaseInsensitive(combinedDocs, dimension), dimension);
  }
  pushCheck("modern-plugin-comparison", "extension uses workspace trust and virtual workspace capability limits", manifest.capabilities?.untrustedWorkspaces?.supported === "limited"
    && (typeof manifest.capabilities?.virtualWorkspaces === "object"
      ? manifest.capabilities.virtualWorkspaces.supported === "limited"
      : manifest.capabilities?.virtualWorkspaces === "limited"), manifest.capabilities);
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
