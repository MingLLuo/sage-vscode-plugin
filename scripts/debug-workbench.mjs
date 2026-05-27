#!/usr/bin/env node
import { execFile } from "node:child_process";
import fsSync from "node:fs";
import fs from "node:fs/promises";
import http from "node:http";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { promisify } from "node:util";

const execFileAsync = promisify(execFile);
const __dirname = path.dirname(fileURLToPath(import.meta.url));
const repositoryRoot = path.resolve(__dirname, "..");
const smokeRoot = path.join(repositoryRoot, "examples", "manual-smoke-workspace");
const smokeSrc = path.join(smokeRoot, "src");
const smokeVendor = path.join(smokeRoot, "vendor");
const nearbySageRoots = discoverNearbySageSourceRoots(smokeRoot);
const indexRoots = dedupe([smokeSrc, smokeVendor, ...nearbySageRoots]);
const editableRoots = [smokeRoot];
const grammarPath = path.join(repositoryRoot, "packages", "syntax-pack", "syntaxes", "sagemath.tmLanguage.json");
const debugInspector = path.join(repositoryRoot, "target", "debug", process.platform === "win32" ? "sage-debug-inspect.exe" : "sage-debug-inspect");
const debugCacheHome = process.env.SAGE_DEBUG_WEB_CACHE_DIR ?? path.join(os.tmpdir(), "sage-vscode-debug-cache");
const defaultFile = "08_highlighting_structures.sage";
const responseCaches = {
  epoch: 0,
  inspect: new Map(),
  query: new Map(),
};

const smokeFiles = [
  "01_hover_and_definition.sage",
  "03_source_mapping_cases.sage",
  "04_lazy_import_and_packages.sage",
  "05_symbols_and_locals.sage",
  "06_runtime_graphs_and_number_theory.sage",
  "07_symbolic_and_combinatorics.sage",
  "08_highlighting_structures.sage",
  "09_advanced_sage_patterns.sage",
  "10_sage_heavy_python.py",
  "11_sage_object_methods.py",
  "cythonish_bridge.pyx",
  "native_support.pxd",
  "native_include.pxi",
];

const defaultQueries = {
  "01_hover_and_definition.sage": "make_demo_matrix",
  "03_source_mapping_cases.sage": "local_power_report",
  "04_lazy_import_and_packages.sage": "alt_square_sum",
  "05_symbols_and_locals.sage": "LocalContainer",
  "06_runtime_graphs_and_number_theory.sage": "graphs.PetersenGraph",
  "07_symbolic_and_combinatorics.sage": "NumberField",
  "08_highlighting_structures.sage": "PolynomialRing",
  "09_advanced_sage_patterns.sage": "FunctionField",
  "10_sage_heavy_python.py": "PolynomialRing",
  "11_sage_object_methods.py": "NumberField",
  "cythonish_bridge.pyx": "NativeAccumulator",
  "native_support.pxd": "native_step",
  "native_include.pxi": "included_native_step",
};

