import test from "node:test";
import assert from "node:assert/strict";

import {
  buildIndexMaintenanceNotice,
  formatDocsStatusMessage,
  formatEnvironmentDetails,
  formatIndexStatusMessage,
  formatStatusBarText,
  formatStatusBarTooltip,
} from "../src/environmentPresentation";

const sample = {
  interpreterPath: "/opt/sage/bin/sage",
  languageServerPath: "/workspace/target/debug/sage-ls",
  languageServerEngine: "rust-v2",
  analysisMode: "default",
  docsSource: "workspace",
  sourceRoots: ["/workspace/src", "/workspace/vendor/src"],
  extraPaths: ["vendor"],
  indexMode: "deferred Sage roots with eager workspace roots",
  runtimeIntrospectionEnabled: true,
  enablePyxParsing: true,
  pythonFilesEnabled: true,
  indexStatus: {
    indexed_file_count: 18,
    symbol_count: 63,
    doc_count: 7,
    generation: 2,
    cache_hit_count: 18,
    cache_miss_count: 0,
    cache_namespace: "abc123def4567890",
    cache_stale: false,
    source_root_fingerprints: [
      {
        root: "/workspace/src",
        exists: true,
        digest: "1111222233334444",
        marker: "/workspace/src/sage/version.py",
      },
      {
        root: "/workspace/vendor/src",
        exists: true,
        digest: "aaaabbbbccccdddd",
        marker: "/workspace/vendor/src/sage/all.py",
      },
    ],
    pending_jobs: 0,
    last_index_ms: 5,
    last_operation: "reconcile",
    last_hydrate_ms: 2,
    last_reconcile_ms: 11,
    last_persist_ms: 3,
    last_peer_seed_ms: 4,
    peer_seed_file_count: 3800,
    last_hot_cache_ms: 1,
    hot_symbol_cache_count: 44,
    sage_method_cache_count: 120,
    source_derived_method_cache_count: 96,
    static_method_cache_count: 24,
  },
  docsStatus: {
    offline_doc_count: 7,
    preferred_source: "workspace",
    runtime_worker_state: "ready",
    runtime_degraded_reason: null,
    runtime_queue_depth: 0,
    runtime_timeout_count: 0,
    runtime_cache_hits: 4,
    runtime_cache_misses: 1,
  },
} as const;

test("formatStatusBarText emphasizes the selected interpreter like Python tools do", () => {
  assert.equal(formatStatusBarText(sample), "$(beaker) Sage: ready (sage)");
});

test("formatStatusBarText surfaces active indexing and index failures", () => {
  assert.equal(
    formatStatusBarText({
      ...sample,
      indexStatus: {
        ...sample.indexStatus,
        pending_jobs: 3,
        pending_task: "cache-check",
      },
    }),
    "$(sync~spin) Sage: checking index (3)",
  );
  assert.equal(
    formatStatusBarText({
      ...sample,
      indexStatus: {
        ...sample.indexStatus,
        pending_jobs: 3,
        pending_task: "rebuild",
      },
    }),
    "$(sync~spin) Sage: rebuilding index (3)",
  );
  assert.equal(
    formatStatusBarText({
      ...sample,
      indexStatus: {
        ...sample.indexStatus,
        last_error: "database is locked",
      },
    }),
    "$(error) Sage: index error",
  );
  assert.equal(
    formatStatusBarText({
      ...sample,
      indexStatus: {
        ...sample.indexStatus,
        cache_stale: true,
        stale_source_roots: [
          {
            root: "/workspace/vendor/src",
            cached_digest: "old",
            current_digest: "new",
          },
        ],
      },
    }),
    "$(warning) Sage: cache stale",
  );
});

test("formatStatusBarText surfaces restricted workspace modes", () => {
  assert.equal(
    formatStatusBarText({
      ...sample,
      workspaceRuntimeState: {
        trusted: false,
        hasVirtualWorkspace: false,
      },
    }),
    "$(shield) Sage: restricted",
  );
});

test("formatStatusBarText surfaces background language-server startup", () => {
  assert.equal(
    formatStatusBarText({
      ...sample,
      languageServerStarting: true,
    }),
    "$(sync~spin) Sage: starting LSP",
  );
});

test("formatStatusBarTooltip includes indexing and docs context", () => {
  const tooltip = formatStatusBarTooltip(sample);
  assert.match(tooltip, /Interpreter: \/opt\/sage\/bin\/sage/);
  assert.match(tooltip, /Language server: rust-v2/);
  assert.match(tooltip, /Language server state: ready or idle/);
  assert.match(tooltip, /Indexed source roots: 2/);
  assert.match(tooltip, /Extra paths: 1/);
  assert.match(tooltip, /Workspace mode: trusted local workspace/);
  assert.match(tooltip, /Runtime introspection: on/);
  assert.match(tooltip, /Preferred docs: workspace/);
  assert.match(tooltip, /Python files: Sage-aware/);
  assert.match(tooltip, /Index status: Index: 18 files/);
  assert.match(tooltip, /Docs status: Docs: 7 offline docs/);
});

