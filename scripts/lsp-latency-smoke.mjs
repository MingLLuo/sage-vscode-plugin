#!/usr/bin/env node
import { spawn } from "node:child_process";
import fsSync from "node:fs";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const repositoryRoot = path.resolve(__dirname, "..");
const args = parseArgs(process.argv.slice(2));
const smokeProfile = args.profile ?? process.env.SAGE_LSP_SMOKE_PROFILE ?? "debug";
const normalizedSmokeProfile = smokeProfile === "release" ? "release" : "debug";
const serverPath = process.env.SAGE_LSP_SMOKE_SERVER
  ? path.resolve(process.env.SAGE_LSP_SMOKE_SERVER)
  : path.join(
    repositoryRoot,
    "target",
    normalizedSmokeProfile,
    process.platform === "win32" ? "sage-ls.exe" : "sage-ls",
  );
const publicFixtureCandidates = [
  path.join(repositoryRoot, "examples", "manual-smoke-workspace", "src", "10_sage_heavy_python.py"),
];
const objectMethodFixture = path.join(
  repositoryRoot,
  "examples",
  "manual-smoke-workspace",
  "src",
  "11_sage_object_methods.py",
);
const advancedSageFixture = path.join(
  repositoryRoot,
  "examples",
  "manual-smoke-workspace",
  "src",
  "07_symbolic_and_combinatorics.sage",
);
const manualSmokeWorkspaceRoot = path.join(repositoryRoot, "examples", "manual-smoke-workspace");
const configuredRealFileCandidates = configuredRealFilePaths();
const selectedRealFileCandidates = configuredRealFileCandidates.length > 0
  ? configuredRealFileCandidates
  : publicFixtureCandidates;
const missingRealFileCandidates = selectedRealFileCandidates.filter((candidate) => !fsSync.existsSync(candidate));
const realFile = selectedRealFileCandidates[0];
const sageSourceRoot = process.env.SAGE_SOURCE_ROOT
  ? path.resolve(process.env.SAGE_SOURCE_ROOT)
  : path.resolve(repositoryRoot, "..", "sage", "src");
const sageInterpreterPath = process.env.SAGE_INTERPRETER_PATH
  ? path.resolve(process.env.SAGE_INTERPRETER_PATH)
  : path.resolve(repositoryRoot, "..", "sage", process.platform === "win32" ? "sage.bat" : "sage");
const cacheDir = process.env.SAGE_LSP_SMOKE_CACHE_DIR
  ? path.resolve(process.env.SAGE_LSP_SMOKE_CACHE_DIR)
  : path.join(os.tmpdir(), "sage-vscode-lsp-latency-cache");
let rustCacheDir = cacheDir;
const minRealFoldingRanges = numberFromEnv("SAGE_LSP_MIN_REAL_FOLDING_RANGES", configuredRealFileCandidates.length > 0 ? 20 : 5);
const minRealDocumentSymbols = numberFromEnv("SAGE_LSP_MIN_REAL_DOCUMENT_SYMBOLS", configuredRealFileCandidates.length > 0 ? 20 : 5);
const hoverBudgetMs = numberFromEnv("SAGE_LSP_HOVER_BUDGET_MS", 250);
const definitionBudgetMs = numberFromEnv("SAGE_LSP_DEFINITION_BUDGET_MS", 250);
const referenceBudgetMs = numberFromEnv("SAGE_LSP_REFERENCES_BUDGET_MS", 250);
const typeDefinitionBudgetMs = numberFromEnv("SAGE_LSP_TYPE_DEFINITION_BUDGET_MS", 250);
const inlayBudgetMs = numberFromEnv("SAGE_LSP_INLAY_BUDGET_MS", 250);
const foldingBudgetMs = numberFromEnv("SAGE_LSP_FOLDING_BUDGET_MS", 250);
const completionBudgetMs = numberFromEnv("SAGE_LSP_COMPLETION_BUDGET_MS", 250);
const signatureHelpBudgetMs = numberFromEnv("SAGE_LSP_SIGNATURE_HELP_BUDGET_MS", 250);
const codeActionBudgetMs = numberFromEnv("SAGE_LSP_CODE_ACTION_BUDGET_MS", 250);
const documentHighlightBudgetMs = numberFromEnv("SAGE_LSP_DOCUMENT_HIGHLIGHT_BUDGET_MS", 250);
const selectionRangeBudgetMs = numberFromEnv("SAGE_LSP_SELECTION_RANGE_BUDGET_MS", 250);
const callHierarchyBudgetMs = numberFromEnv("SAGE_LSP_CALL_HIERARCHY_BUDGET_MS", 250);
const documentLinkBudgetMs = numberFromEnv("SAGE_LSP_DOCUMENT_LINK_BUDGET_MS", 250);
const prepareRenameBudgetMs = numberFromEnv("SAGE_LSP_PREPARE_RENAME_BUDGET_MS", 250);
const documentSymbolBudgetMs = numberFromEnv("SAGE_LSP_DOCUMENT_SYMBOL_BUDGET_MS", 250);
const workspaceSymbolBudgetMs = numberFromEnv("SAGE_LSP_WORKSPACE_SYMBOL_BUDGET_MS", 250);
const semanticTokensRangeBudgetMs = numberFromEnv("SAGE_LSP_SEMANTIC_TOKENS_RANGE_BUDGET_MS", 250);
const initializeTargetMs = numberFromEnv("SAGE_LSP_INITIALIZE_TARGET_MS", 500);
const initializeBudgetMs = numberFromEnv("SAGE_LSP_INITIALIZE_BUDGET_MS", 750);

function parseArgs(values) {
  const parsed = {};
  for (let index = 0; index < values.length; index += 1) {
    const value = values[index];
    if (value === "--profile") {
      parsed.profile = values[index + 1];
      index += 1;
    } else if (value.startsWith("--profile=")) {
      parsed.profile = value.slice("--profile=".length);
    }
  }
  return parsed;
}

if (!fsSync.existsSync(serverPath)) {
  console.log(JSON.stringify({ status: "skipped", reason: `missing sage-ls binary: ${serverPath}` }, null, 2));
  process.exit(0);
}
if (missingRealFileCandidates.length > 0) {
  console.log(JSON.stringify({
    status: "failed",
    reason: configuredRealFileCandidates.length > 0
      ? "missing explicitly configured LSP latency smoke paths"
      : "missing checked-in public LSP latency smoke fixture",
    missingFiles: missingRealFileCandidates.map((candidate) => path.resolve(candidate)),
    configured: configuredRealFileCandidates.length > 0,
  }, null, 2));
  process.exit(1);
}
if (!fsSync.existsSync(path.join(sageSourceRoot, "sage"))) {
  console.log(JSON.stringify({ status: "skipped", reason: `missing Sage source root: ${sageSourceRoot}` }, null, 2));
  process.exit(0);
}

