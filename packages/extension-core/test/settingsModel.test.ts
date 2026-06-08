import test from "node:test";
import assert from "node:assert/strict";

import {
  buildLanguageServerLaunch,
  buildLegacyPythonLanguageServerLaunch,
  resolveDefaultLanguageServerPython,
  resolveLocalRustLanguageServer,
  resolvePackagedRustLanguageServer,
} from "../src/serverLaunch";
import { buildInitializationOptions, type SageSettings } from "../src/settingsModel";

test("buildInitializationOptions mirrors editor settings and workspace context into the LSP payload", () => {
  const settings: SageSettings = {
    interpreterPath: "/opt/sage/bin/sage",
    interpreterArgs: ["--python"],
    languageServerRustPath: "auto",
    languageServerPythonPath: "auto",
    languageServerPythonArgs: [],
    analysisMode: "full",
    extraPaths: ["./stubs"],
    sourceRoots: ["/workspace/src"],
    diagnosticsEnabled: true,
    runtimeIntrospectionEnabled: true,
    enablePyxParsing: true,
    pythonFilesEnabled: true,
    indexingExcludeGlobs: ["**/.venv/**"],
    docsSource: "runtime",
    showDocsOnHover: false,
    loggingLevel: "debug",
    runTarget: "terminal",
    showCellCodeLens: false,
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
      {
        resolvedRustPath: "/workspace/target/debug/sage-ls",
        cacheDir: "/workspace/.vscode-test/globalStorage/rust-index-v2",
        nodePath: "/opt/node/bin/node",
        pyrightServerPath: "/workspace/node_modules/pyright/langserver.index.js",
      },
    ),
    {
      interpreter: {
        path: "/opt/sage/bin/sage",
        args: ["--python"],
      },
      rust: {
        binaryPath: "/workspace/target/debug/sage-ls",
        cacheDir: "/workspace/.vscode-test/globalStorage/rust-index-v2",
      },
      pyright: {
        nodePath: "/opt/node/bin/node",
        serverPath: "/workspace/node_modules/pyright/langserver.index.js",
      },
      analysis: {
        mode: "full",
        extraPaths: ["./stubs"],
        sourceRoots: ["/workspace/src"],
        enableDiagnostics: true,
        enableRuntimeIntrospection: true,
        enablePyxParsing: true,
        enablePythonFiles: true,
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

test("buildLanguageServerLaunch uses an explicit Rust language server path", () => {
  assert.deepEqual(buildLanguageServerLaunch({
    interpreterPath: "/opt/python/bin/python3",
    interpreterArgs: ["-X", "utf8"],
    languageServerRustPath: "/workspace/target/debug/sage-ls",
    languageServerPythonPath: "auto",
    languageServerPythonArgs: [],
  }), {
    command: "/workspace/target/debug/sage-ls",
    args: [],
  });
});

test("buildLanguageServerLaunch prefers the repository-local Rust binary in auto mode", () => {
  assert.deepEqual(buildLanguageServerLaunch({
    interpreterPath: "/opt/sage/bin/sage",
    interpreterArgs: [],
    languageServerRustPath: "auto",
    languageServerPythonPath: "auto",
    languageServerPythonArgs: [],
    repositoryRoot: "/workspace/plugin",
    platform: "linux",
    exists: (candidate) => candidate === "/workspace/plugin/target/debug/sage-ls",
  }), {
    command: "/workspace/plugin/target/debug/sage-ls",
    args: [],
  });
});

test("buildLanguageServerLaunch falls back to sage-ls on PATH", () => {
  assert.deepEqual(buildLanguageServerLaunch({
    interpreterPath: "/opt/sage/bin/sage",
    interpreterArgs: [],
    languageServerRustPath: "auto",
    languageServerPythonPath: "auto",
    languageServerPythonArgs: [],
    repositoryRoot: "/workspace/plugin",
    platform: "linux",
    exists: () => false,
  }), {
    command: "sage-ls",
    args: [],
  });
});

test("buildLanguageServerLaunch uses a packaged Rust binary before PATH fallback", () => {
  assert.deepEqual(buildLanguageServerLaunch({
    interpreterPath: "/opt/sage/bin/sage",
    interpreterArgs: [],
    languageServerRustPath: "auto",
    languageServerPythonPath: "auto",
    languageServerPythonArgs: [],
    extensionPath: "/workspace/plugin/packages/extension-core",
    platform: "darwin",
    arch: "arm64",
    exists: (candidate) => candidate === "/workspace/plugin/packages/extension-core/resources/bin/darwin-arm64/sage-ls",
  }), {
    command: "/workspace/plugin/packages/extension-core/resources/bin/darwin-arm64/sage-ls",
    args: [],
  });
});

test("resolvePackagedRustLanguageServer ignores non-macOS packaged binary paths", () => {
  assert.equal(resolvePackagedRustLanguageServer(
    {
      extensionPath: "/extensions/sage",
      exists: (candidate) => candidate === "/extensions/sage/resources/bin/win32-x64/sage-ls.exe",
    },
    "win32",
    "x64",
  ), undefined);
});

test("buildLegacyPythonLanguageServerLaunch preserves old Python launch behavior", () => {
  assert.deepEqual(buildLegacyPythonLanguageServerLaunch({
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

test("resolveLocalRustLanguageServer finds debug before release builds", () => {
  assert.equal(resolveLocalRustLanguageServer(
    {
      repositoryRoot: "/workspace/plugin",
      exists: (candidate) => candidate === "/workspace/plugin/target/debug/sage-ls",
    },
    "linux",
  ), "/workspace/plugin/target/debug/sage-ls");
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
