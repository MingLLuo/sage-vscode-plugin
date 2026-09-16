#!/usr/bin/env node
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { LspProcess } from "./lib/lsp-process.mjs";

const repositoryRoot = fileURLToPath(new URL("../", import.meta.url));
const root = await fs.mkdtemp(path.join(os.tmpdir(), "sage-docs-smoke-"));
const server = new LspProcess(process.env.SAGE_LS_BINARY
  ?? path.join(repositoryRoot, "target/debug/sage-ls"));
const uri = pathToFileURL(path.join(root, "docs.sage")).href;
const position = { line: 0, character: 1 };
const status = () => server.requestWithTimeout("workspace/executeCommand", {
  command: "sage.__rust.docsStatus", arguments: [],
}, 2000);

try {
  await server.start();
  await server.requestWithTimeout("initialize", {
    processId: process.pid,
    rootUri: pathToFileURL(root).href,
    capabilities: {},
    initializationOptions: {
      interpreter: { path: process.env.SAGE_RUNTIME ?? "sage" },
      analysis: { enableRuntimeIntrospection: true, sourceRoots: [root] },
      workspace: { folders: [root] },
      documentation: { preferredSource: "auto", showOnHover: true },
      rust: { cacheDir: path.join(root, "cache") },
    },
  }, 10000);
  server.notify("initialized", {});
  server.notify("textDocument/didOpen", {
    textDocument: { uri, languageId: "sage", version: 1, text: "sin(x)\n" },
  });
  const started = performance.now();
  const cold = await server.requestWithTimeout("textDocument/hover", {
    textDocument: { uri }, position,
  }, 2000);
  const coldHoverMs = performance.now() - started;
  assert.ok(cold, "cold hover should retain static information");
  assert.match(cold.contents.value, /Loading Sage documentation|The sine function/);
  assert.ok(coldHoverMs < 1000, "Sage startup must not block hover");
  let hover;
  const deadline = performance.now() + 15000;
  do {
    await new Promise((resolve) => setTimeout(resolve, 100));
    hover = await server.requestWithTimeout("textDocument/hover", {
      textDocument: { uri }, position,
    }, 2000);
    if (hover?.contents?.value?.includes("The sine function")) break;
  } while (performance.now() < deadline);
  assert.match(hover?.contents?.value ?? "", /The sine function/,
    `hover must load real docs without first opening the docs panel: ${JSON.stringify(await status())}`);
  const docs = await server.requestWithTimeout("workspace/executeCommand", {
    command: "sage.__rust.getDocumentation",
    arguments: [{ textDocument: { uri }, position }],
  }, 10000);
  assert.match(docs.docstring, /The sine function/);
  assert.match(docs.docstring, /EXAMPLES/);
  assert.ok(!docs.docstring.includes("Runtime documentation worker can provide"));
  const runtime = await status();
  assert.equal(runtime.runtime_timeout_count, 0);
  assert.equal(runtime.runtime_degraded_reason, null);
  console.log(JSON.stringify({ status: "passed", coldHoverMs, summary: docs.summary,
    docstringLength: docs.docstring.length, runtime }, null, 2));
  await server.requestWithTimeout("shutdown", undefined, 2000);
  server.notify("exit");
} catch (error) {
  console.error(server.stderrText());
  throw error;
} finally {
  await server.terminateAndWait();
  await fs.rm(root, { recursive: true, force: true });
}
