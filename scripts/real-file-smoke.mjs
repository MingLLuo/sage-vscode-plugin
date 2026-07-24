#!/usr/bin/env node
import { execFile } from "node:child_process";
import fsSync from "node:fs";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { promisify } from "node:util";

const execFileAsync = promisify(execFile);
const __dirname = path.dirname(fileURLToPath(import.meta.url));
const repositoryRoot = path.resolve(__dirname, "..");
const debugInspector = path.join(
  repositoryRoot,
  "target",
  "debug",
  process.platform === "win32" ? "sage-debug-inspect.exe" : "sage-debug-inspect",
);
const publicFixtureCandidates = [
  path.join(repositoryRoot, "examples", "manual-smoke-workspace", "src", "10_sage_heavy_python.py"),
  path.join(repositoryRoot, "examples", "manual-smoke-workspace", "src", "11_sage_object_methods.py"),
];
const configuredRealFileCandidates = configuredRealFilePaths();
const selectedRealFileCandidates = configuredRealFileCandidates.length > 0
  ? configuredRealFileCandidates
  : publicFixtureCandidates;
const missingRealFileCandidates = selectedRealFileCandidates.filter((candidate) => !fsSync.existsSync(candidate));
const realFiles = selectedRealFileCandidates.map((candidate) => path.resolve(candidate));
const sageSourceRoot = process.env.SAGE_SOURCE_ROOT
  ? path.resolve(process.env.SAGE_SOURCE_ROOT)
  : path.resolve(repositoryRoot, "..", "sage", "src");
const cacheHome = process.env.SAGE_REAL_FILE_SMOKE_CACHE_DIR
  ? path.resolve(process.env.SAGE_REAL_FILE_SMOKE_CACHE_DIR)
  : path.join(os.tmpdir(), "sage-vscode-real-file-smoke-cache");

if (missingRealFileCandidates.length > 0) {
  console.log(JSON.stringify({
    status: "failed",
    reason: configuredRealFileCandidates.length > 0
      ? "missing explicitly configured real-file smoke paths"
      : "missing checked-in public real-file smoke fixture",
    missingFiles: missingRealFileCandidates.map((candidate) => path.resolve(candidate)),
    configured: configuredRealFileCandidates.length > 0,
  }, null, 2));
  process.exit(1);
}
if (!fsSync.existsSync(path.join(sageSourceRoot, "sage"))) {
  console.log(JSON.stringify({ status: "skipped", reason: `missing Sage source root: ${sageSourceRoot}` }, null, 2));
  process.exit(0);
}
if (!fsSync.existsSync(debugInspector)) {
  console.error(`Missing debug inspector: ${debugInspector}. Run npm run build:debug-inspector first.`);
  process.exit(1);
}

const warmedRootKeys = new Set();
const fileResults = [];
for (const file of realFiles) {
  fileResults.push(await runFileSmoke(file, warmedRootKeys));
}

const allRows = fileResults.flatMap((result) => result.rows);
const sortedTimings = allRows.map((row) => row.elapsedMs).sort((left, right) => left - right);
const p95Timing = sortedTimings.length
  ? sortedTimings[Math.min(sortedTimings.length - 1, Math.ceil(sortedTimings.length * 0.95) - 1)]
  : 0;
const summaryChecks = [
  {
    name: "query p95 <= 150ms",
    pass: p95Timing <= 150,
    actual: p95Timing,
  },
  {
    name: "all queries <= 250ms",
    pass: allRows.every((row) => row.elapsedMs <= 250),
    actual: sortedTimings.at(-1) ?? 0,
  },
  {
    name: "all selected real files covered",
    pass: fileResults.length === realFiles.length && fileResults.every((result) => result.rows.length > 0),
    actual: fileResults.map((result) => ({ file: result.file, rows: result.rows.length })),
  },
];
const failed = allRows.filter((row) => row.status !== "pass");
const failedSummary = summaryChecks.filter((check) => !check.pass);
const report = {
  status: failed.length || failedSummary.length ? "failed" : "passed",
  files: realFiles,
  summaryChecks,
  fileResults,
};
await writeJsonReport(report);
process.exitCode = failed.length || failedSummary.length ? 1 : 0;

