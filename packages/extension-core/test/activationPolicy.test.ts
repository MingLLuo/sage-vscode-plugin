import test from "node:test";
import assert from "node:assert/strict";

import {
  isSageDocumentLanguage,
  shouldAutoStartLanguageClient,
  shouldExposeSageExperience,
} from "../src/activationPolicy";

test("Sage document language policy keeps ordinary Python opt-in", () => {
  assert.equal(isSageDocumentLanguage("sagemath", false), true);
  assert.equal(isSageDocumentLanguage("sagemath-cython", false), true);
  assert.equal(isSageDocumentLanguage("python", false), false);
  assert.equal(isSageDocumentLanguage("python", true), true);
});

test("activation policy does not auto-start Sage LSP for unrelated Python workspaces", () => {
  assert.equal(
    shouldAutoStartLanguageClient({
      activeLanguageId: "python",
      pythonFilesEnabled: false,
      sourceRoots: [],
      extraPaths: [],
    }),
    false,
  );
  assert.equal(
    shouldExposeSageExperience({
      activeLanguageId: "python",
      pythonFilesEnabled: false,
      sourceRoots: [],
      extraPaths: [],
    }),
    false,
  );
});

test("activation policy starts for Sage-heavy Python and configured source roots", () => {
  assert.equal(
    shouldAutoStartLanguageClient({
      activeLanguageId: "python",
      pythonFilesEnabled: true,
      sourceRoots: [],
      extraPaths: [],
    }),
    true,
  );
  assert.equal(
    shouldAutoStartLanguageClient({
      activeLanguageId: undefined,
      pythonFilesEnabled: false,
      sourceRoots: ["/workspace/sage/src"],
      extraPaths: [],
    }),
    true,
  );
});
