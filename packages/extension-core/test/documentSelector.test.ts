import assert from "node:assert/strict";
import test from "node:test";

import { buildDocumentSelector } from "../src/documentSelector";

test("Sage document selectors exclude the read-only external source scheme", () => {
  const selector = buildDocumentSelector({ pythonFilesEnabled: false });

  assert.deepEqual(selector, [
    { language: "sagemath", scheme: "file" },
    { language: "sagemath", scheme: "untitled" },
    { language: "sagemath-cython", scheme: "file" },
    { language: "sagemath-cython", scheme: "untitled" },
  ]);
  assert.equal(
    selector.some((filter) => typeof filter !== "string" && filter.scheme === "sage-source"),
    false,
  );
});

test("ordinary Python files remain an explicit opt-in file selector", () => {
  const selector = buildDocumentSelector({ pythonFilesEnabled: true });

  assert.deepEqual(selector.at(-1), { language: "python", scheme: "file" });
  assert.equal(selector.length, 5);
});
