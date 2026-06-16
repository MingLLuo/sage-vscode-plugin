import test from "node:test";
import assert from "node:assert/strict";

import {
  formatDocsStatusReport,
  formatIndexStatusReport,
} from "../src/statusReports";

test("formatIndexStatusReport renders a readable multi-section report", () => {
  const report = formatIndexStatusReport({
    indexed_file_count: 18,
    symbol_count: 63,
    doc_count: 7,
    generation: 2,
    pending_jobs: 1,
    pending_task: "rebuild",
    cache_path: "/tmp/sage-index.sqlite",
    cache_namespace: "abc123",
    cache_hit_count: 14,
    cache_miss_count: 4,
    cache_stale: true,
    source_root_fingerprints: [
      {
        root: "/workspace/vendor/sage/src",
        exists: true,
        digest: "11112222",
        marker: "/workspace/vendor/sage/src/sage/all.py",
      },
    ],
    stale_source_roots: [
      {
        root: "/workspace/vendor/sage/src",
        cached_digest: "old",
        current_digest: "new",
      },
    ],
    last_operation: "hydrate",
    last_hydrate_ms: 12,
    last_reconcile_ms: 34,
    last_persist_ms: 5,
    hot_symbol_cache_count: 40,
    peer_seed_file_count: 3800,
    sage_method_cache_count: 120,
    source_derived_method_cache_count: 96,
    static_method_cache_count: 24,
    last_error: "none",
  });

  assert.match(report, /^Summary\n- Files: 18/m);
  assert.match(report, /Cache\n- Namespace: abc123/);
  assert.match(report, /Timing\n- Last operation: hydrate/);
  assert.match(report, /- Hydrate: 12ms/);
  assert.match(report, /Source Roots\n- sage\/src: digest 11112222, exists true, marker sage\/all.py/);
  assert.match(report, /- stale sage\/src: old -> new/);
  assert.match(report, /Method cache: 120 total, 96 source-derived, 24 static/);
  assert.match(report, /Compact\nIndex: 18 files \| 63 symbols \| 7 docs/);
});

test("formatIndexStatusReport handles unavailable status", () => {
  assert.equal(formatIndexStatusReport(null), "Index status is unavailable.\n");
});

test("formatDocsStatusReport renders runtime fallback status", () => {
  const report = formatDocsStatusReport({
    doc_db_path: "/tmp/docs.sqlite",
    offline_doc_count: 42,
    preferred_source: "workspace",
    runtime_worker_state: "unavailable",
    runtime_degraded_reason: "Sage runtime not configured",
    runtime_queue_depth: 3,
    runtime_timeout_count: 2,
    runtime_cache_hits: 8,
    runtime_cache_misses: 5,
  });

  assert.match(report, /^Summary\n- Offline docs: 42/m);
  assert.match(report, /Runtime Fallback\n- State: static fallback active/);
  assert.match(report, /- Degraded reason: Sage runtime not configured/);
  assert.match(report, /Storage\n- Doc DB: \/tmp\/docs.sqlite/);
  assert.match(report, /Compact\nDocs: 42 offline docs \| preferred workspace/);
});

test("formatDocsStatusReport handles unavailable status", () => {
  assert.equal(formatDocsStatusReport(undefined), "Documentation status is unavailable.\n");
});