async function writeJsonReport(payload) {
  await new Promise((resolve, reject) => {
    process.stdout.write(`${JSON.stringify(payload, null, 2)}\n`, (error) => {
      if (error) {
        reject(error);
        return;
      }
      resolve();
    });
  });
}

async function runFileSmoke(realFile, warmedRootKeys) {
  const source = await fs.readFile(realFile, "utf8");
  const workspaceRoot = path.dirname(realFile);
  const scenarios = buildScenarios(realFile, source);
  await ensureWarmIndexCache(realFile, workspaceRoot, warmedRootKeys);
  const payload = await inspectScenarios(realFile, workspaceRoot, source, scenarios);
  const queryResults = new Map((payload.batchQueries ?? []).map((result) => [result.id, result]));

  const rows = [];
  for (const scenario of scenarios) {
    const result = queryResults.get(scenario.id) ?? {
      timing_ms: 0,
      query: {
        fallback_reason: "missing-batch-query-result",
      },
    };
    const elapsedMs = Number(result.timing_ms ?? 0);
    const scenarioPayload = { ...payload, query: result.query };
    const checks = evaluateScenario(scenario, scenarioPayload, elapsedMs);
    rows.push({
      id: scenario.id,
      status: checks.every((check) => check.pass) ? "pass" : "fail",
      elapsedMs,
      target: scenario.symbol ?? `.${scenario.member}`,
      definition: scenarioPayload.query?.definition?.path ?? null,
      ownerType: scenarioPayload.query?.ownerType ?? null,
      confidence: scenarioPayload.query?.resolutionConfidence ?? null,
      checks,
    });
  }

  return {
    status: rows.every((row) => row.status === "pass") ? "passed" : "failed",
    file: realFile,
    indexStatus: payload.indexStatus,
    rows,
  };
}

