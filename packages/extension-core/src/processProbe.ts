import { spawn } from "node:child_process";
import path from "node:path";
import { StringDecoder } from "node:string_decoder";

export class ProbeTimeoutError extends Error {}
export class ProbeOutputLimitError extends Error {}

/** Run a small discovery command with bounded output, lifetime and cancellation. */
export function readProcessProbe(command: string, args: readonly string[], options: {
  signal?: AbortSignal; timeoutMs: number; maxBytes: number; cwd?: string;
}): Promise<{ stdout: string; code: number | null }> {
  options.signal?.throwIfAborted();
  return new Promise((resolve, reject) => {
    const child = spawn(command, [...args], {
      cwd: options.cwd, windowsHide: true, detached: process.platform !== "win32",
      stdio: ["ignore", "pipe", "pipe"],
    });
    let stdout = "";
    const decoder = new StringDecoder("utf8");
    let bytes = 0;
    let settled = false;
    const terminate = (): void => {
      try {
        if (process.platform !== "win32" && child.pid) process.kill(-child.pid, "SIGKILL");
        else if (child.pid) {
          // Windows has no POSIX process groups. Ask taskkill to terminate the tree first.
          // Resolve taskkill from System32: a PATH or working-directory lookup could run
          // an unrelated executable with the same name.
          const taskkill = path.join(process.env.SystemRoot ?? "C:\\Windows", "System32", "taskkill.exe");
          const killer = spawn(taskkill, ["/pid", String(child.pid), "/T", "/F"], { windowsHide: true, stdio: "ignore" });
          killer.once("error", () => child.kill());
          killer.once("close", () => child.kill());
          killer.unref();
        }
      } catch { /* The process may already have exited. */ }
    };
    const finish = (error?: Error, code: number | null = null): void => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      options.signal?.removeEventListener("abort", abort);
      if (error) { terminate(); reject(error); } else resolve({ stdout: stdout + decoder.end(), code });
    };
    const abort = (): void => finish(new DOMException("Process probe cancelled", "AbortError"));
    const timer = setTimeout(() => finish(new ProbeTimeoutError("Process probe timed out")), options.timeoutMs);
    options.signal?.addEventListener("abort", abort, { once: true });
    if (options.signal?.aborted) abort();
    child.stdout.on("data", (chunk: Buffer) => {
      bytes += chunk.length;
      if (bytes > options.maxBytes) finish(new ProbeOutputLimitError("Process probe exceeded output limit"));
      else stdout += decoder.write(chunk);
    });
    child.stderr.resume();
    child.once("error", (error) => finish(error));
    child.once("close", (code) => finish(undefined, code));
  });
}
