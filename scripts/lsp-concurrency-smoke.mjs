#!/usr/bin/env node
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { LspProcess, withTimeout } from "./lib/lsp-process.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const temp = await fs.mkdtemp(path.join(os.tmpdir(), "sage-concurrency-"));
const file = path.join(temp, "demo.sage");
const uri = pathToFileURL(file).toString();
const source = "def original():\n    return '😀'\noriginal()\n";
let finalDiagnostics;
const diagnostics = new Promise(resolve => { finalDiagnostics = resolve; });
const server = new LspProcess(path.join(root, "target/debug", process.platform === "win32" ? "sage-ls.exe" : "sage-ls"), {
  environment: { ...process.env, SAGE_LS_TEST_BACKGROUND_DELAY_MS: "3000" },
  onNotification(method, params) {
    if (method === "textDocument/publishDiagnostics" && params.uri === uri && params.version === 101) finalDiagnostics(params);
  },
});
try {
  await fs.writeFile(file, source);
  await server.start();
  const workspaceUri = pathToFileURL(temp).toString();
  await server.requestWithTimeout("initialize", {
    processId: process.pid, rootUri: workspaceUri, capabilities: {},
    initializationOptions: {
      analysis: { sourceRoots: [temp], enableDiagnostics: true },
      workspace: { folders: [workspaceUri], exclude: ["**/cache/**"] },
      rust: { cacheDir: path.join(temp, "cache") },
    },
  }, 1500);
  server.notify("initialized", {});
  server.notify("textDocument/didOpen", { textDocument: { uri, languageId: "sagemath", version: 1, text: source } });
  for (let i = 0; i < 12; i++) server.notify("textDocument/didSave", { textDocument: { uri }, text: source });
  const started = performance.now();
  const hover = await server.requestWithTimeout("textDocument/hover", { textDocument: { uri }, position: { line: 2, character: 3 } }, 1500);
  assert.ok(hover, "hover must respond while reconcile is delayed and saves are queued");
  const hoverMs = performance.now() - started;
  for (let version = 2; version <= 101; version++) {
    server.notify("textDocument/didChange", { textDocument: { uri, version }, contentChanges: [{ text: `def updated_${version}():\n    return '😀中'\nupdated_${version}()\n` }] });
  }
  const symbols = await server.requestWithTimeout("textDocument/documentSymbol", { textDocument: { uri } }, 1500);
  assert.equal(symbols[0].name, "updated_101", "the final notification must be visible to subsequent requests");
  const published = await withTimeout(diagnostics, 1500, "versioned debounced diagnostics");
  assert.deepEqual(published.diagnostics, []);
  const cancelled = server.beginRequest("textDocument/references", { textDocument: { uri }, position: { line: 2, character: 3 }, context: { includeDeclaration: true } });
  server.notify("$/cancelRequest", { id: cancelled.id });
  await withTimeout(cancelled.promise.catch(error => error), 1500, "cancelled request settles");
  await server.requestWithTimeout("shutdown", undefined, 1500);
  server.notify("exit"); // deliberately keep stdin open
  assert.equal((await withTimeout(server.exitResult, 1500, "exit with stdin still open")).code, 0);
  console.log(JSON.stringify({ status: "passed", saves: 12, edits: 100, hoverDuringIndexMs: Math.round(hoverMs), diagnosticsVersion: published.version }, null, 2));
} finally {
  await server.terminateAndWait();
  await fs.rm(temp, { force: true, recursive: true });
}

// LSP requires exit status 1 if the client exits without a shutdown request.
const abrupt = new LspProcess(server.command);
try {
  await abrupt.start();
  await abrupt.requestWithTimeout("initialize", { capabilities: {}, processId: process.pid }, 1500);
  abrupt.notify("exit");
  assert.equal((await withTimeout(abrupt.exitResult, 1500, "abrupt protocol exit")).code, 1);
} finally { await abrupt.terminateAndWait(); }