const uxScenarios = [
  {
    id: "workspace-edit-loop",
    title: "Workspace edit loop",
    file: "01_hover_and_definition.sage",
    symbol: "make_demo_matrix",
    expects: {
      hoverIncludes: ["small nested list"],
      docsIncludes: ["small nested list"],
      definitionPathIncludes: "src/local_docs.py",
      signatureIncludes: "make_demo_matrix()",
      completionLabels: ["make_demo_matrix"],
      minReferences: 2,
      minRenameEdits: 2,
    },
  },
  {
    id: "docs-panel-consistency",
    title: "Hover and docs panel consistency",
    file: "01_hover_and_definition.sage",
    symbol: "summarize_coefficients",
    expects: {
      hoverIncludes: ["comma-separated summary"],
      docsIncludes: ["comma-separated summary"],
      definitionPathIncludes: "src/local_docs.py",
      signatureIncludes: "summarize_coefficients(values: list[int])",
      minReferences: 2,
      minRenameEdits: 2,
    },
  },
  {
    id: "lazy-import-vendor",
    title: "Lazy import and vendor source root",
    file: "04_lazy_import_and_packages.sage",
    symbol: "alt_square_sum",
    expects: {
      hoverIncludes: ["alternating sum"],
      docsIncludes: ["alternating sum"],
      definitionPathIncludes: "vendor/external_series.py",
      signatureIncludes: "alternating_square_sum(limit: int)",
      minReferences: 1,
      minRenameEdits: 1,
    },
  },
  {
    id: "sage-internal-library",
    title: "Sage internal library docs and navigation",
    file: "06_runtime_graphs_and_number_theory.sage",
    symbol: "graphs.PetersenGraph",
    expects: {
      hoverIncludes: ["Petersen Graph"],
      docsIncludes: ["Petersen Graph"],
      definitionPathIncludes: "sage/src/sage/graphs/generators/smallgraphs.py",
      docsWorkerState: "static-fallback",
      docsDegraded: true,
    },
  },
  {
    id: "highlighting-structures",
    title: "Sage highlighting structures",
    file: "08_highlighting_structures.sage",
    symbol: "PolynomialRing",
    expects: {
      hoverIncludes: ["PolynomialRing"],
      docsIncludes: ["PolynomialRing"],
      grammarScopes: [
        "entity.name.namespace.sagemath",
        "support.class.constructor.sagemath",
        "storage.type.decorator.cache.sagemath",
        "variable.parameter.preparse.generator.sagemath",
        "variable.parameter.keyword.sagemath",
        "support.function.method.sagemath",
      ],
      semanticTypes: ["namespace", "type", "decorator", "parameter"],
    },
  },
  {
    id: "catalog-namespace-member",
    title: "Sage catalog namespace member navigation",
    file: "08_highlighting_structures.sage",
    symbol: "codes",
    expects: {
      noDiagnostics: true,
      hoverIncludes: ["Hamming"],
      docsIncludes: ["Hamming"],
      definitionPathIncludes: "sage/src/sage/coding/hamming_code.py",
      resolutionConfidence: "high",
      semanticTypes: ["namespace"],
      allowMissingReferences: true,
    },
  },
  {
    id: "advanced-sage-patterns",
    title: "Advanced Sage implementation patterns",
    file: "09_advanced_sage_patterns.sage",
    symbol: "FunctionField",
    expects: {
      noDiagnostics: true,
      grammarScopes: [
        "support.class.constructor.sagemath",
        "variable.parameter.preparse.generator.sagemath",
        "variable.parameter.keyword.sagemath",
        "support.function.method.sagemath",
        "keyword.operator.range.sagemath",
      ],
      semanticTypes: ["type", "parameter"],
    },
  },
  {
    id: "advanced-keyword-call-signature",
    title: "Advanced keyword call signature",
    file: "09_advanced_sage_patterns.sage",
    line: 69,
    character: 35,
    expects: {
      noDiagnostics: true,
      signatureIncludes: "trace_window(poly, base_ring=QQ, *, width=5, normalize=True)",
      signatureActiveParameter: 1,
      allowFallbackReason: true,
    },
  },
  {
    id: "sage-heavy-python-polynomial-ring",
    title: "Sage-heavy Python constructor resolution",
    file: "10_sage_heavy_python.py",
    symbol: "PolynomialRing",
    expects: {
      noDiagnostics: true,
      hoverIncludes: ["PolynomialRing"],
      docsIncludes: ["PolynomialRing"],
      definitionPathIncludes: "sage/src/sage/rings/polynomial/polynomial_ring_constructor.py",
      resolutionConfidence: "high",
    },
  },
  {
    id: "sage-heavy-python-matrix-rank",
    title: "Sage-heavy Python matrix method resolution",
    file: "10_sage_heavy_python.py",
    line: 22,
    character: 14,
    expects: {
      noDiagnostics: true,
      definitionPathIncludes: "sage/src/sage/matrix/matrix0.pyx",
      completionLabels: ["rank"],
      ownerType: "Matrix",
      resolutionConfidence: "high",
    },
  },
  {
    id: "sage-heavy-python-polynomial-derivative",
    title: "Sage-heavy Python indexed polynomial method resolution",
    file: "10_sage_heavy_python.py",
    line: 109,
    character: 18,
    expects: {
      noDiagnostics: true,
      definitionPathIncludes: "sage/src/sage/rings/polynomial/multi_polynomial.pyx",
      ownerType: "PolynomialElement",
      resolutionConfidence: "high",
    },
  },
  {
    id: "sage-heavy-python-ring-ideal",
    title: "Sage-heavy Python polynomial ring ideal resolution",
    file: "10_sage_heavy_python.py",
    line: 87,
    character: 21,
    expects: {
      noDiagnostics: true,
      definitionPathIncludes: "sage/src/sage/rings/polynomial/multi_polynomial_libsingular.pyx",
      ownerType: "PolynomialRing",
      resolutionConfidence: "high",
    },
  },
  {
    id: "sage-heavy-python-ideal-variety",
    title: "Sage-heavy Python ideal variety resolution",
    file: "10_sage_heavy_python.py",
    line: 115,
    character: 17,
    expects: {
      noDiagnostics: true,
      definitionPathIncludes: "sage/src/sage/rings/polynomial/multi_polynomial_ideal.py",
      ownerType: "Ideal",
      resolutionConfidence: "high",
    },
  },
  {
    id: "sage-heavy-python-resultant",
    title: "Sage-heavy Python polynomial resultant resolution",
    file: "10_sage_heavy_python.py",
    line: 105,
    character: 33,
    expects: {
      noDiagnostics: true,
      definitionPathIncludes: "sage/src/sage/rings/polynomial/multi_polynomial_element.py",
      ownerType: "PolynomialElement",
      resolutionConfidence: "high",
    },
  },
  {
    id: "sage-object-graph-method",
    title: "Sage object Graph method resolution",
    file: "11_sage_object_methods.py",
    line: 25,
    character: 26,
    expects: {
      noDiagnostics: true,
      definitionPathIncludes: "sage/src/sage/graphs/generic_graph.py",
      ownerType: "Graph",
      resolutionConfidence: "high",
    },
  },
  {
    id: "sage-object-elliptic-curve-method",
    title: "Sage object EllipticCurve method resolution",
    file: "11_sage_object_methods.py",
    line: 42,
    character: 31,
    expects: {
      noDiagnostics: true,
      definitionPathIncludes: "sage/src/sage/schemes/elliptic_curves/ell_finite_field.py",
      ownerType: "EllipticCurve",
      resolutionConfidence: "high",
    },
  },
  {
    id: "sage-object-number-field-method",
    title: "Sage object NumberField method resolution",
    file: "11_sage_object_methods.py",
    line: 66,
    character: 26,
    expects: {
      noDiagnostics: true,
      definitionPathIncludes: "sage/src/sage/rings/number_field/number_field_base.pyx",
      ownerType: "NumberField",
      resolutionConfidence: "high",
    },
  },
  {
    id: "implicit-sage-all-combinations",
    title: "Implicit .sage sage.all combinatorics constructor resolution",
    file: "07_symbolic_and_combinatorics.sage",
    symbol: "Combinations",
    expects: {
      noDiagnostics: true,
      hoverIncludes: ["Combinations"],
      docsIncludes: ["combinations"],
      definitionPathIncludes: "sage/src/sage/combinat/combination.py",
      signatureIncludes: "Combinations(",
      resolutionConfidence: "high",
      resolutionReasonIncludes: "implicit .sage",
    },
  },
  {
    id: "source-map-preparser",
    title: "Sage source map and preparser diagnostics",
    file: "03_source_mapping_cases.sage",
    symbol: "local_power_report",
    expects: {
      noDiagnostics: true,
      preprocessGeneratedIncludes: ["**"],
      hoverIncludes: ["local_power_report"],
      definitionPathIncludes: "src/03_source_mapping_cases.sage",
    },
  },
  {
    id: "cython-navigation",
    title: "Cython native navigation",
    file: "cythonish_bridge.pyx",
    symbol: "NativeAccumulator",
    expects: {
      hoverIncludes: ["NativeAccumulator"],
      docsIncludes: ["NativeAccumulator"],
      definitionPathIncludes: "src/native_support.pxd",
      grammarScopes: [
        "keyword.control.include.cython.sagemath",
        "keyword.control.import.cython.sagemath",
        "string.quoted.include.cython.sagemath",
      ],
      minReferences: 2,
      minRenameEdits: 2,
    },
  },
  {
    id: "pxd-signature",
    title: ".pxd declaration signature",
    file: "native_support.pxd",
    symbol: "native_step",
    expects: {
      docsIncludes: ["native_step"],
      definitionPathIncludes: "src/native_support.pxd",
      signatureIncludes: "native_step(int value)",
      minReferences: 1,
      minRenameEdits: 1,
    },
  },
  {
    id: "pxi-signature",
    title: ".pxi include signature",
    file: "native_include.pxi",
    symbol: "included_native_step",
    expects: {
      docsIncludes: ["included_native_step"],
      definitionPathIncludes: "src/native_include.pxi",
      signatureIncludes: "included_native_step(int value)",
      minReferences: 1,
      minRenameEdits: 1,
    },
  },
];