function buildScenarios(filePath, text) {
  const fileName = path.basename(filePath);
  if (fileName === "10_sage_heavy_python.py") {
    return [
      coldSymbol("PolynomialRing", "sage/src/sage/rings/polynomial/polynomial_ring_constructor.py"),
      symbolScenario("sage-all-gf", "GF", "sage/src/sage/rings/finite_rings/finite_field_constructor.py"),
      symbolScenario("sage-all-matrix", "matrix", "sage/src/sage/matrix/constructor.pyx"),
      symbolScenario("sage-all-vector", "vector", "sage/src/sage/modules/free_module_element.pyx"),
      symbolScenario("sage-all-zero-matrix", "zero_matrix", "sage/src/sage/matrix/special.py"),
      symbolScenario("sage-all-zero-vector", "zero_vector", "sage/src/sage/modules/free_module_element.pyx"),
      {
        id: "local-top-level-driver",
        symbol: "solve_demo_system",
        maxMs: 250,
        expects: { definitionPathIncludes: fileName, definitionName: "solve_demo_system" },
      },
      matrixMethod("rank", "sage/src/sage/matrix/matrix0.pyx"),
      ambiguousMethod("matrix-solve-right", "solve_right", "sage/src/sage/matrix/matrix2.pyx", 0, 1),
      ambiguousMethod("matrix-right-kernel", "right_kernel", "sage/src/sage/matrix/matrix2.pyx", 0, 1),
      ambiguousMethod(
        "polynomial-derivative",
        "derivative",
        "sage/src/sage/rings/polynomial/multi_polynomial.pyx",
        1,
      ),
      ambiguousMethod(
        "polynomial-ring-ideal",
        "ideal",
        "sage/src/sage/rings/polynomial/multi_polynomial_libsingular.pyx",
      ),
      ambiguousMethod(
        "polynomial-ring-base-ring",
        "base_ring",
        "sage/src/sage/structure/category_object.pyx",
      ),
      ambiguousMethod(
        "ideal-variety",
        "variety",
        "sage/src/sage/rings/polynomial/multi_polynomial_ideal.py",
      ),
      ambiguousMethod(
        "polynomial-resultant",
        "resultant",
        "sage/src/sage/rings/polynomial/multi_polynomial_element.py",
      ),
      ambiguousMethod("polynomial-gcd", "gcd", "sage/src/sage/rings/polynomial/multi_polynomial.pyx"),
      matrixMethod("det", "sage/src/sage/matrix/matrix2.pyx"),
      ambiguousMethod("matrix-rows", "rows", "sage/src/sage/matrix/matrix1.pyx"),
      ambiguousMethod("matrix-transpose", "transpose", "sage/src/sage/matrix/matrix_dense.pyx"),
    ].filter((scenario) => scenarioExists(text, scenario));
  }
  if (fileName === "11_sage_object_methods.py") {
    return [
      symbolScenario("sage-all-graph", "Graph", "sage/src/sage/graphs/graph.py", "Graph", "high"),
      symbolScenario("sage-all-digraph", "DiGraph", "sage/src/sage/graphs/digraph.py", "DiGraph", "high"),
      symbolScenario(
        "sage-all-elliptic-curve",
        "EllipticCurve",
        "sage/src/sage/schemes/elliptic_curves/constructor.py",
        "EllipticCurve",
        "high",
      ),
      symbolScenario(
        "sage-all-number-field",
        "NumberField",
        "sage/src/sage/rings/number_field/number_field.py",
        "NumberField",
        "high",
      ),
      graphMethod("vertices", "sage/src/sage/graphs/generic_graph.py"),
      graphMethod("neighbors", "sage/src/sage/graphs/generic_graph.py"),
      graphMethod("edges", "sage/src/sage/graphs/generic_graph.py"),
      graphMethod("degree", "sage/src/sage/graphs/generic_graph.py"),
      graphMethod("is_connected", "sage/src/sage/graphs/base/c_graph.pyx"),
      graphMethod("shortest_path", "sage/src/sage/graphs/generic_graph.py"),
      graphMethod("adjacency_matrix", "sage/src/sage/graphs/generic_graph.py"),
      graphMethod("plot", "sage/src/sage/graphs/generic_graph.py"),
      ellipticCurveMethod("base_ring", "sage/src/sage/schemes/elliptic_curves/ell_generic.py"),
      ellipticCurveMethod("points", "sage/src/sage/schemes/elliptic_curves/ell_finite_field.py"),
      ellipticCurveMethod("cardinality", "sage/src/sage/schemes/elliptic_curves/ell_finite_field.py"),
      ellipticCurveMethod("order", "sage/src/sage/schemes/elliptic_curves/ell_finite_field.py"),
      ellipticCurveMethod("torsion_subgroup", "sage/src/sage/schemes/elliptic_curves/ell_finite_field.py"),
      ellipticCurveMethod("rank", "sage/src/sage/schemes/elliptic_curves/ell_rational_field.py"),
      ellipticCurveMethod("gens", "sage/src/sage/schemes/elliptic_curves/ell_generic.py"),
      ellipticCurveMethod("integral_points", "sage/src/sage/schemes/elliptic_curves/ell_rational_field.py"),
      ellipticCurveMethod("plot", "sage/src/sage/schemes/elliptic_curves/ell_generic.py", 1),
      numberFieldMethod("gen", "sage/src/sage/rings/number_field/number_field.py", 1),
      numberFieldMethod("gens", "sage/src/sage/rings/number_field/number_field_rel.py", 1),
      numberFieldMethod("degree", "sage/src/sage/rings/number_field/number_field.py", 1),
      numberFieldMethod("absolute_degree", "sage/src/sage/rings/number_field/number_field.py"),
      numberFieldMethod("relative_degree", "sage/src/sage/rings/number_field/number_field.py"),
      numberFieldMethod("discriminant", "sage/src/sage/rings/number_field/number_field.py"),
      numberFieldMethod("signature", "sage/src/sage/rings/number_field/number_field.py"),
      numberFieldMethod("ring_of_integers", "sage/src/sage/rings/number_field/number_field_base.pyx"),
      numberFieldMethod("embeddings", "sage/src/sage/rings/number_field/number_field.py"),
      numberFieldMethod("places", "sage/src/sage/rings/number_field/number_field.py"),
      numberFieldMethod("class_group", "sage/src/sage/rings/number_field/number_field.py"),
      numberFieldMethod("unit_group", "sage/src/sage/rings/number_field/number_field.py"),
    ].filter((scenario) => scenarioExists(text, scenario));
  }
  return [
    coldSymbol("PolynomialRing", "sage/src/sage/rings/polynomial/polynomial_ring_constructor.py"),
    symbolScenario("sage-all-matrix", "matrix", "sage/src/sage/matrix/constructor.pyx"),
    symbolScenario("sage-all-vector", "vector", "sage/src/sage/modules/free_module_element.pyx"),
    symbolScenario("sage-all-zero-matrix", "zero_matrix", "sage/src/sage/matrix/special.py"),
    matrixMethod("rank", "sage/src/sage/matrix/matrix0.pyx"),
    matrixMethod("base_ring", "sage/src/sage/matrix/matrix0.pyx"),
  ].filter((scenario) => scenarioExists(text, scenario));
}

