import { spawnSync as systemSpawnSync } from "node:child_process";

const VERSION_TIMEOUT_MS = 8_000;
const IMPORT_TIMEOUT_MS = 30_000;

export const SAGE_RUNTIME_PROBE_CODE = [
  "from sage.all import ZZ",
  "assert ZZ(2) + ZZ(2) == 4",
].join("; ");

export function probeSageRuntimeCandidates(values, options = {}) {
  const spawnSync = options.spawnSync ?? systemSpawnSync;
  const candidates = [...new Set(values.filter(Boolean).map((value) => String(value).trim()).filter(Boolean))];
  let bestFailure = null;

  for (const candidate of candidates) {
    const versionResult = spawnSync(candidate, ["--version"], spawnOptions(VERSION_TIMEOUT_MS));
    if (versionResult.status !== 0) {
      bestFailure ??= {
        path: null,
        candidate,
        reason: `Sage executable did not start: ${spawnFailure(versionResult)}`,
      };
      continue;
    }

    const version = firstLine(versionResult.stdout) || firstLine(versionResult.stderr) || "version unavailable";
    const importResult = spawnSync(
      candidate,
      ["-python", "-c", SAGE_RUNTIME_PROBE_CODE],
      spawnOptions(IMPORT_TIMEOUT_MS),
    );
    if (importResult.status === 0) {
      return {
        path: candidate,
        version,
        probe: "sage.all import",
      };
    }

    bestFailure = {
      path: null,
      candidate,
      version,
      probe: "sage.all import",
      reason: `Sage reports a version but cannot import its runtime: ${spawnFailure(importResult)}`,
    };
  }

  return bestFailure ?? {
    path: null,
    reason: "Set SAGE_PATH or configure `sage.interpreter.path` in VS Code.",
  };
}

function spawnOptions(timeout) {
  return {
    encoding: "utf8",
    timeout,
    maxBuffer: 1024 * 1024,
  };
}

function spawnFailure(result) {
  if (result.error?.code === "ETIMEDOUT") {
    return "probe timed out";
  }
  return conciseFailure(result.stderr)
    || conciseFailure(result.stdout)
    || result.error?.message
    || (result.signal ? `terminated by ${result.signal}` : `exit status ${result.status ?? "unknown"}`);
}

function conciseFailure(text) {
  const lines = String(text ?? "").trim().split(/\r?\n/).map((line) => line.trim()).filter(Boolean);
  if (lines[0]?.startsWith("Traceback") && lines.length > 1) {
    return lines.at(-1);
  }
  return lines[0] ?? "";
}

function firstLine(text) {
  return String(text ?? "").trim().split(/\r?\n/).find(Boolean) ?? "";
}
