import test from "node:test";
import assert from "node:assert/strict";

import { buildLanguageServerLaunch } from "../src/serverLaunch";
import { buildInitializationOptions, type SageSettings } from "../src/settingsModel";

test("buildInitializationOptions mirrors editor settings and workspace context into the LSP payload", () => {
  const settings: SageSettings = {
    interpreterPath: "/opt/sage/bin/sage",
    interpreterArgs: ["--python"],
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
    notebookSupportEnabled: true,
  };

  assert.deepEqual(
    buildInitializationOptions(settings, {
      rootUri: "file:///workspace",
      folders: ["file:///workspace"],
      sourceRoots: ["file:///workspace/src"],
    }),
    {
      interpreter: {
        path: "/opt/sage/bin/sage",
        args: ["--python"],
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

test("buildLanguageServerLaunch uses python executables directly", () => {
  assert.deepEqual(buildLanguageServerLaunch("/opt/python/bin/python3", ["-X", "utf8"]), {
    command: "/opt/python/bin/python3",
    args: ["-X", "utf8", "-m", "sage_lsp"],
  });
});

test("buildLanguageServerLaunch adds -python for Sage executables", () => {
  assert.deepEqual(buildLanguageServerLaunch("/opt/sage/bin/sage", ["--nodotsage"]), {
    command: "/opt/sage/bin/sage",
    args: ["--nodotsage", "-python", "-m", "sage_lsp"],
  });
});