async function runSmoke() {
  await fs.mkdir(cacheDir, { recursive: true });
  rustCacheDir = await fs.realpath(cacheDir);
  const source = await fs.readFile(realFile, "utf8");
  const workspaceRoot = path.resolve(realFile, "..");
  const fileUri = pathToFileURL(realFile).toString();
  const workspaceUri = pathToFileURL(workspaceRoot).toString();
  const sageSourceUri = pathToFileURL(sageSourceRoot).toString();
  const documentLinkFile = path.join(cacheDir, "document-link-smoke", "links.sage");
  const documentLinkUri = pathToFileURL(documentLinkFile).toString();
  const documentLinkText = [
    "load(\"helpers/setup.sage\")",
    "attach('../shared/tools.sage')",
    "include \"native_include.pxi\"",
  ].join("\n");
  const documentSymbolFile = path.join(cacheDir, "document-symbol-smoke", "outline.py");
  const documentSymbolUri = pathToFileURL(documentSymbolFile).toString();
  const documentSymbolText = [
    "class Solver:",
    "    def build(self):",
    "        return helper()",
    "",
    "def helper():",
    "    return 1",
  ].join("\n");
  const localFeatureFile = path.join(workspaceRoot, "__sage_lsp_latency_local_smoke__.py");
  const localFeatureUri = pathToFileURL(localFeatureFile).toString();
  const localFeatureText = [
    "from sage.all import matrix, zero_matrix",
    "",
    "def kernel_columns(A):",
    "    if A.ncols() == 0:",
    "        return zero_matrix(A.base_ring(), 0, 0)",
    "    return A.right_kernel().basis_matrix().transpose()",
    "",
    "def caller_one(M):",
    "    return kernel_columns(M)",
    "",
    "def caller_two(M):",
    "    value = kernel_columns(M)",
    "    return value",
    "",
    "result = kernel_col",
  ].join("\n");
  const incrementalFeatureFile = path.join(cacheDir, "incremental-sync-smoke", "incremental.sage");
  const incrementalFeatureUri = pathToFileURL(incrementalFeatureFile).toString();
  const incrementalFeatureText = [
    "def incremental_helper(A):",
    "    return A",
    "",
    "value = incremental_hel",
  ].join("\n");
  const objectFeature = await loadObjectMethodFeature(workspaceRoot);
  const advancedSageFeature = await loadAdvancedSageFeature();
  const scenarios = buildScenarios(source);

  const server = new LspProcess(serverPath);
  try {
    await server.start();
    const initializeStarted = performance.now();
    const initializeResult = await server.request("initialize", {
      processId: process.pid,
      rootUri: workspaceUri,
      capabilities: {},
      workspaceFolders: [{ uri: workspaceUri, name: path.basename(workspaceRoot) }],
      initializationOptions: {
        interpreter: {
          path: sageInterpreterPath,
          args: [],
        },
        rust: {
          binaryPath: serverPath,
          cacheDir: rustCacheDir,
        },
        analysis: {
          mode: "full",
          extraPaths: [workspaceRoot, manualSmokeWorkspaceRoot, sageSourceRoot],
          sourceRoots: [workspaceRoot, manualSmokeWorkspaceRoot, sageSourceRoot],
          enableDiagnostics: true,
          enableRuntimeIntrospection: true,
          enablePyxParsing: true,
          enablePythonFiles: true,
        },
        workspace: {
          rootUri: workspaceUri,
          folders: [workspaceUri],
          sourceRoots: [sageSourceUri],
          exclude: [
            "**/.git/**",
            "**/__pycache__/**",
            "**/.venv/**",
            "**/.quarto/**",
            "**/build/**",
            "**/target/**",
          ],
        },
        documentation: {
          preferredSource: "auto",
          showOnHover: true,
        },
        logging: {
          level: "warn",
        },
      },
    });
    const initializeMs = Math.round(performance.now() - initializeStarted);
    const textDocumentSyncChangeKind = initializeResult?.capabilities?.textDocumentSync?.change ?? null;
    const declarationProvider = initializeResult?.capabilities?.declarationProvider ?? null;
    const completionResolveProvider = initializeResult?.capabilities?.completionProvider?.resolveProvider ?? null;
    server.notify("initialized", {});
    server.notify("textDocument/didOpen", {
      textDocument: {
        uri: fileUri,
        languageId: "python",
        version: 1,
        text: source,
      },
    });
    await fs.mkdir(path.dirname(documentLinkFile), { recursive: true });
    server.notify("textDocument/didOpen", {
      textDocument: {
        uri: documentLinkUri,
        languageId: "sagemath",
        version: 1,
        text: documentLinkText,
      },
    });
    await fs.mkdir(path.dirname(documentSymbolFile), { recursive: true });
    server.notify("textDocument/didOpen", {
      textDocument: {
        uri: documentSymbolUri,
        languageId: "python",
        version: 1,
        text: documentSymbolText,
      },
    });
    server.notify("textDocument/didOpen", {
      textDocument: {
        uri: localFeatureUri,
        languageId: "python",
        version: 1,
        text: localFeatureText,
      },
    });
    await fs.mkdir(path.dirname(incrementalFeatureFile), { recursive: true });
    server.notify("textDocument/didOpen", {
      textDocument: {
        uri: incrementalFeatureUri,
        languageId: "sagemath",
        version: 1,
        text: incrementalFeatureText,
      },
    });
    server.notify("textDocument/didChange", {
      textDocument: {
        uri: incrementalFeatureUri,
        version: 2,
      },
      contentChanges: [
        {
          range: {
            start: { line: 3, character: 23 },
            end: { line: 3, character: 23 },
          },
          text: "per",
        },
      ],
    });
    if (objectFeature) {
      server.notify("textDocument/didOpen", {
        textDocument: {
          uri: objectFeature.uri,
          languageId: "python",
          version: 1,
          text: objectFeature.text,
        },
      });
    }
    if (advancedSageFeature) {
      server.notify("textDocument/didOpen", {
        textDocument: {
          uri: advancedSageFeature.uri,
          languageId: "sagemath",
          version: 1,
          text: advancedSageFeature.text,
        },
      });
    }
    const coldInteractivity = advancedSageFeature
      ? await runColdInteractivityScenario(server, advancedSageFeature)
      : null;
    const indexStatus = await waitForIndex(server);
    const docsStatus = await server.request("workspace/executeCommand", {
      command: "sage.__rust.docsStatus",
      arguments: [],
    });

    const rows = [];
    for (const scenario of scenarios) {
      const hover = await timedRequest(server, "textDocument/hover", {
        textDocument: { uri: fileUri },
        position: scenario.position,
      });
      const definition = await timedRequest(server, "textDocument/definition", {
        textDocument: { uri: fileUri },
        position: scenario.position,
      });
      const declaration = await timedRequest(server, "textDocument/declaration", {
        textDocument: { uri: fileUri },
        position: scenario.position,
      });
      const implementation = await timedRequest(server, "textDocument/implementation", {
        textDocument: { uri: fileUri },
        position: scenario.position,
      });
      rows.push({
        id: scenario.id,
        target: scenario.target,
        hoverMs: hover.elapsedMs,
        definitionMs: definition.elapsedMs,
        declarationMs: declaration.elapsedMs,
        implementationMs: implementation.elapsedMs,
        definition: definitionPath(definition.value),
        declaration: definitionPath(declaration.value),
        implementation: definitionPath(implementation.value),
        status: rowPassed(scenario, hover, definition, implementation, declaration) ? "pass" : "fail",
        checks: [
          {
            name: `hover <= ${hoverBudgetMs}ms`,
            pass: hover.elapsedMs <= hoverBudgetMs,
            actual: hover.elapsedMs,
          },
          {
            name: `definition <= ${definitionBudgetMs}ms`,
            pass: definition.elapsedMs <= definitionBudgetMs,
            actual: definition.elapsedMs,
          },
          {
            name: `definition path includes ${scenario.definitionPathIncludes}`,
            pass: normalizePath(definitionPath(definition.value)).includes(scenario.definitionPathIncludes),
            actual: definitionPath(definition.value),
          },
          {
            name: `declaration <= ${definitionBudgetMs}ms`,
            pass: declaration.elapsedMs <= definitionBudgetMs,
            actual: declaration.elapsedMs,
          },
          {
            name: `declaration path includes ${scenario.definitionPathIncludes}`,
            pass: normalizePath(definitionPath(declaration.value)).includes(scenario.definitionPathIncludes),
            actual: definitionPath(declaration.value),
          },
          {
            name: `implementation <= ${definitionBudgetMs}ms`,
            pass: implementation.elapsedMs <= definitionBudgetMs,
            actual: implementation.elapsedMs,
          },
          {
            name: `implementation path includes ${scenario.definitionPathIncludes}`,
            pass: normalizePath(definitionPath(implementation.value)).includes(scenario.definitionPathIncludes),
            actual: definitionPath(implementation.value),
          },
        ],
      });
    }
    const objectRows = objectFeature
      ? await runObjectMethodScenarios(server, objectFeature)
      : [];
    const objectTypeDefinitionRows = objectFeature
      ? await runObjectTypeDefinitionScenarios(server, objectFeature)
      : [];
    const advancedSageRows = advancedSageFeature
      ? await runAdvancedSageScenarios(server, advancedSageFeature)
      : [];
    const inlayHints = await timedRequest(server, "textDocument/inlayHint", {
      textDocument: { uri: fileUri },
      range: {
        start: { line: 0, character: 0 },
        end: { line: Math.min(320, source.split(/\r?\n/).length), character: 0 },
      },
    });
    const inlayLabels = (inlayHints.value ?? []).map(inlayLabel);
    const objectInlayHints = objectFeature
      ? await timedRequest(server, "textDocument/inlayHint", {
          textDocument: { uri: objectFeature.uri },
          range: {
            start: { line: 0, character: 0 },
            end: { line: Math.min(320, objectFeature.text.split(/\r?\n/).length), character: 0 },
          },
        })
      : { elapsedMs: 0, value: [] };
    const objectInlayLabels = (objectInlayHints.value ?? []).map(inlayLabel);
    const inlayChecks = [
      {
        name: `inlay hints <= ${inlayBudgetMs}ms`,
        pass: inlayHints.elapsedMs <= inlayBudgetMs,
        actual: inlayHints.elapsedMs,
      },
      {
        name: "inlay hints include PolynomialRing",
        pass: inlayLabels.includes(": PolynomialRing"),
        actual: inlayLabels,
      },
      {
        name: "inlay hints include Matrix",
        pass: inlayLabels.includes(": Matrix"),
        actual: inlayLabels,
      },
      {
        name: "inlay hints include Vector",
        pass: inlayLabels.includes(": Vector"),
        actual: inlayLabels,
      },
      ...optionalChecks(objectFeature, [
        {
          name: `object inlay hints <= ${inlayBudgetMs}ms`,
          pass: objectInlayHints.elapsedMs <= inlayBudgetMs,
          actual: objectInlayHints.elapsedMs,
        },
        {
          name: "object inlay hints include Graph",
          pass: objectInlayLabels.includes(": Graph"),
          actual: objectInlayLabels,
        },
        {
          name: "object inlay hints include EllipticCurve",
          pass: objectInlayLabels.includes(": EllipticCurve"),
          actual: objectInlayLabels,
        },
        {
          name: "object inlay hints include NumberField",
          pass: objectInlayLabels.includes(": NumberField"),
          actual: objectInlayLabels,
        },
      ]),
    ];
    const foldingRanges = await timedRequest(server, "textDocument/foldingRange", {
      textDocument: { uri: fileUri },
    });
    const foldingItems = Array.isArray(foldingRanges.value) ? foldingRanges.value : [];
    const localFoldingRanges = await timedRequest(server, "textDocument/foldingRange", {
      textDocument: { uri: localFeatureUri },
    });
    const localFoldingItems = Array.isArray(localFoldingRanges.value) ? localFoldingRanges.value : [];
    const kernelColumnsPosition = symbolPosition(localFeatureText, "kernel_columns");
    const foldingChecks = [
      {
        name: `folding ranges <= ${foldingBudgetMs}ms`,
        pass: foldingRanges.elapsedMs <= foldingBudgetMs,
        actual: foldingRanges.elapsedMs,
      },
      {
        name: "folding ranges include substantial real-file structure",
        pass: foldingItems.length >= minRealFoldingRanges,
        actual: foldingItems.length,
      },
      {
        name: "local folding ranges include kernel_columns function",
        pass: localFoldingItems.some((range) => range?.startLine === kernelColumnsPosition?.line),
        actual: localFoldingItems
          .slice(0, 40)
          .map((range) => ({ startLine: range?.startLine, endLine: range?.endLine, kind: range?.kind })),
      },
    ];
    const documentSymbols = await timedRequest(server, "textDocument/documentSymbol", {
      textDocument: { uri: fileUri },
    });
    const documentSymbolItems = Array.isArray(documentSymbols.value) ? documentSymbols.value : [];
    const documentSymbolNames = flattenDocumentSymbolNames(documentSymbolItems);
    const localDocumentSymbols = await timedRequest(server, "textDocument/documentSymbol", {
      textDocument: { uri: localFeatureUri },
    });
    const localDocumentSymbolItems = Array.isArray(localDocumentSymbols.value)
      ? localDocumentSymbols.value
      : [];
    const localDocumentSymbolNames = flattenDocumentSymbolNames(localDocumentSymbolItems);
    const nestedDocumentSymbols = await timedRequest(server, "textDocument/documentSymbol", {
      textDocument: { uri: documentSymbolUri },
    });
    const nestedDocumentSymbolItems = Array.isArray(nestedDocumentSymbols.value)
      ? nestedDocumentSymbols.value
      : [];
    const documentSymbolChecks = [
      {
        name: `document symbols <= ${documentSymbolBudgetMs}ms`,
        pass: documentSymbols.elapsedMs <= documentSymbolBudgetMs,
        actual: documentSymbols.elapsedMs,
      },
      {
        name: "document symbols include substantial real-file outline",
        pass: documentSymbolNames.length >= minRealDocumentSymbols,
        actual: documentSymbolNames.slice(0, 40),
      },
      {
        name: `local document symbols <= ${documentSymbolBudgetMs}ms`,
        pass: localDocumentSymbols.elapsedMs <= documentSymbolBudgetMs,
        actual: localDocumentSymbols.elapsedMs,
      },
      {
        name: "local document symbols include kernel_columns",
        pass: localDocumentSymbolNames.includes("kernel_columns"),
        actual: localDocumentSymbolNames.slice(0, 40),
      },
      {
        name: "local document symbols hide module/import metadata",
        pass: !localDocumentSymbolNames.some((name) =>
          ["__sage_lsp_latency_local_smoke__", "matrix", "zero_matrix"].includes(name),
        ),
        actual: localDocumentSymbolNames.slice(0, 40),
      },
      {
        name: `nested document symbols <= ${documentSymbolBudgetMs}ms`,
        pass: nestedDocumentSymbols.elapsedMs <= documentSymbolBudgetMs,
        actual: nestedDocumentSymbols.elapsedMs,
      },
      {
        name: "document symbols return nested VS Code outline data",
        pass: hasNestedDocumentSymbol(nestedDocumentSymbolItems, "Solver", "build"),
        actual: nestedDocumentSymbolItems,
      },
    ];
    const workspaceSymbols = await timedRequest(server, "workspace/symbol", {
      query: "PolynomialRing",
    });
    const workspaceSymbolItems = Array.isArray(workspaceSymbols.value) ? workspaceSymbols.value : [];
    const firstWorkspaceSymbol = workspaceSymbolItems[0] ?? null;
    const workspaceSymbolChecks = [
      {
        name: `workspace symbols <= ${workspaceSymbolBudgetMs}ms`,
        pass: workspaceSymbols.elapsedMs <= workspaceSymbolBudgetMs,
        actual: workspaceSymbols.elapsedMs,
      },
      {
        name: "workspace symbols rank exact Sage constructor first",
        pass: firstWorkspaceSymbol?.name === "PolynomialRing",
        actual: workspaceSymbolItems.slice(0, 10).map((symbol) => ({
          name: symbol?.name,
          containerName: symbol?.containerName,
          uri: symbol?.location?.uri,
        })),
      },
      {
        name: "workspace symbols resolve PolynomialRing to Sage source",
        pass: firstWorkspaceSymbol?.location?.uri?.includes("sage/rings/polynomial/polynomial_ring_constructor.py"),
        actual: firstWorkspaceSymbol?.location?.uri ?? null,
      },
      {
        name: "workspace symbols suppress duplicate import noise",
        pass: workspaceSymbolItems.length <= 20,
        actual: workspaceSymbolItems.length,
      },
    ];
    const semanticTokensRange = await timedRequest(server, "textDocument/semanticTokens/range", {
      textDocument: { uri: fileUri },
      range: {
        start: { line: 0, character: 0 },
        end: { line: Math.min(320, source.split(/\r?\n/).length), character: 0 },
      },
    });
    const semanticTokensRangeCount = semanticTokenDataLength(semanticTokensRange.value);
    const semanticTokensRangeChecks = [
      {
        name: `semantic tokens range <= ${semanticTokensRangeBudgetMs}ms`,
        pass: semanticTokensRange.elapsedMs <= semanticTokensRangeBudgetMs,
        actual: semanticTokensRange.elapsedMs,
      },
      {
        name: "semantic tokens range returns viewport token data",
        pass: semanticTokensRangeCount > 0,
        actual: semanticTokensRangeCount,
      },
    ];
    const signatureHelpPosition = callArgumentPosition(source, "PolynomialRing");
    const signatureHelp = signatureHelpPosition
      ? await timedRequest(server, "textDocument/signatureHelp", {
          textDocument: { uri: fileUri },
          position: signatureHelpPosition,
        })
      : { elapsedMs: 0, value: null };
    const signatureLabel = signatureHelpLabel(signatureHelp.value);
    const signatureParameterCount = signatureHelpParameterCount(signatureHelp.value);
    const signatureActiveParameter = signatureHelpActiveParameter(signatureHelp.value);
    const signatureHelpChecks = [
      {
        name: `signature help <= ${signatureHelpBudgetMs}ms`,
        pass: signatureHelp.elapsedMs <= signatureHelpBudgetMs,
        actual: signatureHelp.elapsedMs,
      },
      {
        name: "signature help resolves Sage constructor",
        pass: signatureLabel.includes("PolynomialRing"),
        actual: signatureLabel,
      },
      {
        name: "signature help includes parameter labels",
        pass: signatureParameterCount >= 2,
        actual: signatureParameterCount,
      },
      {
        name: "signature help reports active parameter",
        pass: signatureActiveParameter === 0,
        actual: signatureActiveParameter,
      },
    ];
    const completionPosition = memberCompletionPosition(source, "rank", "ra");
    const completion = completionPosition
      ? await timedRequest(server, "textDocument/completion", {
          textDocument: { uri: fileUri },
          position: completionPosition,
        })
      : { elapsedMs: 0, value: [] };
    const completionItems = completionItemsFromResult(completion.value);
    const completionLabels = completionItems.map(completionLabel);
    const rankCompletion = completionItems.find((item) => completionLabel(item) === "rank");
    const resolvedRankCompletion = rankCompletion
      ? await timedRequest(server, "completionItem/resolve", rankCompletion)
      : { elapsedMs: 0, value: null };
    const completionChecks = [
      {
        name: `member completion <= ${completionBudgetMs}ms`,
        pass: completion.elapsedMs <= completionBudgetMs,
        actual: completion.elapsedMs,
      },
      {
        name: "matrix member completion includes rank",
        pass: completionLabels.includes("rank"),
        actual: completionLabels.slice(0, 20),
      },
      {
        name: "matrix member completion is not global symbol noise",
        pass: !completionLabels.includes("PolynomialRing"),
        actual: completionLabels.slice(0, 20),
      },
      {
        name: "matrix member completion carries signature or docs",
        pass: hasCompletionDetail(rankCompletion),
        actual: completionSummary(rankCompletion),
      },
      {
        name: `completion resolve <= ${completionBudgetMs}ms`,
        pass: resolvedRankCompletion.elapsedMs <= completionBudgetMs,
        actual: resolvedRankCompletion.elapsedMs,
      },
      {
        name: "completion resolve returns Sage docs lazily",
        pass: completionDocumentationText(resolvedRankCompletion.value).includes("rank"),
        actual: completionSummary(resolvedRankCompletion.value),
      },
    ];
    const objectCompletionPosition = objectFeature
      ? memberCompletionPosition(objectFeature.text, "vertices", "ver")
      : null;
    const objectCompletion = objectCompletionPosition
      ? await timedRequest(server, "textDocument/completion", {
          textDocument: { uri: objectFeature.uri },
          position: objectCompletionPosition,
        })
      : { elapsedMs: 0, value: [] };
    const objectCompletionItems = completionItemsFromResult(objectCompletion.value);
    const objectCompletionLabels = objectCompletionItems.map(completionLabel);
    const verticesCompletion = objectCompletionItems.find((item) => completionLabel(item) === "vertices");
    const objectCompletionChecks = optionalChecks(objectFeature, [
      {
        name: `object member completion <= ${completionBudgetMs}ms`,
        pass: objectCompletion.elapsedMs <= completionBudgetMs,
        actual: objectCompletion.elapsedMs,
      },
      {
        name: "Graph member completion includes vertices",
        pass: objectCompletionLabels.includes("vertices"),
        actual: objectCompletionLabels.slice(0, 20),
      },
      {
        name: "Graph member completion is not global symbol noise",
        pass: !objectCompletionLabels.includes("PolynomialRing"),
        actual: objectCompletionLabels.slice(0, 20),
      },
      {
        name: "Graph member completion carries signature or docs",
        pass: hasCompletionDetail(verticesCompletion),
        actual: completionSummary(verticesCompletion),
      },
    ]);
    const localCompletionPosition = prefixPosition(localFeatureText, "kernel_col");
    const localCompletion = localCompletionPosition
      ? await timedRequest(server, "textDocument/completion", {
          textDocument: { uri: localFeatureUri },
          position: localCompletionPosition,
        })
      : { elapsedMs: 0, value: [] };
    const localCompletionItems = completionItemsFromResult(localCompletion.value);
    const localCompletionLabels = localCompletionItems.map(completionLabel);
    const kernelCompletion = localCompletionItems.find((item) => completionLabel(item) === "kernel_columns");
    const localCompletionChecks = [
      {
        name: `local completion <= ${completionBudgetMs}ms`,
        pass: localCompletion.elapsedMs <= completionBudgetMs,
        actual: localCompletion.elapsedMs,
      },
      {
        name: "open document completion includes kernel_columns",
        pass: localCompletionLabels.includes("kernel_columns"),
        actual: localCompletionLabels.slice(0, 20),
      },
      {
        name: "open document completion ranks local symbol first",
        pass: localCompletionLabels[0] === "kernel_columns",
        actual: localCompletionLabels.slice(0, 20),
      },
      {
        name: "open document completion carries local signature",
        pass: String(kernelCompletion?.detail ?? "").includes("kernel_columns("),
        actual: completionSummary(kernelCompletion),
      },
    ];
    const documentHighlightPosition = symbolPosition(localFeatureText, "kernel_columns");
    const documentHighlight = documentHighlightPosition
      ? await timedRequest(server, "textDocument/documentHighlight", {
          textDocument: { uri: localFeatureUri },
          position: documentHighlightPosition,
        })
      : { elapsedMs: 0, value: [] };
    const documentHighlightLines = documentHighlightLinesFromResult(documentHighlight.value);
    const documentHighlightChecks = [
      {
        name: `document highlight <= ${documentHighlightBudgetMs}ms`,
        pass: documentHighlight.elapsedMs <= documentHighlightBudgetMs,
        actual: documentHighlight.elapsedMs,
      },
      {
        name: "document highlight returns repeated local helper references",
        pass: documentHighlightLines.length >= 3,
        actual: documentHighlightLines.slice(0, 20),
      },
      {
        name: "document highlight includes the declaration line",
        pass: documentHighlightLines.includes(documentHighlightPosition?.line),
        actual: documentHighlightLines.slice(0, 20),
      },
    ];
    const selectionRange = documentHighlightPosition
      ? await timedRequest(server, "textDocument/selectionRange", {
          textDocument: { uri: localFeatureUri },
          positions: [documentHighlightPosition],
        })
      : { elapsedMs: 0, value: [] };
    const selectionRangeChain = selectionRangeChainFromResult(selectionRange.value?.[0]);
    const selectionRangeChecks = [
      {
        name: `selection range <= ${selectionRangeBudgetMs}ms`,
        pass: selectionRange.elapsedMs <= selectionRangeBudgetMs,
        actual: selectionRange.elapsedMs,
      },
      {
        name: "selection range expands through symbol, line, block, and document",
        pass: selectionRangeChain.length >= 4,
        actual: selectionRangeChain,
      },
      {
        name: "selection range starts at requested symbol",
        pass: selectionRangeChain[0]?.start?.line === documentHighlightPosition?.line
          && selectionRangeChain[0]?.start?.character === documentHighlightPosition?.character,
        actual: selectionRangeChain[0] ?? null,
      },
    ];
    const callHierarchyPrepare = documentHighlightPosition
      ? await timedRequest(server, "textDocument/prepareCallHierarchy", {
          textDocument: { uri: localFeatureUri },
          position: documentHighlightPosition,
        })
      : { elapsedMs: 0, value: [] };
    const callHierarchyItem = Array.isArray(callHierarchyPrepare.value) ? callHierarchyPrepare.value[0] : null;
    const callHierarchyIncoming = callHierarchyItem
      ? await timedRequest(server, "callHierarchy/incomingCalls", { item: callHierarchyItem })
      : { elapsedMs: 0, value: [] };
    const callHierarchyOutgoing = callHierarchyItem
      ? await timedRequest(server, "callHierarchy/outgoingCalls", { item: callHierarchyItem })
      : { elapsedMs: 0, value: [] };
    const incomingCallNames = callHierarchyIncomingNames(callHierarchyIncoming.value);
    const outgoingCallNames = callHierarchyOutgoingNames(callHierarchyOutgoing.value);
    const callHierarchyChecks = [
      {
        name: `call hierarchy prepare <= ${callHierarchyBudgetMs}ms`,
        pass: callHierarchyPrepare.elapsedMs <= callHierarchyBudgetMs,
        actual: callHierarchyPrepare.elapsedMs,
      },
      {
        name: "call hierarchy prepares kernel_columns",
        pass: callHierarchyItem?.name === "kernel_columns",
        actual: callHierarchyItem?.name ?? null,
      },
      {
        name: `call hierarchy incoming <= ${callHierarchyBudgetMs}ms`,
        pass: callHierarchyIncoming.elapsedMs <= callHierarchyBudgetMs,
        actual: callHierarchyIncoming.elapsedMs,
      },
      {
        name: "call hierarchy incoming finds real callers",
        pass: incomingCallNames.length >= 1,
        actual: incomingCallNames.slice(0, 20),
      },
      {
        name: `call hierarchy outgoing <= ${callHierarchyBudgetMs}ms`,
        pass: callHierarchyOutgoing.elapsedMs <= callHierarchyBudgetMs,
        actual: callHierarchyOutgoing.elapsedMs,
      },
      {
        name: "call hierarchy outgoing includes zero_matrix",
        pass: outgoingCallNames.includes("zero_matrix"),
        actual: outgoingCallNames.slice(0, 20),
      },
    ];
    const documentLink = await timedRequest(server, "textDocument/documentLink", {
      textDocument: { uri: documentLinkUri },
    });
    const documentLinkTargets = documentLinkTargetsFromResult(documentLink.value);
    const documentLinkChecks = [
      {
        name: `document links <= ${documentLinkBudgetMs}ms`,
        pass: documentLink.elapsedMs <= documentLinkBudgetMs,
        actual: documentLink.elapsedMs,
      },
      {
        name: "document links cover load, attach, and include",
        pass: documentLinkTargets.length === 3,
        actual: documentLinkTargets,
      },
      {
        name: "document links resolve relative Sage/Cython paths",
        pass: documentLinkTargets.some((target) => target.endsWith("/helpers/setup.sage"))
          && documentLinkTargets.some((target) => target.endsWith("/shared/tools.sage"))
          && documentLinkTargets.some((target) => target.endsWith("/native_include.pxi")),
        actual: documentLinkTargets,
      },
    ];
    const prepareRename = documentHighlightPosition
      ? await timedRequest(server, "textDocument/prepareRename", {
          textDocument: { uri: localFeatureUri },
          position: documentHighlightPosition,
        })
      : { elapsedMs: 0, value: null };
    const externalPrepareRenamePosition = symbolPosition(source, "PolynomialRing");
    const externalPrepareRename = externalPrepareRenamePosition
      ? await timedRequest(server, "textDocument/prepareRename", {
          textDocument: { uri: fileUri },
          position: externalPrepareRenamePosition,
        })
      : { elapsedMs: 0, value: null };
    const prepareRenameChecks = [
      {
        name: `prepare rename local <= ${prepareRenameBudgetMs}ms`,
        pass: prepareRename.elapsedMs <= prepareRenameBudgetMs,
        actual: prepareRename.elapsedMs,
      },
      {
        name: "prepare rename returns local placeholder",
        pass: prepareRename.value?.placeholder === "kernel_columns",
        actual: prepareRename.value,
      },
      {
        name: `prepare rename external <= ${prepareRenameBudgetMs}ms`,
        pass: externalPrepareRename.elapsedMs <= prepareRenameBudgetMs,
        actual: externalPrepareRename.elapsedMs,
      },
      {
        name: "prepare rename suppresses external Sage API",
        pass: externalPrepareRename.value == null,
        actual: externalPrepareRename.value,
      },
    ];
    const codeAction = await timedRequest(server, "textDocument/codeAction", {
      textDocument: { uri: fileUri },
      range: {
        start: { line: 0, character: 0 },
        end: { line: 0, character: 1 },
      },
      context: {
        diagnostics: [
          {
            range: {
              start: { line: 0, character: 9 },
              end: { line: 0, character: 10 },
            },
            severity: 1,
            code: "syntax-error",
            source: "sage-ls",
            message: "Syntax error: incomplete Sage exponentiation",
          },
          {
            range: {
              start: { line: 2, character: 9 },
              end: { line: 2, character: 10 },
            },
            severity: 2,
            code: "sage-python-caret-exponent",
            source: "sage-ls",
            message: "Sage-style exponent operator `^` has Python XOR semantics in `.py`; use `**`.",
          },
        ],
      },
    });
    const codeActionTitles = codeActionTitlesFromResult(codeAction.value);
    const codeActionChecks = [
      {
        name: `code action <= ${codeActionBudgetMs}ms`,
        pass: codeAction.elapsedMs <= codeActionBudgetMs,
        actual: codeAction.elapsedMs,
      },
      {
        name: "Sage exponent diagnostic offers remove quick fix",
        pass: codeActionTitles.includes("Remove incomplete Sage exponent operator"),
        actual: codeActionTitles,
      },
      {
        name: "Sage exponent diagnostic offers placeholder quick fix",
        pass: codeActionTitles.includes("Insert exponent placeholder"),
        actual: codeActionTitles,
      },
      {
        name: "Python Sage caret diagnostic offers ** quick fix",
        pass: codeActionTitles.includes("Replace Sage-style ^ with Python exponent **"),
        actual: codeActionTitles,
      },
    ];
    const incrementalDefinition = await timedRequest(server, "textDocument/definition", {
      textDocument: { uri: incrementalFeatureUri },
      position: { line: 3, character: 10 },
    });
    const incrementalDocumentSymbols = await timedRequest(server, "textDocument/documentSymbol", {
      textDocument: { uri: incrementalFeatureUri },
    });
    const incrementalDocumentSymbolNames = flattenDocumentSymbolNames(
      Array.isArray(incrementalDocumentSymbols.value) ? incrementalDocumentSymbols.value : [],
    );
    const incrementalSyncChecks = [
      {
        name: "server advertises incremental text synchronization",
        pass: textDocumentSyncChangeKind === 2,
        actual: textDocumentSyncChangeKind,
      },
      {
        name: `incremental definition <= ${definitionBudgetMs}ms`,
        pass: incrementalDefinition.elapsedMs <= definitionBudgetMs,
        actual: incrementalDefinition.elapsedMs,
      },
      {
        name: "incremental edit updates navigation without full document resend",
        pass: normalizePath(definitionPath(incrementalDefinition.value)).includes("incremental-sync-smoke/incremental.sage"),
        actual: definitionPath(incrementalDefinition.value),
      },
      {
        name: `incremental document symbols <= ${documentSymbolBudgetMs}ms`,
        pass: incrementalDocumentSymbols.elapsedMs <= documentSymbolBudgetMs,
        actual: incrementalDocumentSymbols.elapsedMs,
      },
      {
        name: "incremental document symbols preserve open-document structure",
        pass: incrementalDocumentSymbolNames.includes("incremental_helper"),
        actual: incrementalDocumentSymbolNames,
      },
    ];
    const hotCacheReady = (indexStatus?.hot_symbol_cache_count ?? 0) > 0;
    const statusChecks = [
      {
        name: `initialize target <= ${initializeTargetMs}ms`,
        pass: true,
        actual: initializeMs,
        target: initializeTargetMs,
        warning: initializeMs > initializeTargetMs,
      },
      {
        name: `initialize hard cap <= ${initializeBudgetMs}ms`,
        pass: initializeMs <= initializeBudgetMs,
        actual: initializeMs,
      },
      {
        name: "index status has no last_error",
        pass: !indexStatus?.last_error,
        actual: indexStatus?.last_error ?? null,
      },
      {
        name: "server advertises declaration provider",
        pass: declarationProvider === true || typeof declarationProvider === "object",
        actual: declarationProvider,
      },
      {
        name: "server advertises completion resolve provider",
        pass: completionResolveProvider === true,
        actual: completionResolveProvider,
      },
      {
        name: "hot cache populated",
        pass: hotCacheReady,
        actual: indexStatus?.hot_symbol_cache_count ?? 0,
      },
      {
        name: "cache namespace exposed",
        pass: typeof indexStatus?.cache_namespace === "string" && indexStatus.cache_namespace.length === 16,
        actual: indexStatus?.cache_namespace ?? null,
      },
      {
        name: "cache path stays inside isolated smoke cache",
        pass: cachePathIsInsideSmokeRoot(indexStatus?.cache_path),
        actual: indexStatus?.cache_path ?? null,
      },
      {
        name: "source root fingerprints exposed",
        pass: Array.isArray(indexStatus?.source_root_fingerprints)
          && indexStatus.source_root_fingerprints.length > 0
          && indexStatus.source_root_fingerprints.every((entry) => typeof entry.digest === "string" && entry.digest.length === 16),
        actual: indexStatus?.source_root_fingerprints ?? null,
      },
      {
        name: "cache stale flag exposed",
        pass: typeof indexStatus?.cache_stale === "boolean",
        actual: indexStatus?.cache_stale ?? null,
      },
      {
        name: "last operation exposed",
        pass: typeof indexStatus?.last_operation === "string" && indexStatus.last_operation.length > 0,
        actual: indexStatus?.last_operation ?? null,
      },
      ...["last_hydrate_ms", "last_reconcile_ms", "last_persist_ms", "last_hot_cache_ms"].map((field) => ({
        name: `${field} present`,
        pass: typeof indexStatus?.[field] === "number" && Number.isFinite(indexStatus[field]),
        actual: indexStatus?.[field],
      })),
    ];
    const docsStatusChecks = [
      {
        name: "docs status exposes preferred source",
        pass: docsStatus?.preferred_source === "auto",
        actual: docsStatus?.preferred_source ?? null,
      },
      {
        name: "docs status exposes runtime state",
        pass: typeof docsStatus?.runtime_worker_state === "string" && docsStatus.runtime_worker_state.length > 0,
        actual: docsStatus?.runtime_worker_state ?? null,
      },
      {
        name: "docs status avoids vague not-started state",
        pass: docsStatus?.runtime_worker_state !== "not-started",
        actual: docsStatus?.runtime_worker_state ?? null,
      },
      {
        name: "docs degraded/unavailable states are explainable",
        pass: docsStatusExplainable(docsStatus),
        actual: {
          state: docsStatus?.runtime_worker_state ?? null,
          reason: docsStatus?.runtime_degraded_reason ?? null,
        },
      },
    ];
    const statusReady = statusChecks.every((check) => check.pass);
    const docsStatusReady = docsStatusChecks.every((check) => check.pass);
    const inlayReady = inlayChecks.every((check) => check.pass);
    const foldingReady = foldingChecks.every((check) => check.pass);
    const completionReady = completionChecks.every((check) => check.pass);
    const objectRowsReady = objectRows.every((row) => row.status === "pass");
    const objectTypeDefinitionReady = objectTypeDefinitionRows.every((row) => row.status === "pass");
    const advancedSageReady = advancedSageRows.every((row) => row.status === "pass");
    const objectCompletionReady = objectCompletionChecks.every((check) => check.pass);
    const localCompletionReady = localCompletionChecks.every((check) => check.pass);
    const documentHighlightReady = documentHighlightChecks.every((check) => check.pass);
    const selectionRangeReady = selectionRangeChecks.every((check) => check.pass);
    const callHierarchyReady = callHierarchyChecks.every((check) => check.pass);
    const documentLinkReady = documentLinkChecks.every((check) => check.pass);
    const documentSymbolReady = documentSymbolChecks.every((check) => check.pass);
    const workspaceSymbolReady = workspaceSymbolChecks.every((check) => check.pass);
    const semanticTokensRangeReady = semanticTokensRangeChecks.every((check) => check.pass);
    const signatureHelpReady = signatureHelpChecks.every((check) => check.pass);
    const prepareRenameReady = prepareRenameChecks.every((check) => check.pass);
    const codeActionReady = codeActionChecks.every((check) => check.pass);
    const incrementalSyncReady = incrementalSyncChecks.every((check) => check.pass);
    const coldInteractivityReady = coldInteractivity
      ? coldInteractivity.checks.every((check) => check.pass)
      : true;
    const failed = rows.filter((row) => row.status !== "pass");
    console.log(JSON.stringify({
      status: failed.length || !objectRowsReady || !objectTypeDefinitionReady || !advancedSageReady || !statusReady || !docsStatusReady || !inlayReady || !foldingReady || !documentSymbolReady || !workspaceSymbolReady || !semanticTokensRangeReady || !signatureHelpReady || !completionReady || !objectCompletionReady || !localCompletionReady || !documentHighlightReady || !selectionRangeReady || !callHierarchyReady || !documentLinkReady || !prepareRenameReady || !codeActionReady || !incrementalSyncReady || !coldInteractivityReady ? "failed" : "passed",
      file: realFile,
      server: serverPath,
      profile: normalizedSmokeProfile,
      initializeMs,
      objectFeatureFile: objectFeature?.file ?? null,
      advancedSageFeatureFile: advancedSageFeature?.file ?? null,
      hotCacheReady,
      indexStatus,
      docsStatus,
      statusChecks,
      docsStatusChecks,
      inlayHints: {
        elapsedMs: inlayHints.elapsedMs,
        count: inlayLabels.length,
        labels: [...new Set(inlayLabels)].sort(),
        objectElapsedMs: objectInlayHints.elapsedMs,
        objectLabels: [...new Set(objectInlayLabels)].sort(),
        checks: inlayChecks,
      },
      foldingRanges: {
        elapsedMs: foldingRanges.elapsedMs,
        count: foldingItems.length,
        sample: foldingItems.slice(0, 20),
        localElapsedMs: localFoldingRanges.elapsedMs,
        localCount: localFoldingItems.length,
        checks: foldingChecks,
      },
      documentSymbols: {
        elapsedMs: documentSymbols.elapsedMs,
        count: documentSymbolNames.length,
        names: documentSymbolNames.slice(0, 40),
        localElapsedMs: localDocumentSymbols.elapsedMs,
        localNames: localDocumentSymbolNames.slice(0, 40),
        nestedElapsedMs: nestedDocumentSymbols.elapsedMs,
        nestedSample: nestedDocumentSymbolItems,
        checks: documentSymbolChecks,
      },
      workspaceSymbols: {
        elapsedMs: workspaceSymbols.elapsedMs,
        count: workspaceSymbolItems.length,
        first: firstWorkspaceSymbol,
        sample: workspaceSymbolItems.slice(0, 20).map((symbol) => ({
          name: symbol?.name,
          containerName: symbol?.containerName,
          uri: symbol?.location?.uri,
        })),
        checks: workspaceSymbolChecks,
      },
      semanticTokensRange: {
        elapsedMs: semanticTokensRange.elapsedMs,
        dataLength: semanticTokensRangeCount,
        checks: semanticTokensRangeChecks,
      },
      signatureHelp: {
        elapsedMs: signatureHelp.elapsedMs,
        label: signatureLabel,
        parameterCount: signatureParameterCount,
        activeParameter: signatureActiveParameter,
        checks: signatureHelpChecks,
      },
      memberCompletion: {
        elapsedMs: completion.elapsedMs,
        count: completionLabels.length,
        labels: completionLabels.slice(0, 40),
        topItem: completionSummary(rankCompletion),
        checks: completionChecks,
      },
      objectMemberCompletion: {
        elapsedMs: objectCompletion.elapsedMs,
        count: objectCompletionLabels.length,
        labels: objectCompletionLabels.slice(0, 40),
        topItem: completionSummary(verticesCompletion),
        checks: objectCompletionChecks,
      },
      localCompletion: {
        elapsedMs: localCompletion.elapsedMs,
        count: localCompletionLabels.length,
        labels: localCompletionLabels.slice(0, 40),
        topItem: completionSummary(kernelCompletion),
        checks: localCompletionChecks,
      },
      documentHighlight: {
        elapsedMs: documentHighlight.elapsedMs,
        count: documentHighlightLines.length,
        lines: documentHighlightLines.slice(0, 40),
        checks: documentHighlightChecks,
      },
      selectionRange: {
        elapsedMs: selectionRange.elapsedMs,
        depth: selectionRangeChain.length,
        chain: selectionRangeChain.slice(0, 8),
        checks: selectionRangeChecks,
      },
      callHierarchy: {
        prepareMs: callHierarchyPrepare.elapsedMs,
        incomingMs: callHierarchyIncoming.elapsedMs,
        outgoingMs: callHierarchyOutgoing.elapsedMs,
        item: callHierarchyItem
          ? {
              name: callHierarchyItem.name,
              kind: callHierarchyItem.kind,
              range: callHierarchyItem.range,
              selectionRange: callHierarchyItem.selectionRange,
            }
          : null,
        incoming: incomingCallNames.slice(0, 40),
        outgoing: outgoingCallNames.slice(0, 40),
        checks: callHierarchyChecks,
      },
      documentLink: {
        elapsedMs: documentLink.elapsedMs,
        targets: documentLinkTargets,
        checks: documentLinkChecks,
      },
      prepareRename: {
        localMs: prepareRename.elapsedMs,
        local: prepareRename.value,
        externalMs: externalPrepareRename.elapsedMs,
        external: externalPrepareRename.value,
        checks: prepareRenameChecks,
      },
      codeAction: {
        elapsedMs: codeAction.elapsedMs,
        titles: codeActionTitles,
        checks: codeActionChecks,
      },
      incrementalSync: {
        changeKind: textDocumentSyncChangeKind,
        definitionMs: incrementalDefinition.elapsedMs,
        definition: definitionPath(incrementalDefinition.value),
        documentSymbolMs: incrementalDocumentSymbols.elapsedMs,
        documentSymbols: incrementalDocumentSymbolNames,
        checks: incrementalSyncChecks,
      },
      coldInteractivity,
      rows,
      objectRows,
      objectTypeDefinitionRows,
      advancedSageRows,
    }, null, 2));
    process.exitCode = failed.length || !objectRowsReady || !objectTypeDefinitionReady || !advancedSageReady || !statusReady || !docsStatusReady || !inlayReady || !foldingReady || !documentSymbolReady || !workspaceSymbolReady || !semanticTokensRangeReady || !signatureHelpReady || !completionReady || !objectCompletionReady || !localCompletionReady || !documentHighlightReady || !selectionRangeReady || !callHierarchyReady || !documentLinkReady || !prepareRenameReady || !codeActionReady || !incrementalSyncReady || !coldInteractivityReady ? 1 : 0;
  } finally {
    server.stop();
  }
}

