import test from "node:test";
import assert from "node:assert/strict";

import {
  DOCUMENTATION_FALLBACK_ACTIONS,
  documentationFallbackActions,
  documentationFallbackCommand,
  documentationFallbackMessage,
} from "../src/documentationFallback";

test("documentationFallbackMessage names the selected symbol when available", () => {
  assert.equal(
    documentationFallbackMessage(" PolynomialRing "),
    "No Sage documentation available for `PolynomialRing`.",
  );
  assert.equal(
    documentationFallbackMessage(),
    "No Sage documentation available for the current symbol.",
  );
});

test("documentationFallbackActions map to troubleshootable Sage commands", () => {
  assert.deepEqual(documentationFallbackActions(), [
    DOCUMENTATION_FALLBACK_ACTIONS.showDocsStatus,
    DOCUMENTATION_FALLBACK_ACTIONS.showIndexStatus,
    DOCUMENTATION_FALLBACK_ACTIONS.runUxSelfCheck,
  ]);
  assert.equal(documentationFallbackCommand(DOCUMENTATION_FALLBACK_ACTIONS.showDocsStatus), "sage.showDocsStatus");
  assert.equal(documentationFallbackCommand(DOCUMENTATION_FALLBACK_ACTIONS.showIndexStatus), "sage.showIndexStatus");
  assert.equal(documentationFallbackCommand(DOCUMENTATION_FALLBACK_ACTIONS.runUxSelfCheck), "sage.runUxSelfCheck");
});
