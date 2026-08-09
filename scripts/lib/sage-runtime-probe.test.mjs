import assert from "node:assert/strict";
import test from "node:test";

import {
  probeSageRuntimeCandidates,
  SAGE_RUNTIME_PROBE_CODE,
} from "./sage-runtime-probe.mjs";

test("rejects a Sage command that only reports a version", () => {
  const result = probeSageRuntimeCandidates(["source-only-sage"], {
    spawnSync: (_command, args) => args[0] === "--version"
      ? completed(0, "SageMath version 10.10.beta8\n")
      : completed(1, "", "Traceback (most recent call last):\n  File \"<string>\", line 1\nModuleNotFoundError: No module named 'sage'\n"),
  });

  assert.equal(result.path, null);
  assert.equal(result.candidate, "source-only-sage");
  assert.equal(result.version, "SageMath version 10.10.beta8");
  assert.match(result.reason, /cannot import its runtime/);
  assert.match(result.reason, /ModuleNotFoundError/);
});

test("continues past a broken checkout to a working Sage runtime", () => {
  const calls = [];
  const result = probeSageRuntimeCandidates(["source-only-sage", "installed-sage"], {
    spawnSync: (command, args) => {
      calls.push([command, ...args]);
      if (args[0] === "--version") {
        return completed(0, `SageMath version for ${command}\n`);
      }
      return command === "installed-sage"
        ? completed(0)
        : completed(1, "", "missing compiled dependency\n");
    },
  });

  assert.equal(result.path, "installed-sage");
  assert.equal(result.probe, "sage.all import");
  assert.ok(calls.some((call) => call.includes(SAGE_RUNTIME_PROBE_CODE)));
});

test("deduplicates candidates and explains a launch timeout", () => {
  let callCount = 0;
  const result = probeSageRuntimeCandidates([" sage ", "sage"], {
    spawnSync: () => {
      callCount += 1;
      return { status: null, signal: "SIGTERM", stdout: "", stderr: "", error: { code: "ETIMEDOUT" } };
    },
  });

  assert.equal(callCount, 1);
  assert.equal(result.path, null);
  assert.match(result.reason, /timed out/);
});

function completed(status, stdout = "", stderr = "") {
  return { status, signal: null, stdout, stderr };
}
