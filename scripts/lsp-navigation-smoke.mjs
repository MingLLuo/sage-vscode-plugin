#!/usr/bin/env node
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { LspProcess, withTimeout } from "./lib/lsp-process.mjs";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const repositoryRoot = path.resolve(__dirname, "..");
const serverPath = path.join(
  repositoryRoot,
  "target",
  "debug",
  process.platform === "win32" ? "sage-ls.exe" : "sage-ls",
);
const tempRoot = await fs.mkdtemp(path.join(os.tmpdir(), "sage-lsp-navigation-smoke-"));
const workspaceRoot = path.join(tempRoot, "workspace ü space");
const cacheRoot = path.join(tempRoot, "cache");
const requestTimeoutMs = 5_000;

function positionOf(text, needle, occurrence = 1) {
  let offset = -1;
  for (let index = 0; index < occurrence; index += 1) {
    offset = text.indexOf(needle, offset + 1);
    assert.notEqual(offset, -1, `missing occurrence ${occurrence} of ${needle}`);
  }
  const prefix = text.slice(0, offset);
  const lines = prefix.split("\n");
  return { line: lines.length - 1, character: lines.at(-1).length + 1 };
}

function positionInLine(text, lineNeedle, symbol) {
  const lines = text.split("\n");
  const line = lines.findIndex((candidate) => candidate.includes(lineNeedle));
  assert.notEqual(line, -1, `missing line containing ${lineNeedle}`);
  const character = lines[line].indexOf(symbol);
  assert.notEqual(character, -1, `missing ${symbol} on line containing ${lineNeedle}`);
  return { line, character };
}

async function waitForIndex(server) {
  const started = performance.now();
  let status;
  while (performance.now() - started < requestTimeoutMs) {
    const remainingMs = Math.max(1, requestTimeoutMs - (performance.now() - started));
    status = await server.requestWithTimeout("workspace/executeCommand", {
      command: "sage.__rust.indexStatus",
      arguments: [],
    }, Math.min(1_000, remainingMs), "index status response");
    if ((status?.symbol_count ?? 0) > 0 && (status?.pending_jobs ?? 0) === 0) {
      return status;
    }
    await new Promise((resolve) => setTimeout(resolve, 20));
  }
  throw new Error(`sage-ls index did not become ready: ${JSON.stringify(status)}`);
}

