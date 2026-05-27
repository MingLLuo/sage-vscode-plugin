import test from "node:test";
import assert from "node:assert/strict";

import {
  buildQueryRequestPayload,
  formatUxSelfCheckReport,
  type QueryResponse,
} from "../src/uxSelfCheck";

test("buildQueryRequestPayload mirrors the active editor position and rename preview target", () => {
  assert.deepEqual(
    buildQueryRequestPayload("file:///workspace/demo.sage", 7, 13, "PolynomialRing"),
    {
      textDocument: {
        uri: "file:///workspace/demo.sage",
      },
      position: {
        line: 7,
        character: 13,
      },
      symbol: "PolynomialRing",
      renameTo: "sage_ux_preview",
    },
  );
});

test("buildQueryRequestPayload can request the navigation-only path", () => {
  assert.deepEqual(
    buildQueryRequestPayload("file:///workspace/demo.sage", 7, 13, "PolynomialRing", { mode: "navigation" }),
    {
      textDocument: {
        uri: "file:///workspace/demo.sage",
      },
      position: {
        line: 7,
        character: 13,
      },
      symbol: "PolynomialRing",
      mode: "navigation",
    },
  );
});

test("formatUxSelfCheckReport summarizes the full edit loop", () => {
  const query: QueryResponse = {
    target: {
      symbol: "make_demo_matrix",
    },
    hover: {
      markdown: "Return a small nested list.",
    },
    documentation: {
      name: "make_demo_matrix",
      module_name: "local_docs",
      kind: "Function",
      summary: "Return a small nested list.",
      uri: "file:///workspace/src/local_docs.py",
    },
    definition: {
      name: "make_demo_matrix",
      module: "local_docs",
      detail: "Function make_demo_matrix",
      path: "/workspace/src/local_docs.py",
    },
    completions: [
      {
        label: "make_demo_matrix",
        kind: "Function",
        detail: "Function make_demo_matrix",
      },
    ],
    references: [
      {
        path: "/workspace/src/local_docs.py",
      },
      {
        path: "/workspace/src/01_hover_and_definition.sage",
      },
    ],
    rename_preview: [
      {
        path: "/workspace/src/local_docs.py",
        new_text: "sage_ux_preview",
      },
      {
        path: "/workspace/src/01_hover_and_definition.sage",
        new_text: "sage_ux_preview",
      },
    ],
    signature: {
      label: "make_demo_matrix()",
    },
    diagnostics: [],
  };

  const result = formatUxSelfCheckReport({
    documentUri: "file:///workspace/src/01_hover_and_definition.sage",
    query,
    indexStatus: {
      indexed_file_count: 18,
      symbol_count: 63,
      doc_count: 7,
      generation: 3,
      pending_jobs: 0,
      cache_hit_count: 18,
      cache_miss_count: 0,
      last_index_ms: 5,
    },
    docsStatus: {
      offline_doc_count: 7,
      runtime_worker_state: "ready",
      runtime_degraded_reason: null,
      runtime_queue_depth: 0,
      runtime_timeout_count: 0,
      runtime_cache_hits: 4,
      runtime_cache_misses: 1,
    },
    editorDiagnostics: [],
  });

  assert.equal(result.passed, result.total);
  assert.match(result.report, /Sage UX Self Check/);
  assert.match(result.report, /Result: 10\/10 checks passing/);
  assert.match(result.report, /Index: 18 files/);
  assert.match(result.report, /Docs: 7 offline docs/);
  assert.match(result.report, /Editor diagnostics: none/);
  assert.match(result.report, /Timings: not measured/);
  assert.match(result.report, /Definition: make_demo_matrix \| local_docs \| \/workspace\/src\/local_docs.py/);
});

test("formatUxSelfCheckReport reports request timings and warns on slow queries", () => {
  const fast = formatUxSelfCheckReport({
    documentUri: "file:///workspace/demo.sage",
    query: {
      target: { symbol: "PolynomialRing" },
      hover: { markdown: "Function PolynomialRing" },
      documentation: { name: "PolynomialRing", summary: "Return a polynomial ring." },
      definition: { path: "/sage/src/sage/rings/polynomial/polynomial_ring_constructor.py" },
      completions: [{ label: "PolynomialRing" }],
      references: [],
      rename_preview: [],
      signature: { label: "PolynomialRing(base_ring, *args, **kwds)" },
      diagnostics: [],
    },
    timings: {
      queryMs: 32,
      indexStatusMs: 2,
      docsStatusMs: 1,
      totalMs: 40,
    },
  });

  assert.equal(fast.passed, fast.total);
  assert.match(fast.report, /Timings: query 32ms \| index 2ms \| docs 1ms \| total 40ms/);
  assert.match(fast.report, /PASS query latency: 32ms/);

  const slow = formatUxSelfCheckReport({
    documentUri: "file:///workspace/demo.sage",
    query: {
      target: { symbol: "PolynomialRing" },
      hover: { markdown: "Function PolynomialRing" },
      documentation: { name: "PolynomialRing", summary: "Return a polynomial ring." },
      definition: { path: "/sage/src/sage/rings/polynomial/polynomial_ring_constructor.py" },
      completions: [{ label: "PolynomialRing" }],
      references: [],
      rename_preview: [],
      signature: { label: "PolynomialRing(base_ring, *args, **kwds)" },
      diagnostics: [],
    },
    timings: {
      queryMs: 251,
      totalMs: 260,
    },
  });

  assert.ok(slow.passed < slow.total);
  assert.match(slow.report, /WARN query latency: 251ms/);
});

