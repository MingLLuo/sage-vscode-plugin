#!/usr/bin/env node
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { LspProcess } from "./lib/lsp-process.mjs";

const repositoryRoot = fileURLToPath(new URL("../", import.meta.url));
const root = await fs.mkdtemp(path.join(os.tmpdir(), "sage-unicode-rename-"));
const server = new LspProcess(path.join(repositoryRoot, "target", "debug", "sage-ls"));
const sources = [
  ["origin.py", "def 已有():\n    return 0\n\ndef 变量(x):\n    return x\n\nprint('🙂'); 结果 = 变量(3)\n", { line: 3, character: 5 }],
  ["unicode.sage", "def 已有():\n    return 0\n\ndef 变量(x):\n    return x\n\n结果 = 变量(3)\n", { line: 3, character: 5 }],
];
const consumer = "from origin import 变量\n结果 = 变量(3)\n";
const preparser = "R.<变量> = PolynomialRing(QQ)\n结果 = 变量^2\n";

try {
  for (const [name, source] of sources) await fs.writeFile(path.join(root, name), source);
  await fs.writeFile(path.join(root, "consumer.py"), consumer);
  await fs.writeFile(path.join(root, "preparser.sage"), preparser);
  await server.start();
  const rootUri = pathToFileURL(root).href;
  await server.requestWithTimeout("initialize", {
    processId: process.pid,
    rootUri,
    capabilities: { workspace: { workspaceEdit: { documentChanges: true } } },
    initializationOptions: {
      analysis: { sourceRoots: [root], enablePythonFiles: true, enableRuntimeIntrospection: false },
      workspace: { folders: [root] },
      rust: { cacheDir: path.join(root, "cache") },
    },
  }, 10_000);
  server.notify("initialized", {});
  for (const [name, source, position] of sources) {
    const uri = pathToFileURL(path.join(root, name)).href;
    server.notify("textDocument/didOpen", {
      textDocument: { uri, languageId: name.endsWith(".sage") ? "sage" : "python", version: 7, text: source },
    });
    const target = { textDocument: { uri }, position };
    for (const badName of ["class", "for", "🙂", "1invalid", "已有"]) {
      const edit = await server.requestWithTimeout("textDocument/rename", { ...target, newName: badName }, 5_000);
      assert.equal(edit, null, `${name}: ${badName} must not produce edits`);
    }
    const references = await server.requestWithTimeout("textDocument/references", {
      ...target, context: { includeDeclaration: true },
    }, 5_000);
    assert.ok(references?.length >= 2, `${name}: expected declaration and use`);
    const edit = await server.requestWithTimeout("textDocument/rename", { ...target, newName: "新变量" }, 5_000);
    const fileEdit = edit?.documentChanges?.find((item) => item.textDocument.uri === uri);
    assert.equal(fileEdit?.textDocument.version, 7, `${name}: expected versioned edit`);
    assert.ok(fileEdit?.edits.length >= 2, `${name}: expected declaration and use edits`);
    assert.ok(fileEdit.edits.every((item) => item.newText === "新变量"));
    assert.equal(fileEdit.edits.find((item) => item.range.start.line === 3)?.range.end.character, 6);
    if (name.endsWith(".py")) {
      const useLine = source.split("\n")[6];
      const useRange = fileEdit.edits.find((item) => item.range.start.line === 6)?.range;
      assert.equal(useRange?.start.character, useLine.indexOf("变量"), "UTF-16 offset after emoji");
      assert.equal(useRange?.end.character, useLine.indexOf("变量") + 2);
    }
  }
  const originUri = pathToFileURL(path.join(root, "origin.py")).href;
  const consumerUri = pathToFileURL(path.join(root, "consumer.py")).href;
  server.notify("textDocument/didOpen", {
    textDocument: { uri: consumerUri, languageId: "python", version: 2, text: consumer },
  });
  const originEdit = await server.requestWithTimeout("textDocument/rename", {
    textDocument: { uri: originUri }, position: { line: 3, character: 5 }, newName: "新变量",
  }, 5_000);
  assert.ok(originEdit.documentChanges.some((item) => item.textDocument.uri === consumerUri
    && item.textDocument.version === 2 && item.edits.length >= 2), `cross-file Unicode rename: ${JSON.stringify(originEdit)}`);
  const preparserUri = pathToFileURL(path.join(root, "preparser.sage")).href;
  server.notify("textDocument/didOpen", {
    textDocument: { uri: preparserUri, languageId: "sage", version: 1, text: preparser },
  });
  const generatorDefinition = await server.requestWithTimeout("textDocument/definition", {
    textDocument: { uri: preparserUri }, position: { line: 1, character: 6 },
  }, 5_000);
  assert.equal(generatorDefinition?.uri, preparserUri, "Sage preparser generator definition");
  assert.equal(generatorDefinition?.range?.start.character, 3);
  await server.requestWithTimeout("shutdown", undefined, 2_000);
  server.notify("exit");
  console.log("Unicode rename and safety smoke passed for .py and .sage");
} finally {
  await server.terminateAndWait();
  await fs.rm(root, { recursive: true, force: true });
}