function docsStatusExplainable(status) {
  const state = status?.runtime_worker_state;
  if (!state || state === "not-started") {
    return false;
  }
  if (["disabled", "unavailable", "degraded", "unconfigured-static-fallback", "static-fallback"].includes(state)) {
    return Boolean(status?.runtime_degraded_reason);
  }
  return true;
}

async function loadObjectMethodFeature(workspaceRoot) {
  if (configuredRealFileCandidates.length > 0) {
    return null;
  }
  if (!fsSync.existsSync(objectMethodFixture) || path.dirname(objectMethodFixture) !== workspaceRoot) {
    return null;
  }
  return {
    file: objectMethodFixture,
    uri: pathToFileURL(objectMethodFixture).toString(),
    text: await fs.readFile(objectMethodFixture, "utf8"),
  };
}

async function loadAdvancedSageFeature() {
  if (!fsSync.existsSync(advancedSageFixture)) {
    return null;
  }
  return {
    file: advancedSageFixture,
    uri: pathToFileURL(advancedSageFixture).toString(),
    text: await fs.readFile(advancedSageFixture, "utf8"),
  };
}

async function runColdInteractivityScenario(server, advancedSageFeature) {
  const position = symbolPosition(advancedSageFeature.text, "Combinations");
  if (!position) {
    return {
      target: "Combinations",
      hoverMs: null,
      definitionMs: null,
      definition: null,
      checks: [
        {
          name: "cold interactivity fixture contains Combinations",
          pass: false,
          actual: null,
        },
      ],
    };
  }

  const hover = await timedRequest(server, "textDocument/hover", {
    textDocument: { uri: advancedSageFeature.uri },
    position,
  });
  const definition = await timedRequest(server, "textDocument/definition", {
    textDocument: { uri: advancedSageFeature.uri },
    position,
  });
  const definitionTarget = definitionPath(definition.value);

  return {
    target: "Combinations",
    hoverMs: hover.elapsedMs,
    definitionMs: definition.elapsedMs,
    definition: definitionTarget,
    checks: [
      {
        name: `cold hover remains responsive during background index <= ${hoverBudgetMs}ms`,
        pass: hover.elapsedMs <= hoverBudgetMs,
        actual: hover.elapsedMs,
      },
      {
        name: `cold definition remains responsive during background index <= ${definitionBudgetMs}ms`,
        pass: definition.elapsedMs <= definitionBudgetMs,
        actual: definition.elapsedMs,
      },
      {
        name: "cold definition resolves Combinations without waiting for full index",
        pass: normalizePath(definitionTarget).includes("sage/src/sage/combinat/combination.py"),
        actual: definitionTarget,
      },
    ],
  };
}

