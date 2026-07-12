#!/usr/bin/env node
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

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
    ...process.env,
    SAGE_LS_TEST_BACKGROUND_DELAY_MS: "4000",
  });
  let completed = false;
  try {
    await fs.mkdir(workspaceRoot, { recursive: true });
    await fs.mkdir(cacheRoot, { recursive: true });
    await fs.writeFile(path.join(workspaceRoot, "demo.sage"), "R.<x> = PolynomialRing(QQ)\n", "utf8");
    await server.start();

    const workspaceUri = pathToFileURL(workspaceRoot).toString();
    await withTimeout(server.request("initialize", {
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
    }), 5_000, "initialize response");
    server.notify("initialized", {});

    await waitForPendingJobs(server, 1, responseBudgetMs);

    await withTimeout(server.request("workspace/executeCommand", {
      command: "sage.__rust.rebuildIndex",
      arguments: [],
    }), responseBudgetMs, "rebuild scheduling response");
    const pendingStatus = await waitForPendingJobs(server, 2, responseBudgetMs);

    const shutdownStarted = performance.now();
    await withTimeout(server.request("shutdown"), responseBudgetMs, "shutdown response");
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
      server.terminate();
    }
    await fs.rm(tempRoot, { recursive: true, force: true });
  }
}

async function waitForPendingJobs(server, expected, timeoutMs) {
  const started = performance.now();
  let status;
  while (performance.now() - started <= timeoutMs) {
    status = await withTimeout(server.request("workspace/executeCommand", {
      command: "sage.__rust.indexStatus",
      arguments: [],
    }), timeoutMs, "index status response");
    if ((status?.pending_jobs ?? 0) >= expected) {
      return status;
    }
    await new Promise((resolve) => setTimeout(resolve, 10));
  }
  throw new Error(`expected ${expected} pending index jobs before shutdown, got ${JSON.stringify(status)}`);
}

function withTimeout(promise, timeoutMs, label) {
  let timeout;
  const timeoutPromise = new Promise((_, reject) => {
    timeout = setTimeout(() => reject(new Error(`${label} exceeded ${timeoutMs}ms`)), timeoutMs);
  });
  return Promise.race([promise, timeoutPromise]).finally(() => clearTimeout(timeout));
}

class LspProcess {
  constructor(command, environment) {
    this.command = command;
    this.environment = environment;
    this.nextId = 1;
    this.pending = new Map();
    this.buffer = Buffer.alloc(0);
    this.stderr = [];
  }

  async start() {
    this.child = spawn(this.command, [], {
      cwd: repositoryRoot,
      env: this.environment,
      stdio: ["pipe", "pipe", "pipe"],
    });
    this.exitResult = new Promise((resolve, reject) => {
      this.child.once("error", reject);
      this.child.once("exit", (code, signal) => {
        for (const pending of this.pending.values()) {
          pending.reject(new Error(`sage-ls exited before responding: code ${code}, signal ${signal}`));
        }
        this.pending.clear();
        resolve({ code, signal });
      });
    });
    this.child.stdout.on("data", (chunk) => this.handleData(chunk));
    this.child.stderr.on("data", (chunk) => {
      this.stderr.push(chunk.toString("utf8"));
    });
  }

  request(method, params) {
    const id = this.nextId++;
    const response = new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
    });
    this.write(params === undefined
      ? { jsonrpc: "2.0", id, method }
      : { jsonrpc: "2.0", id, method, params });
    return response;
  }

  notify(method, params) {
    this.write(params === undefined
      ? { jsonrpc: "2.0", method }
      : { jsonrpc: "2.0", method, params });
  }

  closeInput() {
    this.child?.stdin.end();
  }

  terminate() {
    if (this.child && this.child.exitCode === null && this.child.signalCode === null) {
      this.child.kill("SIGKILL");
    }
  }

  stderrText() {
    return this.stderr.join("").trim();
  }

  write(message) {
    const json = JSON.stringify(message);
    this.child.stdin.write(`Content-Length: ${Buffer.byteLength(json, "utf8")}\r\n\r\n${json}`);
  }

  handleData(chunk) {
    this.buffer = Buffer.concat([this.buffer, chunk]);
    while (true) {
      const headerEnd = this.buffer.indexOf("\r\n\r\n");
      if (headerEnd < 0) {
        return;
      }
      const header = this.buffer.subarray(0, headerEnd).toString("ascii");
      const lengthMatch = /Content-Length:\s*(\d+)/i.exec(header);
      assert.ok(lengthMatch, `missing Content-Length header: ${header}`);
      const length = Number(lengthMatch[1]);
      const messageStart = headerEnd + 4;
      const messageEnd = messageStart + length;
      if (this.buffer.length < messageEnd) {
        return;
      }
      const message = JSON.parse(this.buffer.subarray(messageStart, messageEnd).toString("utf8"));
      this.buffer = this.buffer.subarray(messageEnd);
      this.handleMessage(message);
    }
  }

  handleMessage(message) {
    if (message.method && Object.hasOwn(message, "id")) {
      this.write({ jsonrpc: "2.0", id: message.id, result: null });
      return;
    }
    const pending = this.pending.get(message.id);
    if (!pending) {
      return;
    }
    this.pending.delete(message.id);
    if (message.error) {
      pending.reject(new Error(JSON.stringify(message.error)));
    } else {
      pending.resolve(message.result);
    }
  }
}

await runSmoke();