const args = new Set(process.argv.slice(2));
if (args.has("--smoke")) {
  await runSmoke();
} else {
  const port = Number(process.env.SAGE_DEBUG_WEB_PORT ?? 8765);
  const server = http.createServer(handleRequest);
  server.listen(port, "127.0.0.1", () => {
    console.log(`Sage debug workbench: http://127.0.0.1:${port}/`);
  });
}

async function handleRequest(request, response) {
  try {
    const url = new URL(request.url ?? "/", "http://127.0.0.1");
    if (url.pathname === "/") {
      return sendHtml(response, renderPage());
    }
    if (url.pathname === "/api/files") {
      return sendJson(response, { files: smokeFiles, defaultFile });
    }
    if (url.pathname === "/api/inspect") {
      const file = url.searchParams.get("file") ?? defaultFile;
      const rebuild = url.searchParams.get("rebuild") === "1";
      return sendJson(response, await timed(() => inspectSmokeFile(file, { rebuild })));
    }
    if (url.pathname === "/api/query") {
      const file = url.searchParams.get("file") ?? defaultFile;
      const symbol = url.searchParams.get("symbol") ?? defaultQueries[file] ?? "";
      const renameTo = url.searchParams.get("renameTo") ?? "sage_debug_renamed";
      const line = optionalInteger(url.searchParams.get("line"));
      const character = optionalInteger(url.searchParams.get("character"));
      return sendJson(response, await timed(() => querySmokeFile(file, { symbol, renameTo, line, character })));
    }
    if (url.pathname === "/api/ux") {
      return sendJson(response, await timed(runUxMatrix));
    }
    sendText(response, 404, "not found");
  } catch (error) {
    sendJson(response, { error: String(error?.stack ?? error) }, 500);
  }
}

async function timed(callback) {
  const started = Date.now();
  const payload = await callback();
  return {
    ...payload,
    timing: {
      ...(payload.timing ?? {}),
      server_ms: Date.now() - started,
    },
  };
}

function optionalInteger(value) {
  if (value === null || value === "") {
    return undefined;
  }
  const parsed = Number(value);
  return Number.isInteger(parsed) ? parsed : undefined;
}

async function inspectSmokeFile(fileName, options = {}) {
  if (options.rebuild) {
    clearResponseCaches();
  }
  const filePath = resolveSmokeFile(fileName);
  const cacheKey = JSON.stringify({ fileName, epoch: responseCaches.epoch });
  if (!options.rebuild && responseCaches.inspect.has(cacheKey)) {
    return {
      ...structuredClone(responseCaches.inspect.get(cacheKey)),
      timing: { cache_hit: true },
    };
  }
  const [source, grammarMatches, rust] = await Promise.all([
    fs.readFile(filePath, "utf-8"),
    scanGrammarScopes(filePath),
    inspectWithRust(filePath, options),
  ]);
  const payload = {
    ...rust,
    file: path.relative(smokeRoot, filePath),
    source,
    grammarMatches,
  };
  responseCaches.inspect.set(cacheKey, payload);
  return structuredClone(payload);
}

function resolveSmokeFile(fileName) {
  if (!smokeFiles.includes(fileName)) {
    throw new Error(`unknown smoke file: ${fileName}`);
  }
  const filePath = fileName.includes("/")
    ? path.resolve(smokeRoot, fileName)
    : path.resolve(smokeSrc, fileName);
  if (!filePath.startsWith(smokeRoot + path.sep)) {
    throw new Error(`file escapes smoke workspace: ${fileName}`);
  }
  return filePath;
}

async function inspectWithRust(filePath, options = {}) {
  try {
    await fs.access(debugInspector);
  } catch {
    throw new Error(`missing ${debugInspector}; run npm run debug:web so the inspector binary is built first`);
  }
  const args = indexRootArgs(filePath);
  if (options.rebuild) {
    args.push("--rebuild-index");
  }
  const { stdout } = await execFileAsync(debugInspector, args, {
    cwd: repositoryRoot,
    env: { ...process.env, XDG_CACHE_HOME: debugCacheHome },
    maxBuffer: 20 * 1024 * 1024,
  });
  return JSON.parse(stdout);
}

async function querySmokeFile(fileName, options = {}) {
  if (options.rebuild) {
    clearResponseCaches();
  }
  const filePath = resolveSmokeFile(fileName);
  const cacheKey = JSON.stringify({
    fileName,
    symbol: options.symbol ?? defaultQueries[fileName] ?? "",
    line: options.line,
    character: options.character,
    renameTo: options.renameTo ?? "sage_debug_renamed",
    epoch: responseCaches.epoch,
  });
  if (!options.rebuild && responseCaches.query.has(cacheKey)) {
    return {
      ...structuredClone(responseCaches.query.get(cacheKey)),
      timing: { cache_hit: true },
    };
  }
  const args = [...indexRootArgs(filePath)];
  if (options.rebuild === true) {
    args.push("--rebuild-index");
  }
  if (Number.isInteger(options.line) && Number.isInteger(options.character)) {
    args.push("--line", String(options.line), "--character", String(options.character));
  } else if (options.symbol) {
    args.push("--symbol", options.symbol);
  } else {
    args.push("--symbol", defaultQueries[fileName] ?? "");
  }
  if (options.renameTo) {
    args.push("--rename-to", options.renameTo);
  }
  const { stdout } = await execFileAsync(debugInspector, args, {
    cwd: repositoryRoot,
    env: { ...process.env, XDG_CACHE_HOME: debugCacheHome },
    maxBuffer: 20 * 1024 * 1024,
  });
  const inspectorPayload = JSON.parse(stdout);
  const payload = {
    file: path.relative(smokeRoot, filePath),
    requestedSymbol: options.symbol,
    query: inspectorPayload.query,
  };
  responseCaches.query.set(cacheKey, payload);
  return structuredClone(payload);
}

function clearResponseCaches() {
  responseCaches.epoch += 1;
  responseCaches.inspect.clear();
  responseCaches.query.clear();
}

async function runUxMatrix() {
  const rows = [];
  let needsRebuild = true;
  for (const scenario of uxScenarios) {
    const inspection = await inspectSmokeFile(scenario.file, { rebuild: needsRebuild });
    needsRebuild = false;
    const queryResult = await querySmokeFile(scenario.file, {
      symbol: scenario.symbol,
      line: scenario.line,
      character: scenario.character,
      renameTo: "sage_debug_renamed",
      rebuild: false,
    });
    const checks = evaluateUxScenario(scenario, inspection, queryResult.query);
    rows.push({
      id: scenario.id,
      title: scenario.title,
      file: scenario.file,
      target: scenario.symbol,
      status: checks.every((check) => check.pass) ? "pass" : "fail",
      checks,
    });
  }
  return {
    rows,
    summary: {
      total: rows.length,
      passed: rows.filter((row) => row.status === "pass").length,
      failed: rows.filter((row) => row.status !== "pass").length,
    },
  };
}