async function runAdvancedSageScenarios(server, advancedSageFeature) {
  const scenarios = [
    {
      id: "hover-definition-combinations",
      target: "Combinations",
      position: symbolPosition(advancedSageFeature.text, "Combinations"),
      definitionPathIncludes: "sage/src/sage/combinat/combination.py",
      externalReferenceSymbol: "Combinations",
      referenceUsagePathIncludes: "src/07_symbolic_and_combinatorics.sage",
    },
    {
      id: "hover-definition-number-field",
      target: "NumberField",
      position: symbolPosition(advancedSageFeature.text, "NumberField"),
      definitionPathIncludes: "sage/src/sage/rings/number_field/number_field.py",
    },
  ].filter((scenario) => scenario.position);
  const rows = [];
  for (const scenario of scenarios) {
    const hover = await timedRequest(server, "textDocument/hover", {
      textDocument: { uri: advancedSageFeature.uri },
      position: scenario.position,
    });
    const definition = await timedRequest(server, "textDocument/definition", {
      textDocument: { uri: advancedSageFeature.uri },
      position: scenario.position,
    });
    const declaration = await timedRequest(server, "textDocument/declaration", {
      textDocument: { uri: advancedSageFeature.uri },
      position: scenario.position,
    });
    const implementation = await timedRequest(server, "textDocument/implementation", {
      textDocument: { uri: advancedSageFeature.uri },
      position: scenario.position,
    });
    const externalReferences = scenario.externalReferenceSymbol
      ? await runExternalDefinitionReferenceScenario(server, scenario, definitionPath(definition.value))
      : null;
    const checks = [
      {
        name: `hover <= ${hoverBudgetMs}ms`,
        pass: hover.elapsedMs <= hoverBudgetMs,
        actual: hover.elapsedMs,
      },
      {
        name: `definition <= ${definitionBudgetMs}ms`,
        pass: definition.elapsedMs <= definitionBudgetMs,
        actual: definition.elapsedMs,
      },
      {
        name: `definition path includes ${scenario.definitionPathIncludes}`,
        pass: normalizePath(definitionPath(definition.value)).includes(scenario.definitionPathIncludes),
        actual: definitionPath(definition.value),
      },
      {
        name: `declaration <= ${definitionBudgetMs}ms`,
        pass: declaration.elapsedMs <= definitionBudgetMs,
        actual: declaration.elapsedMs,
      },
      {
        name: `declaration path includes ${scenario.definitionPathIncludes}`,
        pass: normalizePath(definitionPath(declaration.value)).includes(scenario.definitionPathIncludes),
        actual: definitionPath(declaration.value),
      },
      {
        name: `implementation <= ${definitionBudgetMs}ms`,
        pass: implementation.elapsedMs <= definitionBudgetMs,
        actual: implementation.elapsedMs,
      },
      {
        name: `implementation path includes ${scenario.definitionPathIncludes}`,
        pass: normalizePath(definitionPath(implementation.value)).includes(scenario.definitionPathIncludes),
        actual: definitionPath(implementation.value),
      },
      ...(externalReferences?.checks ?? []),
    ];
    rows.push({
      id: scenario.id,
      target: scenario.target,
      hoverMs: hover.elapsedMs,
      definitionMs: definition.elapsedMs,
      declarationMs: declaration.elapsedMs,
      implementationMs: implementation.elapsedMs,
      externalReferences,
      definition: definitionPath(definition.value),
      declaration: definitionPath(declaration.value),
      implementation: definitionPath(implementation.value),
      status: rowPassed(scenario, hover, definition, implementation, declaration)
        && checks.every((check) => check.pass)
        ? "pass"
        : "fail",
      checks,
    });
  }
  return rows;
}

