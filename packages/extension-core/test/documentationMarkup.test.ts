import test from "node:test";
import assert from "node:assert/strict";

import { renderDocumentationHtml } from "../src/documentationMarkup";

test("renderDocumentationHtml converts headings, markers, lists, and inline code", () => {
  const html = renderDocumentationHtml([
    "# Graph",
    "",
    "class Graph",
    "Module: `sage.graphs.graph`",
    "",
    "> `kind:class` `source:py`",
    "",
    "",
    "- vertices",
    "- edges",
  ].join("\n"));

  assert.match(html, /<h1>Graph<\/h1>/);
  assert.match(html, /<code>sage\.graphs\.graph<\/code>/);
  assert.match(html, /<blockquote><code>kind:class<\/code> <code>source:py<\/code><\/blockquote>/);
  assert.match(html, /<ul><li>vertices<\/li><li>edges<\/li><\/ul>/);
});

test("renderDocumentationHtml preserves Sage doctest code fences", () => {
  const html = renderDocumentationHtml([
    "# Combinations",
    "",
    "EXAMPLES:",
    "",
    "```sage",
    "sage: C = Combinations(range(3)); C",
    "Combinations of [0, 1, 2]",
    "",
    "sage: C.cardinality()",
    "8",
    "```",
    "",
    "Parameter ``mset`` is shown as inline code.",
  ].join("\n"));

  assert.match(html, /<pre><code class="language-sage">sage: C = Combinations\(range\(3\)\); C\nCombinations of \[0, 1, 2\]\n\nsage: C\.cardinality\(\)\n8<\/code><\/pre>/);
  assert.match(html, /Parameter <code>mset<\/code> is shown as inline code\./);
});