function evaluateUxScenario(scenario, inspection, query) {
  const checks = [];
  const expect = scenario.expects ?? {};
  const hoverText = query?.hover?.markdown ?? "";
  const docsText = [
    query?.documentation?.summary,
    query?.documentation?.docstring,
    query?.documentation?.detail,
    query?.documentation?.uri,
  ].filter(Boolean).join("\n");
  const definitionPath = query?.definition?.path ?? "";
  const signatureLabel = query?.signature?.label ?? "";
  const completionLabels = new Set((query?.completions ?? []).map((item) => item.label));
  const grammarScopes = new Set((inspection.grammarMatches ?? []).map((match) => match.scope));
  const semanticTypes = new Set((inspection.semanticSpans ?? []).map((span) => span.token_type));
  const generated = inspection.preprocess?.generated ?? inspection.source ?? "";

  for (const needle of expect.hoverIncludes ?? []) {
    pushCheck(checks, `hover includes ${needle}`, hoverText.includes(needle), hoverText);
  }
  for (const needle of expect.docsIncludes ?? []) {
    pushCheck(checks, `docs include ${needle}`, docsText.includes(needle), docsText);
  }
  if (expect.definitionPathIncludes) {
    pushCheck(
      checks,
      `definition path includes ${expect.definitionPathIncludes}`,
      normalizePathText(definitionPath).includes(expect.definitionPathIncludes),
      definitionPath || "none",
    );
  }
  if (expect.signatureIncludes) {
    pushCheck(checks, `signature includes ${expect.signatureIncludes}`, signatureLabel.includes(expect.signatureIncludes), signatureLabel || "none");
  }
  if (expect.signatureActiveParameter !== undefined) {
    pushCheck(
      checks,
      `signature active parameter ${expect.signatureActiveParameter}`,
      query?.signature?.active_parameter === expect.signatureActiveParameter,
      query?.signature?.active_parameter ?? "none",
    );
  }
  if (expect.ownerType) {
    pushCheck(checks, `owner type ${expect.ownerType}`, query?.ownerType === expect.ownerType, query?.ownerType ?? "none");
  }
  if (expect.resolutionConfidence) {
    pushCheck(
      checks,
      `resolution confidence ${expect.resolutionConfidence}`,
      query?.resolutionConfidence === expect.resolutionConfidence,
      query?.resolutionConfidence ?? "none",
    );
  }
  if (expect.resolutionReasonIncludes) {
    pushCheck(
      checks,
      `resolution reason includes ${expect.resolutionReasonIncludes}`,
      String(query?.resolutionReason ?? "").includes(expect.resolutionReasonIncludes),
      query?.resolutionReason ?? "none",
    );
  }
  for (const label of expect.completionLabels ?? []) {
    pushCheck(checks, `completion includes ${label}`, completionLabels.has(label), [...completionLabels].slice(0, 20).join(", "));
  }
  if (expect.minReferences !== undefined) {
    pushCheck(checks, `references >= ${expect.minReferences}`, (query?.references?.length ?? 0) >= expect.minReferences, query?.references?.length ?? 0);
  }
  if (expect.minRenameEdits !== undefined) {
    pushCheck(checks, `rename edits >= ${expect.minRenameEdits}`, (query?.rename_preview?.length ?? 0) >= expect.minRenameEdits, query?.rename_preview?.length ?? 0);
  }
  for (const scope of expect.grammarScopes ?? []) {
    pushCheck(checks, `grammar scope ${scope}`, grammarScopes.has(scope), [...grammarScopes].slice(0, 30).join(", "));
  }
  for (const tokenType of expect.semanticTypes ?? []) {
    pushCheck(checks, `semantic token ${tokenType}`, semanticTypes.has(tokenType), [...semanticTypes].join(", "));
  }
  if (expect.noDiagnostics) {
    pushCheck(checks, "no diagnostics", (inspection.diagnostics ?? []).length === 0, inspection.diagnostics ?? []);
  }
  for (const text of expect.preprocessGeneratedIncludes ?? []) {
    pushCheck(checks, `preprocess generated includes ${text}`, generated.includes(text), generated);
  }
  if (expect.docsWorkerState) {
    pushCheck(
      checks,
      `docs worker state ${expect.docsWorkerState}`,
      inspection.docsStatus?.runtime_worker_state === expect.docsWorkerState,
      inspection.docsStatus?.runtime_worker_state ?? "missing",
    );
  }
  if (expect.docsDegraded) {
    pushCheck(
      checks,
      "docs degraded reason visible",
      Boolean(inspection.docsStatus?.runtime_degraded_reason),
      inspection.docsStatus?.runtime_degraded_reason ?? "missing",
    );
  }
  if (!expect.allowFallbackReason) {
    pushCheck(checks, "query fallback is acceptable", !query?.fallback_reason, query?.fallback_reason ?? "none");
  }
  return checks;
}

function pushCheck(checks, name, pass, actual) {
  checks.push({
    name,
    pass: Boolean(pass),
    actual: formatCheckActual(actual),
  });
}

function formatCheckActual(value) {
  if (value == null) return "";
  const text = typeof value === "string" ? value : JSON.stringify(value);
  return text.length > 260 ? `${text.slice(0, 257)}...` : text;
}

function normalizePathText(value) {
  return String(value).replaceAll(path.sep, "/");
}

function indexRootArgs(filePath) {
  return [
    ...indexRoots.flatMap((root) => ["--root", root]),
    ...editableRoots.flatMap((root) => ["--editable-root", root]),
    "--file",
    filePath,
  ];
}

function discoverNearbySageSourceRoots(workspaceRoot) {
  const roots = [];
  let current = path.resolve(workspaceRoot);
  for (let depth = 0; depth < 8; depth += 1) {
    const directSrc = path.join(current, "src");
    if (fsSync.existsSync(path.join(directSrc, "sage"))) {
      roots.push(directSrc);
    }

    const siblingSageSrc = path.join(current, "sage", "src");
    if (fsSync.existsSync(path.join(siblingSageSrc, "sage"))) {
      roots.push(siblingSageSrc);
    }

    const parent = path.dirname(current);
    if (parent === current) {
      break;
    }
    current = parent;
  }
  return dedupe(roots.map((root) => path.resolve(root)));
}

function dedupe(values) {
  return [...new Set(values)];
}

