import test from "node:test";
import assert from "node:assert/strict";

import {
  shouldRestartLanguageServer,
  shouldStopOnTransportErrors,
} from "../src/serverRestart";

test("shouldRestartLanguageServer ignores run UI and target changes", () => {
  assert.equal(shouldRestartLanguageServer((section) => section === "sage.run.target"), false);
  assert.equal(shouldRestartLanguageServer((section) => section === "sage.run.showCellCodeLens"), false);
});

test("shouldRestartLanguageServer reacts to language-server-affecting settings", () => {
  assert.equal(shouldRestartLanguageServer((section) => section === "sage.analysis"), true);
  assert.equal(shouldRestartLanguageServer((section) => section === "sage.docs"), true);
  assert.equal(shouldRestartLanguageServer((section) => section === "sage.languageServer"), true);
});

test("transport errors stop the failed connection after three errors", () => {
  assert.equal(shouldStopOnTransportErrors(undefined), false);
  assert.equal(shouldStopOnTransportErrors(3), false);
  assert.equal(shouldStopOnTransportErrors(4), true);
});
