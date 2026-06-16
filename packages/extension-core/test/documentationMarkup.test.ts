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
  assert.match(html, /<p class="eyebrow">Sage Documentation<\/p>/);
  assert.match(html, /<p class="title">Graph<\/p>/);
  assert.match(html, /<article class="card">/);
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

test("renderDocumentationHtml uses a script-free webview shell", () => {
  const html = renderDocumentationHtml("# PolynomialRing\n\nBuild a polynomial ring.");

  assert.match(html, /Content-Security-Policy" content="default-src 'none'; style-src 'unsafe-inline';"/);
  assert.match(html, /<title>PolynomialRing<\/title>/);
  assert.doesNotMatch(html, /<script\b/);
});

test("renderDocumentationHtml handles empty docs and sanitizes language classes", () => {
  const html = renderDocumentationHtml([
    "",
    "```sage onclick=alert(1)",
    "x < y",
    "```",
  ].join("\n"));

  assert.match(html, /<p class="title">Sage Documentation<\/p>/);
  assert.match(html, /<code class="language-sage-onclick-alert-1-">x &lt; y<\/code>/);
});

test("renderDocumentationHtml escapes quoted content", () => {
  const html = renderDocumentationHtml('# "Unsafe" <Symbol>\n\nUse `"quoted"` values.');

  assert.match(html, /<title>&quot;Unsafe&quot; &lt;Symbol&gt;<\/title>/);
  assert.match(html, /<h1>&quot;Unsafe&quot; &lt;Symbol&gt;<\/h1>/);
  assert.match(html, /Use <code>&quot;quoted&quot;<\/code> values\./);
});