test("formatEnvironmentDetails expands the configured source roots", () => {
  const detail = formatEnvironmentDetails(sample);
  assert.match(detail, /^Interpreter: /);
  assert.match(detail, /Source roots: \/workspace\/src, \/workspace\/vendor\/src/);
  assert.match(detail, /Language server: rust-v2/);
  assert.match(detail, /Language server state: ready or idle/);
  assert.match(detail, /Extra paths: vendor/);
  assert.match(detail, /Workspace mode: trusted local workspace/);
  assert.match(detail, /Index mode: deferred Sage roots with eager workspace roots/);
  assert.match(detail, /\.pyx parsing: on/);
  assert.match(detail, /Python files: Sage-aware/);
  assert.match(detail, /runtime ready/);
  assert.doesNotMatch(detail, / \| Source roots: /);
});

test("status payload formatting is concise and human-readable", () => {
  assert.match(formatIndexStatusMessage(sample.indexStatus), /18 files \| 63 symbols \| 7 docs/);
  assert.match(formatIndexStatusMessage(sample.indexStatus), /namespace abc123def4567890/);
  assert.match(formatIndexStatusMessage(sample.indexStatus), /roots workspace\/src:1111222233334444,vendor\/src:aaaabbbbccccdddd/);
  assert.match(
    formatIndexStatusMessage({
      ...sample.indexStatus,
      cache_stale: true,
      stale_source_roots: [
        {
          root: "/workspace/vendor/src",
          cached_digest: "aaaabbbbccccdddd",
          current_digest: "eeeeffff00001111",
          current_marker: "/workspace/vendor/src/sage/version.py",
        },
      ],
    }),
    /stale vendor\/src:aaaabbbbccccdddd->eeeeffff00001111/,
  );
  assert.match(formatIndexStatusMessage(sample.indexStatus), /op reconcile/);
  assert.match(formatIndexStatusMessage(sample.indexStatus), /peer-seed 3800/);
  assert.match(formatIndexStatusMessage(sample.indexStatus), /hot-cache 44/);
  assert.match(formatIndexStatusMessage(sample.indexStatus), /method-cache 96\/120 source-derived, 24 static/);
  assert.match(formatIndexStatusMessage(sample.indexStatus), /hydrate 2ms/);
  assert.match(formatIndexStatusMessage(sample.indexStatus), /reconcile 11ms/);
  assert.match(formatIndexStatusMessage(sample.indexStatus), /persist 3ms/);
  assert.match(formatIndexStatusMessage(sample.indexStatus), /peer-seed 4ms/);
  assert.match(
    formatIndexStatusMessage({
      ...sample.indexStatus,
      last_hydrate_ms: 0,
      last_reconcile_ms: 0,
      last_persist_ms: 0,
      last_peer_seed_ms: 0,
      last_hot_cache_ms: 0,
    }),
    /hydrate 0ms, reconcile 0ms, persist 0ms, peer-seed 0ms, hot-cache 0ms/,
  );
  assert.match(formatIndexStatusMessage({ ...sample.indexStatus, last_error: "db locked" }), /last error: db locked/);
  assert.match(formatDocsStatusMessage(sample.docsStatus), /runtime ready/);
  assert.match(formatDocsStatusMessage(sample.docsStatus), /preferred workspace/);
  assert.match(formatDocsStatusMessage(sample.docsStatus), /cache 4 hit\/1 miss/);
  assert.doesNotMatch(formatDocsStatusMessage(sample.docsStatus), /degraded:/);
  assert.match(
    formatDocsStatusMessage({
      ...sample.docsStatus,
      runtime_worker_state: "idle-static-fallback",
      runtime_degraded_reason: null,
    }),
    /static fallback active/,
  );
  assert.match(
    formatDocsStatusMessage({
      ...sample.docsStatus,
      runtime_worker_state: "unavailable",
      runtime_degraded_reason: "Sage interpreter path is empty",
    }),
    /runtime lookup unavailable; static fallback active/,
  );
  assert.match(
    formatDocsStatusMessage({
      ...sample.docsStatus,
      runtime_worker_state: "unavailable",
      runtime_degraded_reason: "Sage interpreter path is empty",
    }),
    /degraded: Sage interpreter path is empty/,
  );
});

test("buildIndexMaintenanceNotice prompts only for stale source-root cache", () => {
  assert.equal(buildIndexMaintenanceNotice(sample.indexStatus), undefined);
  assert.equal(buildIndexMaintenanceNotice({ ...sample.indexStatus, pending_jobs: 1, cache_miss_count: 2 }), undefined);
  assert.equal(buildIndexMaintenanceNotice({ ...sample.indexStatus, last_error: "db locked", cache_miss_count: 2 }), undefined);
  assert.equal(buildIndexMaintenanceNotice({ ...sample.indexStatus, cache_hit_count: 17, cache_miss_count: 1 }), undefined);
  assert.equal(buildIndexMaintenanceNotice({ ...sample.indexStatus, cache_hit_count: 0, cache_miss_count: 18 }), undefined);

  const stale = buildIndexMaintenanceNotice({
    ...sample.indexStatus,
    cache_path: "/tmp/shared.sqlite",
    cache_hit_count: 17,
    cache_miss_count: 1,
    cache_stale: true,
    stale_source_roots: [{ root: "/workspace/sage", cached_digest: "old", current_digest: "new" }],
  });
  assert.match(stale?.message ?? "", /different source roots/);
  assert.match(stale?.key ?? "", /shared\.sqlite/);
});
