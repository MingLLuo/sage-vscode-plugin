import test from "node:test";
import assert from "node:assert/strict";

test("placeholder extension test", () => {
  assert.equal("sagemath".startsWith("sage"), true);
});