async function runSmoke() {
  await fs.mkdir(workspaceRoot, { recursive: true });
  await fs.mkdir(cacheRoot, { recursive: true });
  const providerPath = path.join(workspaceRoot, "provider.py");
  const consumerPath = path.join(workspaceRoot, "consumer.py");
  const providerText = [
    "class First:",
    "    def shared(self):",
    "        return 1",
    "",
    "class Second:",
    "    def shared(self):",
    "        return 2",
    "",
    "class Only:",
    "    def solitary(self):",
    "        return 3",
    "",
    "def exact():",
    "    return 4",
  ].join("\n");
  const consumerText = [
    "from provider import exact",
    "from provider import exact as local_exact",
    "note = \"😀\"; result = exact()",
    "alias_result = local_exact()",
    "def caller():",
    "    ambiguous = unknown.shared()",
    "    weak = unknown.solitary()",
  ].join("\n");
  await fs.writeFile(providerPath, providerText, "utf8");
  await fs.writeFile(consumerPath, consumerText, "utf8");
  const providerUri = pathToFileURL(await fs.realpath(providerPath)).toString();

  const server = new LspProcess(serverPath, { cwd: repositoryRoot });
  let started = false;
  try {
    await server.start();
    started = true;
    const workspaceUri = pathToFileURL(workspaceRoot).toString();
    const consumerUri = pathToFileURL(consumerPath).toString();
    await server.requestWithTimeout("initialize", {
      processId: process.pid,
      rootUri: workspaceUri,
      capabilities: {
        textDocument: {
          declaration: { linkSupport: false },
          definition: { linkSupport: true },
          typeDefinition: { linkSupport: true },
        },
      },
      workspaceFolders: [{ uri: workspaceUri, name: "navigation-smoke" }],
      initializationOptions: {
        analysis: {
          sourceRoots: [workspaceRoot],
          extraPaths: [],
          enableDiagnostics: true,
          enablePyxParsing: true,
          enablePythonFiles: true,
        },
        workspace: {
          rootUri: workspaceUri,
          folders: [workspaceUri],
          sourceRoots: [workspaceUri],
          exclude: [],
        },
        documentation: { preferredSource: "static", showOnHover: true },
        rust: { cacheDir: cacheRoot },
      },
    }, requestTimeoutMs, "initialize response");
    server.notify("initialized", {});
    server.notify("textDocument/didOpen", {
      textDocument: {
        uri: consumerUri,
        languageId: "python",
        version: 1,
        text: consumerText,
      },
    });
    server.notify("textDocument/didOpen", {
      textDocument: {
        uri: providerUri,
        languageId: "python",
        version: 1,
        text: providerText,
      },
    });
    const indexStatus = await waitForIndex(server);

    const exactPosition = positionInLine(consumerText, "result =", "exact");
    const sharedPosition = positionOf(consumerText, "shared");
    const solitaryPosition = positionOf(consumerText, "solitary");
    const requestAtDocument = (method, uri, position, extra = {}) => server.requestWithTimeout(method, {
      textDocument: { uri },
      position,
      ...extra,
    }, requestTimeoutMs, `${method} response`);
    const requestAt = (method, position, extra = {}) => requestAtDocument(
      method,
      consumerUri,
      position,
      extra,
    );

    const exactDefinition = await requestAt("textDocument/definition", exactPosition);
    assert.equal(Array.isArray(exactDefinition), false, "high-confidence definition must be scalar");
    assert.equal(exactDefinition?.uri, providerUri);
    assert.deepEqual(exactDefinition?.range, {
      start: { line: 12, character: 4 },
      end: { line: 12, character: 9 },
    });

    const ambiguousDefinition = await requestAt("textDocument/definition", sharedPosition);
    assert.equal(Array.isArray(ambiguousDefinition), true, "ambiguous definition must return links");
    assert.equal(ambiguousDefinition.length, 2);
    assert.deepEqual(
      ambiguousDefinition.map((link) => link.targetSelectionRange.start.line),
      [1, 5],
      "candidate links must remain deterministically ordered",
    );
    assert.ok(ambiguousDefinition.every((link) => link.targetUri === providerUri));
    assert.deepEqual(
      ambiguousDefinition.map((link) => link.targetSelectionRange),
      [1, 5].map((line) => ({
        start: { line, character: 8 },
        end: { line, character: 14 },
      })),
      "candidate links must retain exact symbol ranges",
    );

    const ambiguousDeclaration = await requestAt("textDocument/declaration", sharedPosition);
    assert.deepEqual(
      ambiguousDeclaration,
      [1, 5].map((line) => ({
        uri: providerUri,
        range: {
          start: { line, character: 8 },
          end: { line, character: 14 },
        },
      })),
      "clients without declaration linkSupport must receive ordered exact Locations",
    );

    const ambiguousHover = await requestAt("textDocument/hover", sharedPosition);
    const hoverMarkdown = ambiguousHover?.contents?.value ?? "";
    assert.match(hoverMarkdown, /Top indexed candidates/i);
    assert.match(hoverMarkdown, /First\.shared/);
    assert.match(hoverMarkdown, /Second\.shared/);

    const weakDefinition = await requestAt("textDocument/definition", solitaryPosition);
    assert.equal(weakDefinition, null, "one weak candidate must not be forced into a jump");

    const ambiguousReferences = await requestAt("textDocument/references", sharedPosition, {
      context: { includeDeclaration: true },
    });
    assert.deepEqual(ambiguousReferences, [], "ambiguous targets must not produce references");
    const ambiguousRename = await requestAt("textDocument/rename", sharedPosition, {
      newName: "renamed_shared",
    });
    assert.equal(ambiguousRename, null, "ambiguous targets must not produce rename edits");
    const ambiguousCallHierarchy = await requestAt("textDocument/prepareCallHierarchy", sharedPosition);
    assert.deepEqual(ambiguousCallHierarchy, [], "ambiguous targets must not enter call hierarchy");

    const sourceRename = await requestAtDocument(
      "textDocument/rename",
      providerUri,
      { line: 12, character: 5 },
      { newName: "renamed_exact" },
    );
    const providerEdits = sourceRename?.changes?.[providerUri] ?? [];
    const consumerEdits = sourceRename?.changes?.[consumerUri] ?? [];
    assert.deepEqual(
      providerEdits.map((edit) => [edit.range.start.line, edit.range.start.character]),
      [[12, 4]],
      "source rename must include the definition",
    );
    assert.deepEqual(
      consumerEdits.map((edit) => [edit.range.start.line, edit.range.start.character]),
      [[0, 21], [1, 21], [2, 22]],
      "source rename must update import source names and direct uses only",
    );
    assert.ok(
      consumerEdits.every((edit) => edit.newText === "renamed_exact"),
      "source rename edits must carry the requested name",
    );

    await server.requestWithTimeout("shutdown", undefined, requestTimeoutMs, "shutdown response");
    server.notify("exit");
    server.closeInput();
    const exit = await withTimeout(server.exitResult, requestTimeoutMs, "server exit");
    assert.equal(exit.code, 0);
    started = false;
    console.log(JSON.stringify({
      status: "passed",
      indexedSymbols: indexStatus.symbol_count,
      exactTargetLine: exactDefinition.range.start.line,
      ambiguousTargetLines: ambiguousDefinition.map((link) => link.targetSelectionRange.start.line),
      fallbackDeclarationLines: ambiguousDeclaration.map((location) => location.range.start.line),
      weakTarget: weakDefinition,
      sourceRenameConsumerEdits: consumerEdits.length,
    }, null, 2));
  } catch (error) {
    console.error(server.stderrText());
    throw error;
  } finally {
    if (started) {
      await server.terminateAndWait();
    }
  }
}

try {
  await runSmoke();
} finally {
  await fs.rm(tempRoot, { recursive: true, force: true });
}