function coldSymbol(symbol, definitionPathIncludes, resolutionConfidence) {
  return {
    id: "diagnostics-and-polynomial-ring",
    symbol,
    maxMs: 250,
    expects: compactExpectations({
      noDiagnostics: true,
      definitionPathIncludes,
      resolutionConfidence,
    }),
  };
}

function symbolScenario(id, symbol, definitionPathIncludes, definitionName = symbol, resolutionConfidence) {
  return {
    id,
    symbol,
    maxMs: 250,
    expects: compactExpectations({ definitionPathIncludes, definitionName, resolutionConfidence }),
  };
}

function matrixMethod(member, definitionPathIncludes, occurrence = 0) {
  return {
    id: `matrix-${member.replaceAll("_", "-")}`,
    member,
    occurrence,
    maxMs: 250,
    expects: {
      definitionPathIncludes,
      ownerType: "Matrix",
      resolutionConfidence: "high",
    },
  };
}

function ambiguousMethod(
  id,
  member,
  definitionCandidatePathIncludes,
  occurrence = 0,
  minDefinitionCandidates = 2,
) {
  return {
    id,
    member,
    occurrence,
    maxMs: 250,
    expects: {
      noDefinition: true,
      minDefinitionCandidates,
      definitionCandidatePathIncludes,
      resolutionConfidence: "ambiguous",
      allowFallbackReason: true,
    },
  };
}

function polynomialMethod(member, definitionPathIncludes, occurrence = 0) {
  return {
    id: `polynomial-${member.replaceAll("_", "-")}`,
    member,
    occurrence,
    maxMs: 250,
    expects: {
      definitionPathIncludes,
      ownerType: "PolynomialElement",
      resolutionConfidence: "high",
    },
  };
}

function vectorMethod(member, definitionPathIncludes, occurrence = 0) {
  return ownedMethod("vector", "Vector", member, definitionPathIncludes, occurrence);
}

function freeModuleMethod(member, definitionPathIncludes, occurrence = 0) {
  return ownedMethod("free-module", "FreeModule", member, definitionPathIncludes, occurrence);
}

function fieldMethod(member, definitionPathIncludes, occurrence = 0) {
  return ownedMethod("field", "Field", member, definitionPathIncludes, occurrence);
}