test("formatUxSelfCheckReport surfaces degraded query paths as warnings", () => {
  const result = formatUxSelfCheckReport({
    documentUri: "file:///workspace/demo.sage",
    query: {
      completions: [],
      references: [],
      rename_preview: [],
      diagnostics: [
        {
          message: "Syntax error: source could not be parsed",
          code: "syntax-error",
        },
      ],
      fallback_reason: "symbol-not-in-index-or-known-sage-set",
    },
  });

  assert.ok(result.passed < result.total);
  assert.match(result.report, /WARN hover: missing/);
  assert.match(result.report, /WARN diagnostics: Syntax error/);
  assert.match(result.report, /Fallback: symbol-not-in-index-or-known-sage-set/);
});

test("formatUxSelfCheckReport separates third-party editor diagnostics from Sage LSP health", () => {
  const result = formatUxSelfCheckReport({
    documentUri: "file:///workspace/10_sage_heavy_python.py",
    query: {
      target: {
        symbol: "PolynomialRing",
      },
      hover: {
        markdown: "Function PolynomialRing",
      },
      documentation: {
        name: "PolynomialRing",
        module_name: "sage.rings.polynomial.polynomial_ring_constructor",
        summary: "Return a polynomial ring.",
      },
      definition: {
        name: "PolynomialRing",
        module: "sage.rings.polynomial.polynomial_ring_constructor",
        path: "/opt/sage/src/sage/rings/polynomial/polynomial_ring_constructor.py",
      },
      completions: [{ label: "PolynomialRing" }],
      references: [],
      rename_preview: [],
      signature: {
        label: "PolynomialRing(base_ring, *args, **kwds)",
      },
      diagnostics: [],
    },
    editorDiagnostics: [
      {
        source: "Ruff",
        code: "F401",
        severity: "Warning",
        range: "18:4-18:18",
        message: "`sage.all.PolynomialRing` imported but unused",
      },
      {
        source: "Ruff",
        code: "F401",
        severity: "Warning",
        range: "17:4-17:6",
        message: "`sage.all.GF` imported but unused",
      },
      {
        source: "Ruff",
        code: "F401",
        severity: "Warning",
        range: "17:4-17:6",
        message: "`sage.all.GF` imported but unused",
      },
    ],
  });

  assert.equal(result.passed, result.total);
  assert.match(result.report, /Editor diagnostics: 2 total \(Ruff 2; deduped from 3 raw\)/);
  assert.match(result.report, /PASS editor diagnostic ownership: 2 third-party diagnostics; Sage LSP diagnostics are clean/);
});

test("formatUxSelfCheckReport treats read-only Sage API references and rename as not applicable", () => {
  const result = formatUxSelfCheckReport({
    documentUri: "file:///workspace/07_symbolic_and_combinatorics.sage",
    query: {
      target: {
        symbol: "Combinations",
      },
      hover: {
        markdown: "Function Combinations",
      },
      documentation: {
        name: "Combinations",
        module_name: "sage.combinat.combination",
        kind: "Function",
        summary: "Return the combinatorial class of combinations of the multiset.",
        uri: "file:///workspace/sage/src/sage/combinat/combination.py",
      },
      definition: {
        name: "Combinations",
        module: "sage.combinat.combination",
        detail: "Function Combinations",
        path: "/workspace/sage/src/sage/combinat/combination.py",
      },
      completions: [],
      references: [],
      rename_preview: [],
      signature: {
        label: "Combinations(mset, k=None, *, as_tuples=False)",
      },
      diagnostics: [],
    },
  });

  assert.equal(result.passed, result.total);
  assert.match(result.report, /Result: 10\/10 checks passing/);
  assert.match(result.report, /PASS completion: not applicable for read-only Sage API/);
  assert.match(result.report, /PASS references: not applicable for read-only Sage API/);
  assert.match(result.report, /PASS rename preview: not applicable for read-only Sage API/);
});
