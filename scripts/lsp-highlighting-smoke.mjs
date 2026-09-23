#!/usr/bin/env node
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { LspProcess } from "./lib/lsp-process.mjs";

const repositoryRoot = fileURLToPath(new URL("../", import.meta.url));
const root = await fs.mkdtemp(path.join(os.tmpdir(), "sage-highlighting-"));
const server = new LspProcess(path.join(repositoryRoot, "target", "debug", "sage-ls"));
const source = [
  "outside = plot(1)",
  "def first():",
  "    变量 = 1",
  "    return '🚀' + str(变量)",
  "",
  "def second():",
  "    变量 = 2",
  "    return 变量",
  "",
  "def draw(plot):",
  "    return plot(2)",
  "",
  "@自定义",
  "def 过程():",
  "    return 变量",
  "# 变量",
  "text = '变量'",
  "",
  "from sage.all import QQ",
  "base = QQ",
  "from sage.misc.cachefunc import cached_method",
  "@cached_method",
  "def cached(self):",
  "    return QQ",
  "",
].join("\n");

function decodeTokens(data) {
  const tokens = [];
  let line = 0;
  let start = 0;
  for (let index = 0; index < data.length; index += 5) {
    const [deltaLine, deltaStart, length, tokenType, modifiers] = data.slice(index, index + 5);
    line += deltaLine;
    start = deltaLine === 0 ? start + deltaStart : deltaStart;
    tokens.push({ line, start, length, tokenType, modifiers });
  }
  return tokens;
}

try {
  for (const extension of ["sage", "py"]) {
    await fs.writeFile(path.join(root, `highlight.${extension}`), source);
  }
  await server.start();
  const initialize = await server.requestWithTimeout("initialize", {
    processId: process.pid,
    rootUri: pathToFileURL(root).href,
    capabilities: {},
    initializationOptions: {
      analysis: { sourceRoots: [root], enablePythonFiles: true, enableRuntimeIntrospection: false },
      workspace: { folders: [root] },
      rust: { cacheDir: path.join(root, "cache") },
    },
  }, 10_000);
  server.notify("initialized", {});
  const types = initialize.capabilities.semanticTokensProvider.legend.tokenTypes;
  const modifiers = initialize.capabilities.semanticTokensProvider.legend.tokenModifiers;
  for (const extension of ["sage", "py"]) {
    const uri = pathToFileURL(path.join(root, `highlight.${extension}`)).href;
    server.notify("textDocument/didOpen", {
      textDocument: { uri, languageId: extension === "sage" ? "sage" : "python", version: 1, text: source },
    });
    const firstUse = { line: 3, character: source.split("\n")[3].indexOf("变量") };
    const first = await server.requestWithTimeout("textDocument/documentHighlight", {
      textDocument: { uri }, position: firstUse,
    }, 5_000);
    assert.deepEqual(first.map((item) => item.range.start.line).sort((a, b) => a - b), [2, 3],
      `${extension}: highlights must stay in the selected scope and skip comments/strings`);
    assert.equal(first.find((item) => item.range.start.line === 3)?.range.start.character,
      firstUse.character, `${extension}: highlight must use UTF-16 positions after emoji`);

    const second = await server.requestWithTimeout("textDocument/documentHighlight", {
      textDocument: { uri }, position: { line: 7, character: 11 },
    }, 5_000);
    assert.deepEqual(second.map((item) => item.range.start.line).sort((a, b) => a - b), [6, 7],
      `${extension}: same-name variable in another function must have separate highlights`);

    const semantic = await server.requestWithTimeout("textDocument/semanticTokens/full", {
      textDocument: { uri },
    }, 5_000);
    const tokens = decodeTokens(semantic.data);
    const libraryBit = 1 << modifiers.indexOf("defaultLibrary");
    const functionType = types.indexOf("function");
    assert.ok(tokens.some((token) => token.line === 0 && token.tokenType === functionType
      && (token.modifiers & libraryBit) !== 0), `${extension}: unshadowed Sage function should be marked as library`);
    assert.ok(!tokens.some((token) => token.line === 10 && token.tokenType === functionType
      && (token.modifiers & libraryBit) !== 0), `${extension}: parameter shadowing must suppress library coloring`);
    assert.ok(tokens.some((token) => token.line === 13 && token.tokenType === functionType
      && token.length === 2), `${extension}: Unicode declaration length must be UTF-16`);
    assert.ok(tokens.some((token) => token.line === 19 && token.tokenType === types.indexOf("variable")
      && (token.modifiers & libraryBit) !== 0), `${extension}: explicit Sage imports retain library coloring`);
    assert.ok(tokens.some((token) => token.line === 21 && token.tokenType === types.indexOf("decorator")
      && (token.modifiers & libraryBit) !== 0), `${extension}: imported Sage decorators retain library coloring`);
  }
  await server.requestWithTimeout("shutdown", undefined, 2_000);
  server.notify("exit");
  console.log("Semantic tokens and scoped document highlights passed for .sage and .py");
} finally {
  await server.terminateAndWait();
  await fs.rm(root, { recursive: true, force: true });
}