function graphMethod(member, definitionPathIncludes, occurrence = 0) {
  return ownedMethod("graph", "Graph", member, definitionPathIncludes, occurrence);
}

function ellipticCurveMethod(member, definitionPathIncludes, occurrence = 0) {
  return ownedMethod("elliptic-curve", "EllipticCurve", member, definitionPathIncludes, occurrence);
}

function numberFieldMethod(member, definitionPathIncludes, occurrence = 0) {
  return ownedMethod("number-field", "NumberField", member, definitionPathIncludes, occurrence);
}

function ownedMethod(idPrefix, ownerType, member, definitionPathIncludes, occurrence = 0) {
  return {
    id: `${idPrefix}-${member.replaceAll("_", "-")}`,
    member,
    occurrence,
    maxMs: 250,
    expects: {
      definitionPathIncludes,
      ownerType,
      resolutionConfidence: "high",
    },
  };
}

function scenarioExists(text, scenario) {
  if (scenario.symbol) {
    return text.includes(scenario.symbol);
  }
  return hasMember(text, scenario.member, scenario.occurrence ?? 0);
}

function compactExpectations(value) {
  return Object.fromEntries(Object.entries(value).filter(([, entry]) => entry !== undefined));
}

async function ensureWarmIndexCache(realFile, workspaceRoot, warmedRootKeys) {
  const rootKey = JSON.stringify([workspaceRoot, sageSourceRoot]);
  if (warmedRootKeys.has(rootKey)) {
    return;
  }
  warmedRootKeys.add(rootKey);
  const args = [...baseArgsForFile(realFile, workspaceRoot), "--symbol", "PolynomialRing", "--rebuild-index"];
  await execFileAsync(debugInspector, args, {
    cwd: repositoryRoot,
    env: { ...process.env, XDG_CACHE_HOME: cacheHome },
    maxBuffer: 30 * 1024 * 1024,
  });
}

async function inspectScenarios(realFile, workspaceRoot, source, scenarios) {
  await fs.mkdir(cacheHome, { recursive: true });
  const batchPath = path.join(cacheHome, `real-file-smoke-${process.pid}.json`);
  const measuredBatch = scenarios.map((scenario) => batchItemForScenario(source, scenario));
  const warmupBatch = measuredBatch.map((item) => ({
    ...item,
    id: `__warmup__${item.id}`,
  }));
  const batch = [...warmupBatch, ...measuredBatch];
  await fs.writeFile(batchPath, JSON.stringify(batch), "utf8");
  const args = [...baseArgsForFile(realFile, workspaceRoot), "--batch-file", batchPath];
  const { stdout } = await execFileAsync(debugInspector, args, {
    cwd: repositoryRoot,
    env: { ...process.env, XDG_CACHE_HOME: cacheHome },
    maxBuffer: 30 * 1024 * 1024,
  });
  return JSON.parse(stdout);
}

function batchItemForScenario(source, scenario) {
  if (scenario.member) {
    const position = memberPosition(source, scenario.member, scenario.occurrence ?? 0);
    return {
      id: scenario.id,
      line: position.line,
      character: position.character,
      rename_to: "sage_real_file_smoke",
    };
  }
  return {
    id: scenario.id,
    symbol: scenario.symbol,
    rename_to: "sage_real_file_smoke",
  };
}

function baseArgsForFile(realFile, workspaceRoot) {
  return [
    "--root",
    workspaceRoot,
    "--root",
    sageSourceRoot,
    "--editable-root",
    workspaceRoot,
    "--file",
    realFile,
  ];
}

function memberPosition(text, member, occurrence) {
  const needle = `.${member}`;
  let seen = 0;
  for (const [lineIndex, line] of text.split(/\r?\n/).entries()) {
    let start = line.indexOf(needle);
    while (start >= 0) {
      const after = line[start + needle.length] ?? "";
      const isWholeMember = !isIdentifierChar(after);
      if (isWholeMember && seen === occurrence) {
        return { line: lineIndex, character: start + 1 };
      }
      if (isWholeMember) {
        seen += 1;
      }
      start = line.indexOf(needle, start + needle.length);
    }
  }
  throw new Error(`missing member ${needle}`);
}