async function scanGrammarScopes(filePath) {
  const [source, grammarRaw] = await Promise.all([
    fs.readFile(filePath, "utf-8"),
    fs.readFile(grammarPath, "utf-8"),
  ]);
  const grammar = JSON.parse(grammarRaw);
  const matchers = collectMatchers(grammar);
  const matches = [];
  const lines = source.split(/\r?\n/);
  lines.forEach((line, lineIndex) => {
    for (const matcher of matchers) {
      matcher.regex.lastIndex = 0;
      let match;
      while ((match = matcher.regex.exec(line)) !== null) {
        if (match[0].length === 0) {
          matcher.regex.lastIndex += 1;
          continue;
        }
        const indices = match.indices;
        matches.push({
          line: lineIndex,
          start: match.index,
          end: match.index + match[0].length,
          text: match[0],
          scope: matcher.name,
          source: matcher.repository,
        });
        if (indices && matcher.captures) {
          for (const [captureIndex, capture] of Object.entries(matcher.captures)) {
            const index = Number(captureIndex);
            const range = indices[index];
            if (!range || !capture.name || range[0] === range[1]) {
              continue;
            }
            matches.push({
              line: lineIndex,
              start: range[0],
              end: range[1],
              text: line.slice(range[0], range[1]),
              scope: capture.name,
              source: `${matcher.repository}:capture-${captureIndex}`,
            });
          }
        }
      }
    }
  });
  return matches.sort((left, right) =>
    left.line - right.line || left.start - right.start || left.end - right.end || left.scope.localeCompare(right.scope),
  );
}

function collectMatchers(grammar) {
  const matchers = [];
  const repository = grammar.repository ?? {};
  for (const [name, entry] of Object.entries(repository)) {
    collectPatternMatchers(entry.patterns ?? [], name, matchers);
  }
  return matchers;
}

function collectPatternMatchers(patterns, repositoryName, matchers) {
  for (const pattern of patterns) {
    if (pattern.match && pattern.name) {
      pushGrammarMatcher(matchers, repositoryName, pattern.name, pattern.match, pattern.captures);
    }
    if (pattern.begin && pattern.name) {
      pushGrammarMatcher(matchers, repositoryName, pattern.name, pattern.begin, pattern.beginCaptures);
    }
    if (pattern.end && pattern.name) {
      pushGrammarMatcher(matchers, repositoryName, pattern.name, pattern.end, pattern.endCaptures);
    }
    collectPatternMatchers(pattern.patterns ?? [], repositoryName, matchers);
  }
}

function pushGrammarMatcher(matchers, repositoryName, name, pattern, captures) {
  try {
    matchers.push({
      repository: repositoryName,
      name,
      captures,
      regex: new RegExp(pattern, "gd"),
    });
  } catch {
    // TextMate uses Oniguruma; skip patterns not accepted by JavaScript regex.
  }
}

async function runSmoke() {
  const inspection = await inspectSmokeFile(defaultFile, { rebuild: true });
  const query = await querySmokeFile(defaultFile, {
    symbol: defaultQueries[defaultFile],
    renameTo: "sage_debug_renamed",
  });
  const grammarScopes = new Set(inspection.grammarMatches.map((match) => match.scope));
  const semanticNames = new Set(inspection.semanticSpans.map((span) => `${span.token_type}:${span.line}:${span.start}`));
  const requiredScopes = [
    "entity.name.namespace.sagemath",
    "support.class.constructor.sagemath",
    "storage.type.decorator.cache.sagemath",
    "variable.parameter.preparse.generator.sagemath",
    "variable.parameter.keyword.sagemath",
    "support.function.method.sagemath",
  ];
  for (const scope of requiredScopes) {
    if (!grammarScopes.has(scope)) {
      throw new Error(`debug workbench smoke missing grammar scope: ${scope}`);
    }
  }
  for (const token of ["namespace", "type", "function", "decorator"]) {
    if (![...semanticNames].some((entry) => entry.startsWith(`${token}:`))) {
      throw new Error(`debug workbench smoke missing semantic token type: ${token}`);
    }
  }
  if (!inspection.indexStatus || inspection.indexStatus.indexed_file_count < 1) {
    throw new Error("debug workbench smoke expected indexed files");
  }
  if (!inspection.docsStatus?.runtime_worker_state) {
    throw new Error("debug workbench smoke expected docs worker state");
  }
  if (
    ["static-fallback", "disabled", "unavailable", "degraded"].includes(inspection.docsStatus.runtime_worker_state)
    && !inspection.docsStatus.runtime_degraded_reason
  ) {
    throw new Error("debug workbench smoke expected docs fallback/degraded reason");
  }
  if (!query.query?.hover?.markdown?.includes("PolynomialRing")) {
    throw new Error("debug workbench smoke expected hover markdown for PolynomialRing");
  }
  if (!query.query?.documentation?.summary) {
    throw new Error("debug workbench smoke expected documentation payload");
  }
  if (!Array.isArray(query.query?.completions)) {
    throw new Error("debug workbench smoke expected completion list");
  }
  const matrix = await runUxMatrix();
  if (matrix.summary.failed > 0) {
    const failed = matrix.rows
      .filter((row) => row.status !== "pass")
      .map((row) =>
        `${row.id}: ${row.checks
          .filter((check) => !check.pass)
          .map((check) => `${check.name} [${String(check.actual).slice(0, 200)}]`)
          .join(", ")}`,
      )
      .join("; ");
    throw new Error(`debug workbench UX matrix failed: ${failed}`);
  }
  await assertWarmQueryBudget("08_highlighting_structures.sage", "PolynomialRing", 1000);
  await assertWarmQueryBudget("08_highlighting_structures.sage", "A.degree", 1000);
  await assertWarmQueryBudget("08_highlighting_structures.sage", "M.rank", 1000);
  await assertWarmQueryBudget("09_advanced_sage_patterns.sage", "FunctionField", 1000);
  await assertWarmQueryBudget("09_advanced_sage_patterns.sage", "I.groebner_basis", 1000);
  await assertWarmPositionBudget("09_advanced_sage_patterns.sage", 69, 35, 1000);
  await assertWarmQueryBudget("06_runtime_graphs_and_number_theory.sage", "graphs.PetersenGraph", 1000);
  console.log("debug workbench smoke passed");
}

async function assertWarmQueryBudget(fileName, symbol, budgetMs) {
  const started = Date.now();
  await querySmokeFile(fileName, {
    symbol,
    renameTo: "sage_latency_probe",
    rebuild: false,
  });
  const elapsed = Date.now() - started;
  if (elapsed > budgetMs) {
    throw new Error(`query latency budget exceeded for ${symbol}: ${elapsed}ms > ${budgetMs}ms`);
  }
}

async function assertWarmPositionBudget(fileName, line, character, budgetMs) {
  const started = Date.now();
  await querySmokeFile(fileName, {
    line,
    character,
    renameTo: "sage_latency_probe",
    rebuild: false,
  });
  const elapsed = Date.now() - started;
  if (elapsed > budgetMs) {
    throw new Error(`position query latency budget exceeded for ${fileName}:${line}:${character}: ${elapsed}ms > ${budgetMs}ms`);
  }
}

