import test from "node:test";
import assert from "node:assert/strict";

import {
  buildLanguageServerLaunch,
  resolveLocalRustLanguageServer,
  resolvePackagedRustLanguageServer,
} from "../src/serverLaunch";
import { buildInitializationOptions, type SageSettings } from "../src/settingsModel";

test("buildInitializationOptions mirrors editor settings and workspace context into the LSP payload", () => {
  const settings: SageSettings = {
    interpreterPath: "/opt/sage/bin/sage",
    interpreterArgs: ["--python"],
    languageServerRustPath: "auto",
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

test("resolveLocalRustLanguageServer finds debug before release builds", () => {
  assert.equal(resolveLocalRustLanguageServer(
    {
      repositoryRoot: "/workspace/plugin",
      exists: (candidate) => candidate === "/workspace/plugin/target/debug/sage-ls",
    },
    "linux",
  ), "/workspace/plugin/target/debug/sage-ls");
});
