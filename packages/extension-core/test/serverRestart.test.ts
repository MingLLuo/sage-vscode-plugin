import test from "node:test";
import assert from "node:assert/strict";

import {
  shouldRestartLanguageServer,
  shouldAutoRestartOnLanguageServerClose,
} from "../src/serverRestart";

test("shouldRestartLanguageServer ignores run-target-only changes", () => {
  assert.equal(shouldRestartLanguageServer((section) => section === "sage.run.target"), false);
});

test("shouldRestartLanguageServer reacts to language-server-affecting settings", () => {
  assert.equal(shouldRestartLanguageServer((section) => section === "sage.analysis"), true);
  assert.equal(shouldRestartLanguageServer((section) => section === "sage.docs"), true);
  assert.equal(shouldRestartLanguageServer((section) => section === "sage.languageServer"), true);
});

test("shouldAutoRestartOnLanguageServerClose suppresses auto-restart for managed shutdown", () => {
  assert.equal(shouldAutoRestartOnLanguageServerClose(true), false);
  assert.equal(shouldAutoRestartOnLanguageServerClose(false), true);
});
