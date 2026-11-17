import test from "node:test";
import assert from "node:assert/strict";
import { PACKAGE_TARGET_NAMES, resolvePackageTarget, selectedPackageTargetName, stagedRelativePaths } from "./package-targets.mjs";
import { evaluateLicenseExpression } from "./third-party-notices.mjs";

test("target selection propagates explicit overrides and rejects invalid names", () => {
  assert.equal(selectedPackageTargetName(["--target", "linux-arm64"], { SAGE_VSIX_TARGET: "darwin-x64" }), "linux-arm64");
  assert.equal(selectedPackageTargetName(["--target=win32-x64"], {}), "win32-x64");
  assert.throws(() => selectedPackageTargetName(["--target", "--out-dir"], {}));
  for (const name of ["toString", "__proto__", "linux-mips", ""]) assert.throws(() => resolvePackageTarget(name));
});

test("eight VSIX targets map to six native binaries and consistent sidecars", () => {
  assert.equal(PACKAGE_TARGET_NAMES.length, 8);
  assert.equal(new Set(PACKAGE_TARGET_NAMES.map(name => resolvePackageTarget(name).rustTarget)).size, 6);
  for (const name of PACKAGE_TARGET_NAMES) {
    const target = resolvePackageTarget(name);
    const files = stagedRelativePaths(target);
    assert.equal(files.binary.endsWith(".exe"), name.startsWith("win32"));
    assert.equal(files.sha256, `${files.binary}.sha256`);
  }
  for (const arch of ["arm64", "x64"]) assert.deepEqual(stagedRelativePaths(resolvePackageTarget(`linux-${arch}`)), stagedRelativePaths(resolvePackageTarget(`alpine-${arch}`)));
});

test("license expressions preserve AND/OR precedence and reject unknown terms", () => {
  assert.equal(evaluateLicenseExpression("MIT OR Apache-2.0").accepted, true);
  assert.equal(evaluateLicenseExpression("MIT AND Unknown").accepted, false);
  assert.equal(evaluateLicenseExpression("Unknown OR (MIT AND Apache-2.0)").accepted, true);
  assert.equal(evaluateLicenseExpression("Apache-2.0 WITH LLVM-exception").accepted, true);
  assert.throws(() => evaluateLicenseExpression("(MIT OR Apache-2.0"));
});