async function runExternalDefinitionReferenceScenario(server, scenario, definitionFile) {
  if (!definitionFile || !fsSync.existsSync(definitionFile)) {
    return {
      elapsedMs: null,
      count: 0,
      paths: [],
      checks: [
        {
          name: "external definition references have a readable definition file",
          pass: false,
          actual: definitionFile,
        },
      ],
    };
  }
  const source = await fs.readFile(definitionFile, "utf8");
  const position = symbolPositionInLine(
    source,
    scenario.externalReferenceSymbol,
    `def ${scenario.externalReferenceSymbol}`,
  ) ?? symbolPosition(source, scenario.externalReferenceSymbol);
  if (!position) {
    return {
      elapsedMs: null,
      count: 0,
      paths: [],
      checks: [
        {
          name: `external definition contains ${scenario.externalReferenceSymbol}`,
          pass: false,
          actual: definitionFile,
        },
      ],
    };
  }
  const references = await timedRequest(server, "textDocument/references", {
    textDocument: { uri: pathToFileURL(definitionFile).toString() },
    position,
    context: { includeDeclaration: true },
  });
  const paths = referencePaths(references.value);
  const normalizedPaths = paths.map(normalizePath);
  return {
    elapsedMs: references.elapsedMs,
    count: paths.length,
    paths,
    checks: [
      {
        name: `external definition references <= ${referenceBudgetMs}ms`,
        pass: references.elapsedMs <= referenceBudgetMs,
        actual: references.elapsedMs,
      },
      {
        name: "external definition references include declaration",
        pass: normalizedPaths.some((item) => item.includes(scenario.definitionPathIncludes)),
        actual: paths,
      },
      {
        name: "external definition references include open workspace usage",
        pass: normalizedPaths.some((item) => item.includes(scenario.referenceUsagePathIncludes)),
        actual: paths,
      },
    ],
  };
}

