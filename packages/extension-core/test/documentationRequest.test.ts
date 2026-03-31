import test from "node:test";
import assert from "node:assert/strict";

import {
  buildDocumentationRequestPayload,
  normalizeDocumentationResponse,
  renderDocumentationMarkdown,
} from "../src/documentationRequest";

test("buildDocumentationRequestPayload includes symbol when provided", () => {
  assert.deepEqual(
    buildDocumentationRequestPayload("file:///workspace/example.sage", 3, 7, "PolynomialRing"),
    {
      textDocument: { uri: "file:///workspace/example.sage" },
      position: { line: 3, character: 7 },
      symbol: "PolynomialRing",
    },
  );
});

test("renderDocumentationMarkdown formats module and source metadata", () => {
  assert.equal(
    renderDocumentationMarkdown({
      symbol: "PolynomialRing",
      detail: "function PolynomialRing",
      module: "sage.rings.polynomial.polynomial_ring_constructor",
      kind: "function",
      uri: "file:///workspace/src/sage/rings/polynomial/polynomial_ring_constructor.py",
      summary: "Construct a polynomial ring.",
      markers: ["kind:function", "source:python"],
      sections: [{ title: "Details", body: "Extra details." }],
    }),
    [
      "# PolynomialRing",
      "",
      "function PolynomialRing",
      "Module: `sage.rings.polynomial.polynomial_ring_constructor`",
      "Kind: function",
      "Source: file:///workspace/src/sage/rings/polynomial/polynomial_ring_constructor.py",
      "",
      "> `kind:function` `source:python`",
      "",
      "Construct a polynomial ring.",
      "",
      "## Details",
      "",
      "Extra details.",
    ].join("\n"),
  );
});

test("normalizeDocumentationResponse maps server payload into extension result", () => {
  assert.deepEqual(
    normalizeDocumentationResponse({
      name: "sqrt",
      moduleName: "sage.functions.other",
      kind: "function",
      detail: "function sqrt",
      summary: "Return the principal square root.",
      docstring: "Return the principal square root.",
      uri: "file:///workspace/src/sage/functions/other.py",
      markers: ["kind:function"],
      sections: [{ title: "Signature", body: "sqrt(value)" }],
    }),
    {
      symbol: "sqrt",
      module: "sage.functions.other",
      kind: "function",
      detail: "function sqrt",
      summary: "Return the principal square root.",
      uri: "file:///workspace/src/sage/functions/other.py",
      markers: ["kind:function"],
      sections: [{ title: "Signature", body: "sqrt(value)" }],
    },
  );
});