function sendHtml(response, body) {
  response.writeHead(200, { "content-type": "text/html; charset=utf-8" });
  response.end(body);
}

function sendJson(response, body, status = 200) {
  response.writeHead(status, { "content-type": "application/json; charset=utf-8" });
  response.end(JSON.stringify(body, null, 2));
}

function sendText(response, status, body) {
  response.writeHead(status, { "content-type": "text/plain; charset=utf-8" });
  response.end(body);
}

function renderPage() {
  return `<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Sage Debug Workbench</title>
  <style>
    :root {
      color-scheme: light;
      --bg: #f7f8fb;
      --panel: #ffffff;
      --ink: #1d2430;
      --muted: #657084;
      --line: #d9dee8;
      --accent: #2367c7;
      --type: #7a3db8;
      --fn: #a84f11;
      --ns: #0b7285;
      --decorator: #9b1c55;
      --var: #276749;
    }
    * { box-sizing: border-box; }
    body {
      margin: 0;
      background: var(--bg);
      color: var(--ink);
      font: 14px/1.4 -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
    }
    header {
      display: flex;
      align-items: center;
      flex-wrap: wrap;
      gap: 12px;
      padding: 12px 16px;
      border-bottom: 1px solid var(--line);
      background: var(--panel);
      position: sticky;
      top: 0;
      z-index: 1;
    }
    h1 { flex: 0 0 88px; font-size: 16px; margin: 0; }
    select, button, input {
      height: 32px;
      border: 1px solid var(--line);
      background: white;
      color: var(--ink);
      border-radius: 6px;
      padding: 0 10px;
      font: inherit;
    }
    select { flex: 1 1 300px; min-width: 220px; max-width: 420px; }
    input { flex: 1 1 170px; min-width: 150px; }
    button { background: var(--accent); color: white; border-color: var(--accent); }
    button.secondary { background: white; color: var(--accent); }
    #loadMessage {
      flex: 1 1 100%;
      margin-left: 100px;
      color: var(--muted);
      font-size: 12px;
    }
    main {
      display: grid;
      grid-template-columns: minmax(460px, 1.1fr) minmax(420px, .9fr);
      gap: 12px;
      padding: 12px;
    }
    section {
      background: var(--panel);
      border: 1px solid var(--line);
      border-radius: 8px;
      min-width: 0;
      overflow: hidden;
    }
    section h2 {
      font-size: 13px;
      margin: 0;
      padding: 8px 10px;
      border-bottom: 1px solid var(--line);
      color: var(--muted);
      text-transform: uppercase;
      letter-spacing: 0;
    }
    pre {
      margin: 0;
      padding: 12px;
      overflow: auto;
      font: 13px/1.55 "SFMono-Regular", Menlo, Consolas, monospace;
      white-space: pre;
    }
    .line-number { display: inline-block; width: 34px; color: #8a94a6; user-select: none; }
    .tok-type { color: var(--type); font-weight: 600; }
    .tok-function { color: var(--fn); font-weight: 600; }
    .tok-namespace { color: var(--ns); font-weight: 600; }
    .tok-decorator { color: var(--decorator); font-weight: 600; }
    .tok-variable, .tok-parameter { color: var(--var); font-weight: 600; }
    .grid {
      display: grid;
      grid-template-columns: repeat(2, minmax(0, 1fr));
      gap: 12px;
    }
    table {
      width: 100%;
      border-collapse: collapse;
      font-size: 12px;
    }
    th, td {
      border-bottom: 1px solid var(--line);
      padding: 6px 8px;
      text-align: left;
      vertical-align: top;
      word-break: break-word;
    }
    th { color: var(--muted); font-weight: 600; background: #fbfcfe; position: sticky; top: 0; }
    .panel-body { max-height: 44vh; overflow: auto; }
    .wide { grid-column: 1 / -1; }
    .status {
      display: grid;
      grid-template-columns: repeat(4, minmax(0, 1fr));
      gap: 8px;
      padding: 10px;
    }
    .metric {
      border: 1px solid var(--line);
      border-radius: 6px;
      padding: 8px;
      min-width: 0;
    }
    .metric b { display: block; font-size: 11px; color: var(--muted); }
    .note {
      margin: 0;
      padding: 8px 10px;
      color: var(--muted);
      border-bottom: 1px solid var(--line);
      font-size: 12px;
    }
    .query-toolbar {
      display: flex;
      align-items: center;
      gap: 8px;
      flex-wrap: wrap;
      padding: 10px;
      border-bottom: 1px solid var(--line);
    }
    .query-grid {
      display: grid;
      grid-template-columns: repeat(3, minmax(0, 1fr));
      gap: 12px;
      padding: 10px;
    }
    .query-grid h3 {
      margin: 0 0 6px;
      font-size: 12px;
      color: var(--muted);
      text-transform: uppercase;
      letter-spacing: 0;
    }
    .markdown {
      min-height: 82px;
      max-height: 220px;
      overflow: auto;
      border: 1px solid var(--line);
      border-radius: 6px;
      padding: 8px;
      white-space: pre-wrap;
      font: 12px/1.45 "SFMono-Regular", Menlo, Consolas, monospace;
      background: #fbfcfe;
    }
    .pass { color: #067647; font-weight: 600; }
    .fail { color: #b42318; font-weight: 600; }
    .matrix-summary {
      padding: 10px;
      border-bottom: 1px solid var(--line);
      color: var(--muted);
    }
    .error { color: #b42318; padding: 12px; }
    @media (max-width: 980px) {
      main { grid-template-columns: 1fr; }
      .status { grid-template-columns: repeat(2, minmax(0, 1fr)); }
      .query-grid { grid-template-columns: 1fr; }
    }
    @media (max-width: 720px) {
      h1 { flex-basis: 100%; }
      #loadMessage { margin-left: 0; }
    }
  </style>
</head>
<body>
  <header>
    <h1>Sage Debug Workbench</h1>
    <select id="fileSelect" aria-label="Smoke file"></select>
    <button id="refreshButton" type="button">Refresh</button>
    <input id="symbolInput" aria-label="Query symbol" placeholder="symbol or dotted.name">
    <input id="renameInput" aria-label="Rename preview target" value="sage_debug_renamed">
    <button id="queryButton" type="button">Run Query</button>
    <button id="uxButton" class="secondary" type="button">Run UX Matrix</button>
    <span id="message" aria-live="polite"></span>
  </header>
  <main>
    <section>
      <h2>Semantic Preview</h2>
      <pre id="sourceView"></pre>
    </section>
    <section>
      <h2>Index / Docs Status</h2>
      <div id="statusView" class="status"></div>
    </section>
    <section class="wide">
      <h2>UX Defect Matrix</h2>
      <div id="uxSummary" class="matrix-summary">Not run yet.</div>
      <div class="panel-body"><table id="uxTable"></table></div>
    </section>
    <section class="wide">
      <h2>LSP-like Query</h2>
      <div class="query-toolbar">
        <span id="querySummary"></span>
      </div>
      <div class="query-grid">
        <div>
          <h3>Hover Markdown</h3>
          <div id="hoverView" class="markdown"></div>
        </div>
        <div>
          <h3>Documentation</h3>
          <table id="documentationTable"></table>
        </div>
        <div>
          <h3>Signature</h3>
          <table id="signatureTable"></table>
        </div>
        <div>
          <h3>Definition</h3>
          <table id="definitionTable"></table>
        </div>
        <div>
          <h3>Resolution</h3>
          <table id="resolutionTable"></table>
        </div>
        <div>
          <h3>Completions</h3>
          <table id="completionTable"></table>
        </div>
        <div>
          <h3>References</h3>
          <table id="referenceTable"></table>
        </div>
        <div>
          <h3>Rename Preview</h3>
          <table id="renameTable"></table>
        </div>
      </div>
    </section>
    <section>
      <h2>TextMate Scope Matches</h2>
      <p class="note">Actual grammar approximation: this table scans TextMate regexes with JavaScript regex. VS Code uses Oniguruma, so this is a debug aid, not the final token stream.</p>
      <div class="panel-body"><table id="scopeTable"></table></div>
    </section>
    <section>
      <h2>Rust Semantic Tokens</h2>
      <div class="panel-body"><table id="semanticTable"></table></div>
    </section>
    <section>
      <h2>Symbols</h2>
      <div class="panel-body"><table id="symbolTable"></table></div>
    </section>
    <section>
      <h2>Diagnostics / Preprocess</h2>
      <div class="panel-body"><table id="diagnosticTable"></table></div>
    </section>
  </main>
  <script>
    const fileSelect = document.querySelector("#fileSelect");
    const refreshButton = document.querySelector("#refreshButton");
    const queryButton = document.querySelector("#queryButton");
    const uxButton = document.querySelector("#uxButton");
    const symbolInput = document.querySelector("#symbolInput");
    const renameInput = document.querySelector("#renameInput");
    const message = document.querySelector("#message");
    const sourceView = document.querySelector("#sourceView");
    const defaultQueries = ${JSON.stringify(defaultQueries)};

    init().catch(showError);

    async function init() {
      const files = await fetchJson("/api/files");
      for (const file of files.files) {
        const option = document.createElement("option");
        option.value = file;
        option.textContent = file;
        fileSelect.append(option);
      }
      fileSelect.value = files.defaultFile;
      fileSelect.addEventListener("change", () => {
        symbolInput.value = defaultQueries[fileSelect.value] ?? "";
        loadInspection();
      });
      refreshButton.addEventListener("click", () => loadInspection(true));
      queryButton.addEventListener("click", () => loadQuery());
      uxButton.addEventListener("click", () => loadUxMatrix());
      symbolInput.addEventListener("keydown", (event) => {
        if (event.key === "Enter") loadQuery();
      });
      symbolInput.value = defaultQueries[fileSelect.value] ?? "";
      await loadInspection(false);
      document.querySelector("#uxSummary").textContent = "Not run yet";
    }

    async function loadInspection(rebuild = false) {
      message.textContent = "Loading...";
      const payload = await fetchJson("/api/inspect?file=" + encodeURIComponent(fileSelect.value) + (rebuild ? "&rebuild=1" : ""));
      if (payload.error) throw new Error(payload.error);
      renderSource(payload.source, payload.semanticSpans);
      renderStatus(payload);
      renderTable("#scopeTable", ["line", "text", "scope", "source"], payload.grammarMatches.slice(0, 240));
      renderTable("#semanticTable", ["line", "start", "length", "token_type", "modifiers"], payload.semanticSpans);
      renderTable("#symbolTable", ["name", "kind", "detail", "module"], payload.parsed.symbols);
      renderTable("#diagnosticTable", ["message", "code", "range"], [
        ...payload.diagnostics,
        ...(payload.preprocess?.edits ?? []).map((edit) => ({
          message: "preprocess " + edit.source_text,
          code: edit.generated_text,
          range: edit.line + ":" + edit.source_character + " -> " + edit.generated_character,
        })),
      ]);
      await loadQuery();
      message.textContent = "Loaded " + payload.file;
    }

    async function loadQuery() {
      const started = performance.now();
      const params = new URLSearchParams({
        file: fileSelect.value,
        symbol: symbolInput.value || defaultQueries[fileSelect.value] || "",
        renameTo: renameInput.value || "sage_debug_renamed",
      });
      const payload = await fetchJson("/api/query?" + params.toString());
      if (payload.error) throw new Error(payload.error);
      payload.client_ms = Math.round(performance.now() - started);
      renderQuery(payload);
    }

    async function loadUxMatrix() {
      document.querySelector("#uxSummary").textContent = "Running UX matrix...";
      const payload = await fetchJson("/api/ux");
      if (payload.error) throw new Error(payload.error);
      renderUxMatrix(payload);
    }

    function renderUxMatrix(payload) {
      const summary = payload.summary ?? {};
      document.querySelector("#uxSummary").innerHTML =
        '<span class="' + (summary.failed ? "fail" : "pass") + '">' +
        escapeHtml(String(summary.passed ?? 0)) + "/" + escapeHtml(String(summary.total ?? 0)) +
        " passing</span>" +
        (summary.failed ? "  failed=" + escapeHtml(String(summary.failed)) : "");
      renderTable("#uxTable", ["status", "title", "file", "target", "checks"], (payload.rows ?? []).map((row) => ({
        status: row.status === "pass" ? "PASS" : "FAIL",
        title: row.title,
        file: row.file,
        target: row.target,
        checks: row.checks.map((check) => (check.pass ? "OK " : "FAIL ") + check.name + (check.pass ? "" : " -> " + check.actual)).join("\\n"),
      })));
      for (const cell of document.querySelectorAll("#uxTable td:first-child")) {
        cell.className = cell.textContent === "PASS" ? "pass" : "fail";
      }
    }

    function renderQuery(payload) {
      const query = payload.query ?? {};
      const target = query.target ?? {};
      document.querySelector("#querySummary").textContent = [
        "target=" + (target.dotted_symbol || target.symbol || payload.requestedSymbol || "n/a"),
        "owner=" + (query.ownerType || "n/a"),
        "confidence=" + (query.resolutionConfidence || "n/a"),
        "fallback=" + (query.fallback_reason || "none"),
        "query ms=" + (payload.client_ms ?? payload.timing?.server_ms ?? "n/a"),
        "cache=" + (payload.timing?.cache_hit ? "hit" : "miss"),
        "references=" + (query.references?.length ?? 0),
        "rename edits=" + (query.rename_preview?.length ?? 0),
      ].join("  ");
      document.querySelector("#hoverView").textContent = query.hover?.markdown ?? "";
      renderTable("#documentationTable", ["name", "module_name", "kind", "summary", "uri"], query.documentation ? [query.documentation] : []);
      renderTable("#signatureTable", ["label", "active_parameter", "documentation"], query.signature ? [query.signature] : []);
      renderTable("#definitionTable", ["name", "module", "detail", "path", "range"], query.definition ? [query.definition] : []);
      renderTable("#resolutionTable", ["ownerType", "resolutionConfidence", "resolutionReason", "candidateCount", "fallback_reason"], [{
        ownerType: query.ownerType,
        resolutionConfidence: query.resolutionConfidence,
        resolutionReason: query.resolutionReason,
        candidateCount: query.candidateCount,
        fallback_reason: query.fallback_reason,
      }]);
      renderTable("#completionTable", ["label", "kind", "detail", "signature", "documentation"], (query.completions ?? []).slice(0, 80));
      renderTable("#referenceTable", ["path", "range"], (query.references ?? []).slice(0, 120));
      renderTable("#renameTable", ["path", "range", "new_text"], (query.rename_preview ?? []).slice(0, 120));
    }

    function renderSource(source, spans) {
      const byLine = new Map();
      for (const span of spans) {
        if (!byLine.has(span.line)) byLine.set(span.line, []);
        byLine.get(span.line).push(span);
      }
      sourceView.innerHTML = source.split(/\\r?\\n/).map((line, lineIndex) => {
        const spansForLine = (byLine.get(lineIndex) ?? []).slice().sort((a, b) => a.start - b.start || b.length - a.length);
        let cursor = 0;
        let rendered = "";
        for (const span of spansForLine) {
          if (span.start < cursor) continue;
          rendered += escapeHtml(line.slice(cursor, span.start));
          const tokenText = line.slice(span.start, span.start + span.length);
          rendered += '<span class="tok-' + escapeAttr(span.token_type) + '" title="' + escapeAttr(span.token_type + " " + span.modifiers.join(",")) + '">' + escapeHtml(tokenText) + '</span>';
          cursor = span.start + span.length;
        }
        rendered += escapeHtml(line.slice(cursor));
        return '<span class="line-number">' + String(lineIndex + 1).padStart(2, " ") + '</span>' + rendered;
      }).join("\\n");
    }

    function renderStatus(payload) {
      const status = payload.indexStatus ?? {};
      const docs = payload.docsStatus ?? {};
      const entries = [
        ["file", payload.file],
        ["indexed files", status.indexed_file_count],
        ["symbols", status.symbol_count],
        ["docs", status.doc_count],
        ["generation", status.generation],
        ["cache namespace", status.cache_namespace],
        ["cache stale", status.cache_stale],
        ["root fingerprints", formatRootFingerprints(status.source_root_fingerprints)],
        ["stale roots", formatStaleRoots(status.stale_source_roots)],
        ["operation", status.last_operation],
        ["cache hits", status.cache_hit_count],
        ["cache misses", status.cache_miss_count],
        ["hot cache", status.hot_symbol_cache_count],
        ["last index ms", status.last_index_ms],
        ["hydrate ms", status.last_hydrate_ms],
        ["reconcile ms", status.last_reconcile_ms],
        ["persist ms", status.last_persist_ms],
        ["hot cache ms", status.last_hot_cache_ms],
        ["inspect ms", payload.timing?.server_ms],
        ["worker", docs.runtime_worker_state],
        ["docs db", docs.doc_db_path],
        ["offline docs", docs.offline_doc_count],
        ["worker degraded", docs.runtime_degraded_reason],
        ["worker queue", docs.runtime_queue_depth],
        ["worker timeouts", docs.runtime_timeout_count],
        ["worker cache hits", docs.runtime_cache_hits],
        ["worker cache misses", docs.runtime_cache_misses],
      ];
      document.querySelector("#statusView").innerHTML = entries.map(([key, value]) =>
        '<div class="metric"><b>' + escapeHtml(key) + '</b>' + escapeHtml(String(value ?? "n/a")) + '</div>'
      ).join("");
    }

    function formatRootFingerprints(fingerprints) {
      if (!Array.isArray(fingerprints) || fingerprints.length === 0) return "n/a";
      return fingerprints.map((entry) => {
        const segments = String(entry.root ?? "").split(/[\\\\/]+/).filter(Boolean);
        const label = segments.length >= 2 ? segments.slice(-2).join("/") : (segments[0] || "root");
        return label + ":" + (entry.digest ?? "unknown") + (entry.exists === false ? ":missing" : "");
      }).join(", ");
    }

    function formatStaleRoots(staleRoots) {
      if (!Array.isArray(staleRoots) || staleRoots.length === 0) return "none";
      return staleRoots.map((entry) => {
        const segments = String(entry.root ?? "").split(/[\\\\/]+/).filter(Boolean);
        const label = segments.length >= 2 ? segments.slice(-2).join("/") : (segments[0] || "root");
        return label + ":" + (entry.cached_digest ?? "unknown") + "->" + (entry.current_digest ?? "unknown");
      }).join(", ");
    }

    function renderTable(selector, columns, rows) {
      document.querySelector(selector).innerHTML = '<thead><tr>' + columns.map((column) => '<th>' + escapeHtml(column) + '</th>').join("") + '</tr></thead><tbody>' +
        rows.map((row) => '<tr>' + columns.map((column) => '<td>' + escapeHtml(formatCell(row[column])) + '</td>').join("") + '</tr>').join("") + '</tbody>';
    }

    function formatCell(value) {
      if (value == null) return "";
      if (Array.isArray(value)) return value.join(", ");
      if (typeof value === "object") return JSON.stringify(value);
      return String(value);
    }

    async function fetchJson(url) {
      const response = await fetch(url);
      if (!response.ok) throw new Error(await response.text());
      return response.json();
    }

    function showError(error) {
      message.textContent = "";
      document.body.insertAdjacentHTML("beforeend", '<div class="error">' + escapeHtml(String(error.message ?? error)) + '</div>');
    }

    function escapeHtml(value) {
      return String(value).replace(/[&<>"']/g, (char) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[char]));
    }
    function escapeAttr(value) { return escapeHtml(value); }
  </script>
</body>
</html>`;
}
