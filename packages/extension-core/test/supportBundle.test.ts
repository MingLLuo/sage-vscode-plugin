import test from "node:test";
import assert from "node:assert/strict";

import { buildSupportBundle } from "../src/supportBundle";

test("buildSupportBundle records troubleshootable state without source contents", () => {
  const bundleText = buildSupportBundle({
    generatedAt: "2026-05-24T00:00:00.000Z",
    extension: {
      id: "sage-vscode.sage-vscode-extension",
      version: "0.1.0",
    },
    host: {
      vscodeVersion: "1.97.0",
      platform: "darwin",
      arch: "arm64",
      nodeVersion: "v22.0.0",
    },
    workspace: {
      folders: ["/workspace"],
      trusted: true,
      hasVirtualWorkspace: false,
    },
    activeDocument: {
      uri: "file:///workspace/example.sage",
      languageId: "sagemath",
      scheme: "file",
    },
    settings: {
      interpreterPath: "sage",
      interpreterArgs: [],
      languageServerRustPath: "auto",
      languageServerPythonPath: "auto",
      languageServerPythonArgs: [],
      analysisMode: "default",
      extraPaths: ["src"],
      sourceRoots: ["/workspace/sage/src"],
      diagnosticsEnabled: true,
      runtimeIntrospectionEnabled: true,
      enablePyxParsing: true,
      pythonFilesEnabled: true,
      indexingExcludeGlobs: ["**/.git/**"],
      docsSource: "auto",
      showDocsOnHover: true,
      loggingLevel: "info",
      runTarget: "terminal",
      showCellCodeLens: true,
      cleanupGeneratedPython: false,
      notebookSupportEnabled: false,
    },
    environment: {
      interpreterPath: "sage",
      languageServerPath: "/workspace/target/debug/sage-ls",
      languageServerEngine: "rust-v2",
      analysisMode: "default",
      docsSource: "auto",
      sourceRoots: ["/workspace/sage/src"],
      extraPaths: ["src"],
      indexMode: "deferred Sage roots with eager workspace roots",
      runtimeIntrospectionEnabled: true,
      enablePyxParsing: true,
      pythonFilesEnabled: true,
      workspaceRuntimeState: {
        trusted: true,
        hasVirtualWorkspace: false,
      },
      indexStatus: {
        indexed_file_count: 12,
        symbol_count: 34,
        doc_count: 5,
      },
      docsStatus: {
        offline_doc_count: 5,
        runtime_worker_state: "ready",
      },
    },
    lifecycle: {
      launchCount: 1,
      hasClient: true,
    },
  });

  const bundle = JSON.parse(bundleText);
  assert.equal(bundle.schema_version, 1);
  assert.equal(bundle.privacy.includes_source_contents, false);
  assert.equal(bundle.privacy.includes_selected_text, false);
  assert.equal(bundle.privacy.includes_environment_variables, false);
  assert.equal(bundle.privacy.includes_paths_and_settings, true);
  assert.equal(bundle.extension.id, "sage-vscode.sage-vscode-extension");
  assert.equal(bundle.settings.enable_python_files, true);
  assert.equal(bundle.settings.show_cell_code_lens, true);
  assert.equal(bundle.environment.language_server_engine, "rust-v2");
  assert.equal(bundle.index_status.indexed_file_count, 12);
  assert.equal(bundle.docs_status.runtime_worker_state, "ready");
  assert.deepEqual(bundle.language_client_lifecycle, {
    launchCount: 1,
    hasClient: true,
  });
});
