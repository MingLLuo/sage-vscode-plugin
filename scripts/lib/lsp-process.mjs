import assert from "node:assert/strict";
import { spawn } from "node:child_process";

export class LspProcess {
  constructor(command, options = {}) {
    this.command = command;
    this.cwd = options.cwd ?? process.cwd();
    this.environment = options.environment ?? process.env;
    this.nextId = 1;
    this.pending = new Map();
    this.buffer = Buffer.alloc(0);
    this.stderr = [];
  }

  async start() {
    this.child = spawn(this.command, [], {
      cwd: this.cwd,
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
    return this.beginRequest(method, params).promise;
  }

  requestWithTimeout(method, params, timeoutMs, label = `${method} response`) {
    const { id, promise } = this.beginRequest(method, params);
    let timeout;
    const timeoutPromise = new Promise((_, reject) => {
      timeout = setTimeout(() => {
        this.pending.delete(id);
        reject(new Error(`${label} exceeded ${timeoutMs}ms`));
      }, timeoutMs);
    });
    return Promise.race([promise, timeoutPromise]).finally(() => clearTimeout(timeout));
  }

  beginRequest(method, params) {
    const id = this.nextId++;
    const promise = new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
    });
    this.write(params === undefined
      ? { jsonrpc: "2.0", id, method }
      : { jsonrpc: "2.0", id, method, params });
    return { id, promise };
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

  async terminateAndWait(timeoutMs = 1_000) {
    this.terminate();
    if (!this.exitResult) {
      return;
    }
    try {
      await withTimeout(this.exitResult, timeoutMs, "forced server exit");
    } catch {
      // The caller is already handling the primary failure. Do not mask it with cleanup noise.
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

export function withTimeout(promise, timeoutMs, label) {
  let timeout;
  const timeoutPromise = new Promise((_, reject) => {
    timeout = setTimeout(() => reject(new Error(`${label} exceeded ${timeoutMs}ms`)), timeoutMs);
  });
  return Promise.race([promise, timeoutPromise]).finally(() => clearTimeout(timeout));
}