async function runObjectMethodScenarios(server, objectFeature) {
  const scenarios = [
    {
      id: "hover-definition-graph-vertices",
      target: ".vertices",
      position: memberPosition(objectFeature.text, "vertices"),
      definitionPathIncludes: "sage/src/sage/graphs/generic_graph.py",
    },
    {
      id: "hover-definition-elliptic-points",
      target: ".points",
      position: memberPosition(objectFeature.text, "points"),
      definitionPathIncludes: "sage/src/sage/schemes/elliptic_curves/ell_finite_field.py",
    },
    {
      id: "hover-definition-number-field-ring-of-integers",
      target: ".ring_of_integers",
      position: memberPosition(objectFeature.text, "ring_of_integers"),
      definitionPathIncludes: "sage/src/sage/rings/number_field/number_field_base.pyx",
    },
  ].filter((scenario) => scenario.position);
  const rows = [];
  for (const scenario of scenarios) {
    const hover = await timedRequest(server, "textDocument/hover", {
      textDocument: { uri: objectFeature.uri },
      position: scenario.position,
    });
    const definition = await timedRequest(server, "textDocument/definition", {
      textDocument: { uri: objectFeature.uri },
      position: scenario.position,
    });
    const declaration = await timedRequest(server, "textDocument/declaration", {
      textDocument: { uri: objectFeature.uri },
      position: scenario.position,
    });
    const implementation = await timedRequest(server, "textDocument/implementation", {
      textDocument: { uri: objectFeature.uri },
      position: scenario.position,
    });
    rows.push({
      id: scenario.id,
      target: scenario.target,
      hoverMs: hover.elapsedMs,
      definitionMs: definition.elapsedMs,
      declarationMs: declaration.elapsedMs,
      implementationMs: implementation.elapsedMs,
      definition: definitionPath(definition.value),
      declaration: definitionPath(declaration.value),
      implementation: definitionPath(implementation.value),
      status: rowPassed(scenario, hover, definition, implementation, declaration) ? "pass" : "fail",
      checks: [
        {
          name: `hover <= ${hoverBudgetMs}ms`,
          pass: hover.elapsedMs <= hoverBudgetMs,
          actual: hover.elapsedMs,
        },
        {
          name: `definition <= ${definitionBudgetMs}ms`,
          pass: definition.elapsedMs <= definitionBudgetMs,
          actual: definition.elapsedMs,
        },
        {
          name: `definition path includes ${scenario.definitionPathIncludes}`,
          pass: normalizePath(definitionPath(definition.value)).includes(scenario.definitionPathIncludes),
          actual: definitionPath(definition.value),
        },
        {
          name: `declaration <= ${definitionBudgetMs}ms`,
          pass: declaration.elapsedMs <= definitionBudgetMs,
          actual: declaration.elapsedMs,
        },
        {
          name: `declaration path includes ${scenario.definitionPathIncludes}`,
          pass: normalizePath(definitionPath(declaration.value)).includes(scenario.definitionPathIncludes),
          actual: definitionPath(declaration.value),
        },
        {
          name: `implementation <= ${definitionBudgetMs}ms`,
          pass: implementation.elapsedMs <= definitionBudgetMs,
          actual: implementation.elapsedMs,
        },
        {
          name: `implementation path includes ${scenario.definitionPathIncludes}`,
          pass: normalizePath(definitionPath(implementation.value)).includes(scenario.definitionPathIncludes),
          actual: definitionPath(implementation.value),
        },
      ],
    });
  }
  return rows;
}

