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
const responseBudgetMs = 1_500;
const tempRoot = await fs.mkdtemp(path.join(os.tmpdir(), "sage-lsp-shutdown-smoke-"));
const workspaceRoot = path.join(tempRoot, "workspace");
const cacheRoot = path.join(tempRoot, "cache");

async function runSmoke() {
  const server = new LspProcess(serverPath, {
    cwd: repositoryRoot,
    environment: {
      ...process.env,
      SAGE_LS_TEST_BACKGROUND_DELAY_MS: "4000",
    },
  });
  let completed = false;
  try {
    await fs.mkdir(workspaceRoot, { recursive: true });
    await fs.mkdir(cacheRoot, { recursive: true });
    await fs.writeFile(path.join(workspaceRoot, "demo.sage"), "R.<x> = PolynomialRing(QQ)\n", "utf8");
    await server.start();

    const workspaceUri = pathToFileURL(workspaceRoot).toString();
    await server.requestWithTimeout("initialize", {
      processId: process.pid,
      rootUri: workspaceUri,
      capabilities: {},
      workspaceFolders: [{ uri: workspaceUri, name: "shutdown-smoke" }],
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
    }, 5_000, "initialize response");
    server.notify("initialized", {});

    await waitForPendingJobs(server, 1, responseBudgetMs);

    await server.requestWithTimeout("workspace/executeCommand", {
      command: "sage.__rust.rebuildIndex",
      arguments: [],
    }, responseBudgetMs, "rebuild scheduling response");
    const pendingStatus = await waitForPendingJobs(server, 2, responseBudgetMs);

    const shutdownStarted = performance.now();
    await server.requestWithTimeout("shutdown", undefined, responseBudgetMs, "shutdown response");
    const shutdownResponseMs = Math.round(performance.now() - shutdownStarted);
    server.notify("exit");
    server.closeInput();
    const exit = await withTimeout(server.exitResult, responseBudgetMs, "server process exit");
    const shutdownAndExitMs = Math.round(performance.now() - shutdownStarted);

    assert.equal(exit.code, 0, `sage-ls exited with code ${exit.code}, signal ${exit.signal}`);
    assert.ok(
      shutdownResponseMs <= responseBudgetMs,
      `shutdown response took ${shutdownResponseMs}ms (budget ${responseBudgetMs}ms)`,
    );
    assert.ok(
      shutdownAndExitMs <= responseBudgetMs,
      `shutdown and process exit took ${shutdownAndExitMs}ms (budget ${responseBudgetMs}ms)`,
    );
    completed = true;
    console.log(JSON.stringify({
      status: "passed",
      shutdownResponseMs,
      shutdownAndExitMs,
      responseBudgetMs,
      pendingJobsBeforeShutdown: pendingStatus.pending_jobs,
    }, null, 2));
  } catch (error) {
    console.error(server.stderrText());
    throw error;
  } finally {
    if (!completed) {
      await server.terminateAndWait();
    }
    await fs.rm(tempRoot, { recursive: true, force: true });
  }
}

async function waitForPendingJobs(server, expected, timeoutMs) {
  const started = performance.now();
  let status;
  while (performance.now() - started <= timeoutMs) {
    const remainingMs = Math.max(1, timeoutMs - (performance.now() - started));
    status = await server.requestWithTimeout("workspace/executeCommand", {
      command: "sage.__rust.indexStatus",
      arguments: [],
    }, remainingMs, "index status response");
    if ((status?.pending_jobs ?? 0) >= expected) {
      return status;
    }
    await new Promise((resolve) => setTimeout(resolve, 10));
  }
  throw new Error(`expected ${expected} pending index jobs before shutdown, got ${JSON.stringify(status)}`);
}

await runSmoke();
