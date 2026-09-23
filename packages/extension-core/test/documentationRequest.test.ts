import test from "node:test";
import assert from "node:assert/strict";

import {
  buildDocumentationRequestPayload,
  formatSageDocstringMarkdown,
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
      docstring: "Construct a polynomial ring.\n\nFull constructor details.",
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
      "Full constructor details.",
      "",
      "## Details",
      "",
      "Extra details.",
    ].join("\n"),
  );
});

test("candidate documentation explains uncertainty without internal labels", () => {
  const markdown = renderDocumentationMarkdown({
    symbol: "degree",
    detail: "Possible definitions of `degree`",
    module: "",
    kind: "AmbiguousMember",
    uri: undefined,
    summary: "Found two indexed definitions; the receiver type is not known.",
    docstring: "Add a known constructor to narrow the result.",
    markers: ["ambiguous", "source:rust-index-v2"],
    sections: [{ title: "PolynomialElement — sage.rings", body: "degree(self)\n\nReturn the degree." }],
  });
  assert.match(markdown, /Possible definitions of `degree`/);
  assert.match(markdown, /Return the degree/);
  assert.doesNotMatch(markdown, /Kind: AmbiguousMember|Module: `ambiguous`|source:rust-index-v2/);
});

test("normalizeDocumentationResponse maps server payload into extension result", () => {
  assert.deepEqual(
    normalizeDocumentationResponse({
      name: "sqrt",
      moduleName: "sage.functions.other",
      kind: "function",
      detail: "function sqrt",
      summary: "Return the principal square root.",
      docstring: "Return the principal square root.\n\nFull docs.",
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
      docstring: "Return the principal square root.\n\nFull docs.",
      uri: "file:///workspace/src/sage/functions/other.py",
      markers: ["kind:function"],
      sections: [{ title: "Signature", body: "sqrt(value)" }],
    },
  );
});

test("formatSageDocstringMarkdown converts Sage literal examples into code fences", () => {
  assert.equal(
    formatSageDocstringMarkdown([
      "Return combinations.",
      "",
      "    EXAMPLES::",
      "",
      "        sage: C = Combinations(range(3)); C",
      "        Combinations of [0, 1, 2]",
      "        sage: C.cardinality()",
      "        8",
      "",
      "    The parameter ``mset`` controls the input multiset.",
    ].join("\n")),
    [
      "Return combinations.",
      "",
      "EXAMPLES:",
      "",
      "```sage",
      "sage: C = Combinations(range(3)); C",
      "Combinations of [0, 1, 2]",
      "sage: C.cardinality()",
      "8",
      "",
      "```",
      "The parameter ``mset`` controls the input multiset.",
    ].join("\n"),
  );
});