async function runObjectTypeDefinitionScenarios(server, objectFeature) {
  const scenarios = [
    {
      id: "type-definition-graph-variable",
      target: "graph",
      position: ownerPositionBeforeMember(objectFeature.text, "graph", "vertices"),
      definitionPathIncludes: "sage/src/sage/graphs/graph.py",
    },
    {
      id: "type-definition-elliptic-curve-variable",
      target: "finite_curve",
      position: ownerPositionBeforeMember(objectFeature.text, "finite_curve", "points"),
      definitionPathIncludes: "sage/src/sage/schemes/elliptic_curves/constructor.py",
    },
    {
      id: "type-definition-number-field-variable",
      target: "field",
      position: ownerPositionBeforeMember(objectFeature.text, "field", "ring_of_integers"),
      definitionPathIncludes: "sage/src/sage/rings/number_field/number_field.py",
    },
  ].filter((scenario) => scenario.position);
  const rows = [];
  for (const scenario of scenarios) {
    const definition = await timedRequest(server, "textDocument/typeDefinition", {
      textDocument: { uri: objectFeature.uri },
      position: scenario.position,
    });
    rows.push({
      id: scenario.id,
      target: scenario.target,
      typeDefinitionMs: definition.elapsedMs,
      definition: definitionPath(definition.value),
      status: normalizePath(definitionPath(definition.value)).includes(scenario.definitionPathIncludes)
        && definition.elapsedMs <= typeDefinitionBudgetMs
        ? "pass"
        : "fail",
      checks: [
        {
          name: `type definition <= ${typeDefinitionBudgetMs}ms`,
          pass: definition.elapsedMs <= typeDefinitionBudgetMs,
          actual: definition.elapsedMs,
        },
        {
          name: `type definition path includes ${scenario.definitionPathIncludes}`,
          pass: normalizePath(definitionPath(definition.value)).includes(scenario.definitionPathIncludes),
          actual: definitionPath(definition.value),
        },
      ],
    });
  }
  return rows;
}

function optionalChecks(condition, checks) {
  return condition ? checks : [];
}

function buildScenarios(text) {
  const scenarios = [
    {
      id: "hover-definition-polynomial-ring",
      target: "PolynomialRing",
      position: symbolPositionInLine(text, "PolynomialRing", "ring = "),
      definitionPathIncludes: "sage/src/sage/rings/polynomial/polynomial_ring_constructor.py",
    },
    {
      id: "hover-definition-matrix-rank",
      target: ".rank",
      position: memberPosition(text, "rank"),
      definitionPathIncludes: "sage/src/sage/matrix/matrix0.pyx",
    },
  ];
  return scenarios.filter((scenario) => scenario.position);
}

function symbolPosition(text, symbol) {
  for (const [lineIndex, line] of text.split(/\r?\n/).entries()) {
    const character = line.indexOf(symbol);
    if (character >= 0) {
      return { line: lineIndex, character };
    }
  }
  return null;
}

function symbolPositionInLine(text, symbol, lineNeedle) {
  for (const [lineIndex, line] of text.split(/\r?\n/).entries()) {
    if (!line.includes(lineNeedle)) {
      continue;
    }
    const character = line.indexOf(symbol);
    if (character >= 0) {
      return { line: lineIndex, character };
    }
  }
  return symbolPosition(text, symbol);
}

function memberPosition(text, member) {
  const needle = `.${member}`;
  for (const [lineIndex, line] of text.split(/\r?\n/).entries()) {
    const start = line.indexOf(needle);
    if (start >= 0) {
      return { line: lineIndex, character: start + 1 };
    }
  }
  return null;
}

function ownerPositionBeforeMember(text, owner, member) {
  const needle = `${owner}.${member}`;
  for (const [lineIndex, line] of text.split(/\r?\n/).entries()) {
    const start = line.indexOf(needle);
    if (start >= 0) {
      return { line: lineIndex, character: start };
    }
  }
  return null;
}

function memberCompletionPosition(text, member, prefix) {
  const needle = `.${member}`;
  for (const [lineIndex, line] of text.split(/\r?\n/).entries()) {
    const start = line.indexOf(needle);
    if (start >= 0) {
      return { line: lineIndex, character: start + 1 + prefix.length };
    }
  }
  return null;
}

