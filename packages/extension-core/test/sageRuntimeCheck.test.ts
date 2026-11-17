import assert from "node:assert/strict";
import test from "node:test";

import { describeVersionOutput, SageRuntimeChecker } from "../src/sageRuntimeCheck";

test("Sage version output is summarized", () => {
  assert.equal(describeVersionOutput("SageMath version 11.0.beta5, Release Date: 2026-10-28\n"), "SageMath 11.0.beta5");
  assert.equal(describeVersionOutput("\nPython 3.12.1\n"), "Python 3.12.1");
  assert.equal(describeVersionOutput(""), undefined);
});

test("a missing executable is reported as missing and cached", async () => {
  let calls = 0;
  let changes = 0;
  const checker = new SageRuntimeChecker(() => { changes += 1; }, async () => {
    calls += 1;
    throw Object.assign(new Error("spawn sage ENOENT"), { code: "ENOENT" });
  });
  assert.equal((await checker.ensure("sage")).status, "missing");
  assert.equal((await checker.ensure("sage")).status, "missing");
  assert.equal(calls, 1, "the answer is remembered per interpreter path");
  assert.equal(changes, 1);
  checker.invalidate();
  await checker.ensure("sage");
  assert.equal(calls, 2, "invalidate forces a new check");
});

test("concurrent checks share one probe and report the version", async () => {
  let calls = 0;
  const checker = new SageRuntimeChecker(() => undefined, async () => {
    calls += 1;
    return { stdout: "SageMath version 10.8, Release Date: 2026-01-01\n", code: 0 };
  });
  const [first, second] = await Promise.all([checker.ensure("/opt/sage"), checker.ensure("/opt/sage")]);
  assert.deepEqual(first, { status: "found", version: "SageMath 10.8" });
  assert.deepEqual(second, first);
  assert.equal(calls, 1);
});

test("a slow version check is not mistaken for a missing Sage", async () => {
  const { ProbeTimeoutError } = await import("../src/processProbe");
  const checker = new SageRuntimeChecker(() => undefined, async () => {
    throw new ProbeTimeoutError("Process probe timed out");
  });
  assert.equal((await checker.ensure("sage")).status, "unknown");
});