function hasMember(text, member, occurrence) {
  try {
    memberPosition(text, member, occurrence);
    return true;
  } catch {
    return false;
  }
}

function isIdentifierChar(value) {
  return /^[A-Za-z0-9_]$/.test(value);
}

function evaluateScenario(scenario, payload, elapsedMs) {
  const query = payload.query ?? {};
  const checks = [];
  const expects = scenario.expects ?? {};
  const definitionCandidates = query.definitionCandidates ?? [];
  if (expects.noDiagnostics) {
    checks.push({ name: "no diagnostics", pass: (payload.diagnostics ?? []).length === 0, actual: payload.diagnostics ?? [] });
  }
  if (expects.definitionPathIncludes) {
    const definitionPath = normalizePath(query.definition?.path ?? "");
    checks.push({
      name: `definition path includes ${expects.definitionPathIncludes}`,
      pass: definitionPath.includes(expects.definitionPathIncludes),
      actual: query.definition?.path ?? "none",
    });
  }
  if (expects.definitionName) {
    checks.push({
      name: `definition name ${expects.definitionName}`,
      pass: query.definition?.name === expects.definitionName,
      actual: query.definition?.name ?? "none",
    });
  }
  if (expects.noDefinition) {
    checks.push({
      name: "no forced single definition",
      pass: !query.definition,
      actual: query.definition?.path ?? "none",
    });
  }
  if (expects.minDefinitionCandidates !== undefined) {
    checks.push({
      name: `definition candidates >= ${expects.minDefinitionCandidates}`,
      pass: definitionCandidates.length >= expects.minDefinitionCandidates,
      actual: definitionCandidates.length,
    });
  }
  if (expects.definitionCandidatePathIncludes) {
    const candidatePaths = definitionCandidates.map((candidate) => candidate?.definition?.path ?? "");
    checks.push({
      name: `definition candidates include ${expects.definitionCandidatePathIncludes}`,
      pass: candidatePaths.some((candidatePath) =>
        normalizePath(candidatePath).includes(expects.definitionCandidatePathIncludes)
      ),
      actual: candidatePaths.length > 0 ? candidatePaths : ["none"],
    });
  }
  if (expects.ownerType) {
    checks.push({ name: `owner type ${expects.ownerType}`, pass: query.ownerType === expects.ownerType, actual: query.ownerType ?? "none" });
  }
  if (expects.resolutionConfidence) {
    checks.push({
      name: `resolution confidence ${expects.resolutionConfidence}`,
      pass: query.resolutionConfidence === expects.resolutionConfidence,
      actual: query.resolutionConfidence ?? "none",
    });
  }
  if (scenario.maxMs) {
    checks.push({ name: `elapsed <= ${scenario.maxMs}ms`, pass: elapsedMs <= scenario.maxMs, actual: elapsedMs });
  }
  if (!expects.allowFallbackReason) {
    checks.push({ name: "no wrong fallback", pass: !query.fallback_reason, actual: query.fallback_reason ?? "none" });
  }
  return checks;
}

function normalizePath(value) {
  return String(value).replaceAll(path.sep, "/");
}

function configuredRealFilePaths() {
  const values = [];
  if (process.env.SAGE_REAL_FILE_SMOKE_PATH) {
    values.push(process.env.SAGE_REAL_FILE_SMOKE_PATH);
  }
  if (process.env.SAGE_REAL_FILE_SMOKE_PATHS) {
    values.push(...process.env.SAGE_REAL_FILE_SMOKE_PATHS.split(path.delimiter));
  }
  return values
    .map((value) => value.trim())
    .filter(Boolean)
    .map((value) => path.resolve(value));
}
