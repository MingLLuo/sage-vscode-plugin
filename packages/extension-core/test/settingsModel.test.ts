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

test("buildLanguageServerLaunch uses repository-local Rust binaries in development mode", () => {
  assert.deepEqual(buildLanguageServerLaunch({
    interpreterPath: "/opt/sage/bin/sage",
    interpreterArgs: [],
    languageServerRustPath: "auto",
    repositoryRoot: "/workspace/plugin",
    developmentMode: true,
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

test("resolvePackagedRustLanguageServer resolves Windows executables", () => {
  assert.equal(resolvePackagedRustLanguageServer(
    {
      extensionPath: "/extensions/sage",
      exists: (candidate) => candidate === "/extensions/sage/resources/bin/win32-x64/sage-ls.exe",
    },
    "win32",
    "x64",
  ), "/extensions/sage/resources/bin/win32-x64/sage-ls.exe");
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

for (const platform of ["darwin", "linux", "win32"] as const) {
  for (const arch of ["arm64", "x64"]) {
    test(`packaged ${platform}-${arch} binary wins over workspace build artifacts`, () => {
      const executable = platform === "win32" ? "sage-ls.exe" : "sage-ls";
      assert.equal(buildLanguageServerLaunch({
        interpreterPath: "sage", interpreterArgs: [], languageServerRustPath: "auto",
        extensionPath: "/extensions/sage", repositoryRoot: "/workspace", developmentMode: true,
        environment: {}, platform, arch, exists: () => true,
      }).command, `/extensions/sage/resources/bin/${platform}-${arch}/${executable}`);
    });
  }
}

test("production does not run a workspace target/debug executable", () => {
  assert.equal(buildLanguageServerLaunch({
    interpreterPath: "sage", interpreterArgs: [], languageServerRustPath: "auto",
    repositoryRoot: "/workspace", environment: {}, platform: "linux", exists: () => true,
  }).command, "sage-ls");
});

test("an explicit Rust path setting wins over the SAGE_LS_PATH environment override", () => {
  assert.equal(buildLanguageServerLaunch({
    interpreterPath: "sage",
    interpreterArgs: [],
    languageServerRustPath: "/configured/sage-ls",
    environment: { SAGE_LS_PATH: "/from/environment/sage-ls" },
  }).command, "/configured/sage-ls");
  assert.equal(buildLanguageServerLaunch({
    interpreterPath: "sage",
    interpreterArgs: [],
    languageServerRustPath: "auto",
    environment: { SAGE_LS_PATH: "/from/environment/sage-ls" },
  }).command, "/from/environment/sage-ls");
});
