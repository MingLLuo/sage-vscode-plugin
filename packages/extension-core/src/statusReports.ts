import path from "node:path";

import {
  formatDocsStatusMessage,
  formatIndexStatusMessage,
  type DocsStatusSummary,
  type IndexStatusSummary,
} from "./environmentPresentation";

export function formatIndexStatusReport(input: IndexStatusSummary | null | undefined): string {
  if (!input) {
    return "Index status is unavailable.\n";
  }

  return [
    "Summary",
    bullet("Files", input.indexed_file_count ?? 0),
    bullet("Symbols", input.symbol_count ?? 0),
    bullet("Docs", input.doc_count ?? 0),
    bullet("Generation", input.generation ?? 0),
    bullet("Pending jobs", formatPendingJobs(input)),
    "",
    "Cache",
    bullet("Namespace", input.cache_namespace ?? "default"),
    bullet("Path", input.cache_path ?? "default cache location"),
    bullet("Hits / misses", `${input.cache_hit_count ?? 0} / ${input.cache_miss_count ?? 0}`),
    bullet("Stale", input.cache_stale ? "yes" : "no"),
    bullet("Hot symbols", input.hot_symbol_cache_count ?? "unknown"),
    bullet("Peer seed files", input.peer_seed_file_count ?? "unknown"),
    bullet("Method cache", formatMethodCache(input)),
    "",
    "Timing",
    bullet("Last operation", input.last_operation ?? "unknown"),
    bullet("Index", formatMilliseconds(input.last_index_ms)),
    bullet("Hydrate", formatMilliseconds(input.last_hydrate_ms)),
    bullet("Reconcile", formatMilliseconds(input.last_reconcile_ms)),
    bullet("Persist", formatMilliseconds(input.last_persist_ms)),
    bullet("Peer seed", formatMilliseconds(input.last_peer_seed_ms)),
    bullet("Hot cache", formatMilliseconds(input.last_hot_cache_ms)),
    "",
    "Source Roots",
    ...formatSourceRoots(input),
    "",
    "Diagnostics",
    bullet("Last error", input.last_error ?? "none"),
    "",
    "Compact",
    formatIndexStatusMessage(input),
  ].join("\n");
}

export function formatDocsStatusReport(input: DocsStatusSummary | null | undefined): string {
  if (!input) {
    return "Documentation status is unavailable.\n";
  }

  return [
    "Summary",
    bullet("Offline docs", input.offline_doc_count ?? 0),
    bullet("Preferred source", input.preferred_source ?? "auto"),
    bullet("Runtime worker", input.runtime_worker_state ?? "unknown"),
    bullet("Queue depth", input.runtime_queue_depth ?? 0),
    bullet("Timeouts", input.runtime_timeout_count ?? 0),
    bullet("Cache hits / misses", `${input.runtime_cache_hits ?? 0} / ${input.runtime_cache_misses ?? 0}`),
    "",
    "Runtime Fallback",
    bullet("State", runtimeFallbackLabel(input)),
    bullet("Degraded reason", input.runtime_degraded_reason ?? "none"),
    "",
    "Storage",
    bullet("Doc DB", input.doc_db_path ?? "not reported"),
    "",
    "Compact",
    formatDocsStatusMessage(input),
  ].join("\n");
}

function bullet(label: string, value: unknown): string {
  return `- ${label}: ${String(value)}`;
}

function formatPendingJobs(input: IndexStatusSummary): string {
  const count = input.pending_jobs ?? 0;
  return input.pending_task ? `${count} (${input.pending_task})` : String(count);
}

function formatMilliseconds(value: number | undefined): string {
  return value !== undefined && Number.isFinite(value) ? `${value}ms` : "not reported";
}

function formatMethodCache(input: IndexStatusSummary): string {
  if (input.sage_method_cache_count === undefined) {
    return "unknown";
  }
  const sourceDerived = input.source_derived_method_cache_count ?? 0;
  const staticFallback = input.static_method_cache_count ?? Math.max(0, input.sage_method_cache_count - sourceDerived);
  return `${input.sage_method_cache_count} total, ${sourceDerived} source-derived, ${staticFallback} static`;
}

function formatSourceRoots(input: IndexStatusSummary): string[] {
  const fingerprints = input.source_root_fingerprints ?? [];
  const staleRoots = input.stale_source_roots ?? [];
  if (fingerprints.length === 0 && staleRoots.length === 0) {
    return [bullet("Roots", "not reported")];
  }

  const lines = fingerprints.map((fingerprint) => {
    const label = fingerprint.root ? compactPathLabel(fingerprint.root) : "root";
    const marker = fingerprint.marker ? `, marker ${compactPathLabel(fingerprint.marker)}` : "";
    return bullet(label, `digest ${fingerprint.digest ?? "unknown"}, exists ${fingerprint.exists !== false}${marker}`);
  });

  for (const stale of staleRoots) {
    const label = stale.root ? compactPathLabel(stale.root) : "root";
    lines.push(bullet(`stale ${label}`, `${stale.cached_digest ?? "unknown"} -> ${stale.current_digest ?? "unknown"}`));
  }
  return lines;
}

function runtimeFallbackLabel(input: DocsStatusSummary): string {
  const state = input.runtime_worker_state ?? "unknown";
  if (state === "ready") {
    return "runtime docs available";
  }
  if (state === "disabled") {
    return "runtime lookup disabled; static docs only";
  }
  if (state.includes("fallback") || state === "unavailable" || state === "degraded") {
    return "static fallback active";
  }
  return state;
}

function compactPathLabel(value: string): string {
  const normalized = value.split(/[\\/]+/).filter(Boolean);
  if (normalized.length >= 2) {
    return normalized.slice(-2).join(path.sep);
  }
  return path.basename(value) || value;
}