function prefixPosition(text, prefix) {
  for (const [lineIndex, line] of text.split(/\r?\n/).entries()) {
    const start = line.indexOf(prefix);
    if (start >= 0) {
      return { line: lineIndex, character: start + prefix.length };
    }
  }
  return null;
}

function callArgumentPosition(text, callee) {
  const needle = `${callee}(`;
  for (const [lineIndex, line] of text.split(/\r?\n/).entries()) {
    if (line.trimStart().startsWith("def ")) {
      continue;
    }
    const start = line.indexOf(needle);
    if (start >= 0) {
      return { line: lineIndex, character: start + needle.length };
    }
  }
  return null;
}

function completionItemsFromResult(value) {
  return Array.isArray(value) ? value : value?.items ?? [];
}

function completionLabel(item) {
  if (typeof item?.label === "string") {
    return item.label;
  }
  return item?.label?.label ?? String(item?.label ?? "");
}

function completionDocumentationText(item) {
  const documentation = item?.documentation;
  if (typeof documentation === "string") {
    return documentation;
  }
  return documentation?.value ?? "";
}

function hasCompletionDetail(item) {
  return Boolean(item?.detail) || Boolean(completionDocumentationText(item));
}

function completionSummary(item) {
  if (!item) {
    return null;
  }
  const docs = completionDocumentationText(item);
  return {
    label: completionLabel(item),
    detail: item.detail ?? null,
    documentation: docs ? docs.slice(0, 160) : null,
  };
}

function codeActionTitlesFromResult(value) {
  return (Array.isArray(value) ? value : []).map((action) => action?.title ?? action?.command?.title ?? "");
}

function documentHighlightLinesFromResult(value) {
  return (Array.isArray(value) ? value : [])
    .map((highlight) => highlight?.range?.start?.line)
    .filter((line) => typeof line === "number");
}

function selectionRangeChainFromResult(value) {
  const ranges = [];
  let current = value;
  while (current?.range) {
    ranges.push(current.range);
    current = current.parent;
  }
  return ranges;
}

function flattenDocumentSymbolNames(value) {
  const names = [];
  const visit = (symbols) => {
    for (const symbol of Array.isArray(symbols) ? symbols : []) {
      if (typeof symbol?.name === "string") {
        names.push(symbol.name);
      }
      visit(symbol?.children);
    }
  };
  visit(value);
  return names;
}

function hasNestedDocumentSymbol(value, parentName, childName) {
  for (const symbol of Array.isArray(value) ? value : []) {
    if (
      symbol?.name === parentName
      && Array.isArray(symbol.children)
      && symbol.children.some((child) => child?.name === childName)
    ) {
      return true;
    }
    if (hasNestedDocumentSymbol(symbol?.children, parentName, childName)) {
      return true;
    }
  }
  return false;
}

function semanticTokenDataLength(value) {
  const data = Array.isArray(value?.data) ? value.data : [];
  return data.length;
}

function signatureHelpLabel(value) {
  return value?.signatures?.[value?.activeSignature ?? 0]?.label ?? "";
}

function signatureHelpParameterCount(value) {
  const signature = value?.signatures?.[value?.activeSignature ?? 0];
  return Array.isArray(signature?.parameters) ? signature.parameters.length : 0;
}

function signatureHelpActiveParameter(value) {
  const signature = value?.signatures?.[value?.activeSignature ?? 0];
  const active = signature?.activeParameter ?? value?.activeParameter;
  return typeof active === "number" ? active : null;
}

function callHierarchyIncomingNames(value) {
  return (Array.isArray(value) ? value : [])
    .map((call) => call?.from?.name)
    .filter((name) => typeof name === "string");
}

function callHierarchyOutgoingNames(value) {
  return (Array.isArray(value) ? value : [])
    .map((call) => call?.to?.name)
    .filter((name) => typeof name === "string");
}

function documentLinkTargetsFromResult(value) {
  return (Array.isArray(value) ? value : [])
    .map((link) => link?.target)
    .filter((target) => typeof target === "string")
    .map((target) => {
      try {
        return fileURLToPath(target);
      } catch {
        return target;
      }
    });
}

async function waitForIndex(server) {
  const started = performance.now();
  let lastStatus = null;
  while (performance.now() - started < 15000) {
    lastStatus = await server.request("workspace/executeCommand", {
      command: "sage.__rust.indexStatus",
      arguments: [],
    });
    if ((lastStatus?.symbol_count ?? 0) > 0 && (lastStatus?.pending_jobs ?? 0) === 0) {
      return lastStatus;
    }
    await delay(200);
  }
  throw new Error(`sage-ls index did not become ready: ${JSON.stringify(lastStatus)}`);
}

async function timedRequest(server, method, params) {
  const started = performance.now();
  const value = await server.request(method, params);
  return {
    elapsedMs: Math.round(performance.now() - started),
    value,
  };
}

function rowPassed(scenario, hover, definition, implementation, declaration) {
  return hover.elapsedMs <= hoverBudgetMs
    && definition.elapsedMs <= definitionBudgetMs
    && normalizePath(definitionPath(definition.value)).includes(scenario.definitionPathIncludes)
    && (!declaration
      || (
        declaration.elapsedMs <= definitionBudgetMs
        && normalizePath(definitionPath(declaration.value)).includes(scenario.definitionPathIncludes)
      ))
    && (!implementation
      || (
        implementation.elapsedMs <= definitionBudgetMs
        && normalizePath(definitionPath(implementation.value)).includes(scenario.definitionPathIncludes)
      ));
}

function definitionPath(value) {
  const location = Array.isArray(value) ? value[0] : value;
  if (!location?.uri) {
    return "";
  }
  try {
    return fileURLToPath(location.uri);
  } catch {
    return location.uri;
  }
}

function referencePaths(value) {
  if (!Array.isArray(value)) {
    return [];
  }
  return value
    .map((location) => {
      if (!location?.uri) {
        return "";
      }
      try {
        return fileURLToPath(location.uri);
      } catch {
        return location.uri;
      }
    })
    .filter(Boolean);
}

function normalizePath(value) {
  return String(value).replaceAll(path.sep, "/");
}

function inlayLabel(value) {
  if (typeof value?.label === "string") {
    return value.label;
  }
  if (Array.isArray(value?.label)) {
    return value.label.map((part) => part.value ?? "").join("");
  }
  return "";
}

function numberFromEnv(name, fallback) {
  const raw = process.env[name];
  if (!raw) {
    return fallback;
  }
  const parsed = Number(raw);
  return Number.isFinite(parsed) ? parsed : fallback;
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

function delay(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function cachePathIsInsideSmokeRoot(cachePath) {
  if (typeof cachePath !== "string" || cachePath.length === 0) {
    return false;
  }
  const absoluteCachePath = path.resolve(cachePath);
  return [cacheDir, rustCacheDir, path.join(rustCacheDir, "xdg-cache")].some((root) => {
    const relative = path.relative(path.resolve(root), absoluteCachePath);
    return relative === "" || (!relative.startsWith("..") && !path.isAbsolute(relative));
  });
}

class LspProcess {
  constructor(command) {
    this.command = command;
    this.nextId = 1;
    this.pending = new Map();
    this.buffer = Buffer.alloc(0);
  }

  async start() {
    this.child = spawn(this.command, [], {
      cwd: repositoryRoot,
      env: {
        ...process.env,
        XDG_CACHE_HOME: path.join(rustCacheDir, "xdg-cache"),
      },
      stdio: ["pipe", "pipe", "pipe"],
    });
    this.child.stdout.on("data", (chunk) => this.handleData(chunk));
    this.child.stderr.on("data", (chunk) => {
      if (process.env.SAGE_LSP_SMOKE_DEBUG === "1") {
        process.stderr.write(chunk);
      }
    });
    this.child.on("exit", (code, signal) => {
      const error = new Error(`sage-ls exited with code ${code} signal ${signal}`);
      for (const pending of this.pending.values()) {
        pending.reject(error);
      }
      this.pending.clear();
    });
  }

  request(method, params) {
    const id = this.nextId++;
    this.write({ jsonrpc: "2.0", id, method, params });
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
    });
  }

  notify(method, params) {
    this.write({ jsonrpc: "2.0", method, params });
  }

  write(message) {
    const json = JSON.stringify(message);
    this.child.stdin.write(`Content-Length: ${Buffer.byteLength(json, "utf8")}\r\n\r\n${json}`);
  }

  handleData(chunk) {
    this.buffer = Buffer.concat([this.buffer, chunk]);
    while (true) {
      const headerEnd = this.buffer.indexOf("\r\n\r\n");
      if (headerEnd < 0) {
        return;
      }
      const header = this.buffer.slice(0, headerEnd).toString("ascii");
      const lengthMatch = /Content-Length:\s*(\d+)/i.exec(header);
      if (!lengthMatch) {
        throw new Error(`missing Content-Length header: ${header}`);
      }
      const length = Number(lengthMatch[1]);
      const messageStart = headerEnd + 4;
      const messageEnd = messageStart + length;
      if (this.buffer.length < messageEnd) {
        return;
      }
      const raw = this.buffer.slice(messageStart, messageEnd).toString("utf8");
      this.buffer = this.buffer.slice(messageEnd);
      this.handleMessage(JSON.parse(raw));
    }
  }

  handleMessage(message) {
    if (!Object.hasOwn(message, "id")) {
      return;
    }
    const pending = this.pending.get(message.id);
    if (!pending) {
      return;
    }
    this.pending.delete(message.id);
    if (message.error) {
      pending.reject(new Error(JSON.stringify(message.error)));
    } else {
      pending.resolve(message.result);
    }
  }

  stop() {
    if (this.child && !this.child.killed) {
      this.child.kill();
    }
  }
}

await runSmoke();
