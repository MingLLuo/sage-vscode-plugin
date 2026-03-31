import test from "node:test";
import assert from "node:assert/strict";

import { buildLanguageServerLaunch, resolveDefaultLanguageServerPython } from "../src/serverLaunch";
import { buildInitializationOptions, type SageSettings } from "../src/settingsModel";

test("buildInitializationOptions mirrors editor settings and workspace context into the LSP payload", () => {
  const settings: SageSettings = {
    interpreterPath: "/opt/sage/bin/sage",
    interpreterArgs: ["--python"],
    languageServerPythonPath: "auto",
    languageServerPythonArgs: [],
    analysisMode: "full",
    extraPaths: ["./stubs"],
    sourceRoots: ["/workspace/src"],
    diagnosticsEnabled: true,
    runtimeIntrospectionEnabled: true,
    enablePyxParsing: true,
    indexingExcludeGlobs: ["**/.venv/**"],
    docsSource: "runtime",
    showDocsOnHover: false,
    loggingLevel: "debug",
    runTarget: "terminal",
    cleanupGeneratedPython: true,
    notebookSupportEnabled: true,
  };

  assert.deepEqual(
    buildInitializationOptions(
      settings,
      {
        rootUri: "file:///workspace",
        folders: ["file:///workspace"],
        sourceRoots: ["file:///workspace/src"],
      },
      "/opt/python/bin/python3",
    ),
    {
      interpreter: {
        path: "/opt/sage/bin/sage",
        args: ["--python"],
        pythonPath: "/opt/python/bin/python3",
      },
      analysis: {
        mode: "full",
        extraPaths: ["./stubs"],
        sourceRoots: ["/workspace/src"],
        enableDiagnostics: true,
        enableRuntimeIntrospection: true,
        enablePyxParsing: true,
      },
      workspace: {
        rootUri: "file:///workspace",
        folders: ["file:///workspace"],
        sourceRoots: ["file:///workspace/src"],
        exclude: ["**/.venv/**"],
      },
      documentation: {
        preferredSource: "runtime",
        showOnHover: false,
      },
      logging: {
        level: "debug",
      },
      experimental: {
        notebookSupport: true,
      },
    },
  );
});

test("buildLanguageServerLaunch uses python interpreters directly when auto mode is active", () => {
  assert.deepEqual(buildLanguageServerLaunch({
    interpreterPath: "/opt/python/bin/python3",
    interpreterArgs: ["-X", "utf8"],
    languageServerPythonPath: "auto",
    languageServerPythonArgs: [],
  }), {
    command: "/opt/python/bin/python3",
    args: ["-X", "utf8", "-m", "sage_lsp"],
  });
});

test("buildLanguageServerLaunch falls back to a dedicated python runtime for Sage executables", () => {
  assert.deepEqual(buildLanguageServerLaunch({
    interpreterPath: "/opt/sage/bin/sage",
    interpreterArgs: ["--nodotsage"],
    languageServerPythonPath: "auto",
    languageServerPythonArgs: ["-X", "utf8"],
    environment: { CONDA_PREFIX: "/opt/conda" },
    platform: "linux",
    homeDir: "/Users/example",
    exists: (candidate) => candidate === "/opt/conda/bin/python",
  }), {
    command: "/opt/conda/bin/python",
    args: ["-X", "utf8", "-m", "sage_lsp"],
  });
});

test("buildLanguageServerLaunch honors an explicit language server python override", () => {
  assert.deepEqual(buildLanguageServerLaunch({
    interpreterPath: "/opt/sage/bin/sage",
    interpreterArgs: [],
    languageServerPythonPath: "/custom/python",
    languageServerPythonArgs: ["-X", "utf8"],
  }), {
    command: "/custom/python",
    args: ["-X", "utf8", "-m", "sage_lsp"],
  });
});

test("resolveDefaultLanguageServerPython prefers explicit environment overrides", () => {
  assert.equal(
    resolveDefaultLanguageServerPython({ SAGE_LSP_PYTHON: "/override/python" }, "linux"),
    "/override/python",
  );
  assert.equal(
    resolveDefaultLanguageServerPython(
      { VIRTUAL_ENV: "/venv" },
      "linux",
      {
        homeDir: "/Users/example",
        exists: (candidate) => candidate === "/venv/bin/python",
      },
    ),
    "/venv/bin/python",
  );
});

test("resolveDefaultLanguageServerPython prefers the local sage-dev python for checkout runtimes", () => {
  assert.equal(
    resolveDefaultLanguageServerPython(
      {},
      "linux",
      {
        interpreterPath: "/workspace/sage/sage",
        homeDir: "/Users/example",
        exists: (candidate) => candidate === "/workspace/sage/src/bin/sage"
          || candidate === "/Users/example/miniforge3/envs/sage-dev/bin/python",
      },
    ),
    "/Users/example/miniforge3/envs/sage-dev/bin/python",
  );
});
