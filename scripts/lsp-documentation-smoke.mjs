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
async function checkDefinition() {
  const definition = await server.requestWithTimeout("textDocument/definition", {
    textDocument: { uri }, position,
  }, 15000);
  assert.ok(definition?.uri, "sin should have a definition even without a Sage source index");
  assert.match(definition.uri, /sage\/functions\/trig\.py$/);
  const source = await fs.readFile(fileURLToPath(definition.uri), "utf8");
  const line = source.split(/\r?\n/)[definition.range.start.line];
  assert.match(line, /class Function_sin\b/);
  return { uri: definition.uri, line: definition.range.start.line + 1 };
}
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
  if (process.argv.includes("--definition-first")) await checkDefinition();
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
  const definition = await checkDefinition();
  // These globals are deliberately absent from the static built-in catalog.
  // Exercise both implicit Sage exports and an explicit import alias without
  // any Sage source roots, so runtime fallback is required.
  const runtimeSymbols = [];
  for (const [index, symbol] of [
    "RealBallField", "ComplexBallField", "RealIntervalField", "ComplexIntervalField",
  ].entries()) {
    const imported = index % 2 === 1;
    const text = imported
      ? `from sage.all import ${symbol} as Constructor\nConstructor(100)\n`
      : index === 0 ? "Reals = RealBallField()\n" : `${symbol}(100)\n`;
    const targetPosition = {
      line: imported ? 1 : 0,
      character: index === 0 ? text.indexOf(symbol) + 1 : 1,
    };
    server.notify("textDocument/didChange", {
      textDocument: { uri, version: index + 2 }, contentChanges: [{ text }],
    });
    if (index === 0) {
      const firstLocation = await server.requestWithTimeout("textDocument/definition", {
        textDocument: { uri }, position: targetPosition,
      }, 10000);
      assert.ok(firstLocation?.uri, `${symbol} must navigate before opening documentation`);
    }
    const result = await server.requestWithTimeout("workspace/executeCommand", {
      command: "sage.__rust.getDocumentation",
      arguments: [{ textDocument: { uri }, position: targetPosition }],
    }, 10000);
    assert.ok(result?.docstring?.length > 100, `${symbol} must load full runtime docs`);
    assert.ok(!result.docstring.includes("Runtime documentation worker can provide"));
    const location = await server.requestWithTimeout("textDocument/definition", {
      textDocument: { uri }, position: targetPosition,
    }, 10000);
    assert.ok(location?.uri, `${symbol} must resolve a source location`);
    const hover = await server.requestWithTimeout("textDocument/hover", {
      textDocument: { uri }, position: targetPosition,
    }, 2000);
    assert.ok(hover?.contents?.value, `${symbol} must show hover documentation`);
    runtimeSymbols.push({ symbol, uri: location.uri });
  }
  const runtime = await status();
  assert.equal(runtime.runtime_timeout_count, 0);
  assert.equal(runtime.runtime_degraded_reason, null);
  console.log(JSON.stringify({ status: "passed", coldHoverMs, summary: docs.summary,
    docstringLength: docs.docstring.length, definition, runtimeSymbols, runtime }, null, 2));
  await server.requestWithTimeout("shutdown", undefined, 2000);
  server.notify("exit");
} catch (error) {
  console.error(server.stderrText());
  throw error;
} finally {
  await server.terminateAndWait();
  await fs.rm(root, { recursive: true, force: true });
}
