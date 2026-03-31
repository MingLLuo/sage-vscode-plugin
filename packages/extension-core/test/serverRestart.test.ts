import test from "node:test";
import assert from "node:assert/strict";

import { shouldRestartLanguageServer } from "../src/serverRestart";

test("shouldRestartLanguageServer ignores run-target-only changes", () => {
  assert.equal(shouldRestartLanguageServer((section) => section === "sage.run.target"), false);
});

test("shouldRestartLanguageServer reacts to language-server-affecting settings", () => {
  assert.equal(shouldRestartLanguageServer((section) => section === "sage.analysis"), true);
  assert.equal(shouldRestartLanguageServer((section) => section === "sage.docs"), true);
});
