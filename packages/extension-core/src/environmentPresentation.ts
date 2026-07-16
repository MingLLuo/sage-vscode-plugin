import path from "node:path";

import {
  formatWorkspaceRuntimeMode,
  isWorkspaceRuntimeAvailable,
  type WorkspaceRuntimeState,
} from "./workspaceTrust";

export interface EnvironmentPresentationInput {
  interpreterPath: string;
  analysisMode: string;
  docsSource: string;
  languageServerPath?: string;
  languageServerEngine?: string;
  sourceRoots: readonly string[];
  extraPaths?: readonly string[];
  indexMode?: string;
  indexStatus?: IndexStatusSummary;
  docsStatus?: DocsStatusSummary;
  runtimeIntrospectionEnabled?: boolean;
  enablePyxParsing: boolean;
  pythonFilesEnabled?: boolean;
  workspaceRuntimeState?: WorkspaceRuntimeState;
  languageServerStarting?: boolean;
  languageServerAvailable?: boolean;
}

export interface IndexStatusSummary {
  indexed_file_count?: number;
  symbol_count?: number;
  doc_count?: number;
  generation?: number;
  cache_path?: string;
  cache_namespace?: string;
  source_root_fingerprints?: readonly {
    root?: string;
    exists?: boolean;
    digest?: string;
    marker?: string | null;
  }[];
  cache_stale?: boolean;
  stale_source_roots?: readonly {
    root?: string;
    cached_digest?: string;
    current_digest?: string;
    cached_marker?: string | null;
    current_marker?: string | null;
  }[];
  cache_hit_count?: number;
  cache_miss_count?: number;
  pending_jobs?: number;
  pending_task?: "cache-check" | "rebuild" | string | null;
  last_index_ms?: number;
  last_operation?: string | null;
  last_hydrate_ms?: number;
  last_reconcile_ms?: number;
  last_persist_ms?: number;
  last_hot_cache_ms?: number;
  last_peer_seed_ms?: number;
  peer_seed_file_count?: number;
  sage_method_cache_count?: number;
  source_derived_method_cache_count?: number;
  static_method_cache_count?: number;
  hot_symbol_cache_count?: number;
  last_error?: string | null;
}

export interface IndexMaintenanceNotice {
  key: string;
  message: string;
}

export interface DocsStatusSummary {
  doc_db_path?: string;
  offline_doc_count?: number;
  preferred_source?: string;
  runtime_worker_state?: string;
  runtime_degraded_reason?: string | null;
  runtime_queue_depth?: number;
  runtime_timeout_count?: number;
  runtime_cache_hits?: number;
  runtime_cache_misses?: number;
}

export function formatStatusBarText(input: EnvironmentPresentationInput): string {
  const interpreterLabel = path.basename(input.interpreterPath) || input.interpreterPath || "sage";
  if (input.workspaceRuntimeState && !isWorkspaceRuntimeAvailable(input.workspaceRuntimeState)) {
    return "$(shield) Sage: restricted";
  }
  if (input.languageServerStarting) {
    return "$(sync~spin) Sage: starting LSP";
  }
  if (input.languageServerAvailable === false) {
    return "$(warning) Sage: LSP unavailable";
  }
  if (input.indexStatus?.last_error) {
    return "$(error) Sage: index error";
  }
  if (input.indexStatus?.cache_stale) {
    return "$(warning) Sage: cache stale";
  }
  const pendingJobs = input.indexStatus?.pending_jobs ?? 0;
  if (pendingJobs > 0) {
    if (input.indexStatus?.pending_task === "cache-check") {
      return `$(sync~spin) Sage: checking index (${pendingJobs})`;
    }
    if (input.indexStatus?.pending_task === "rebuild") {
      return `$(sync~spin) Sage: rebuilding index (${pendingJobs})`;
    }
    return `$(sync~spin) Sage: indexing (${pendingJobs})`;
  }
  return `$(beaker) Sage: ready (${interpreterLabel})`;
}

export function formatStatusBarTooltip(input: EnvironmentPresentationInput): string {
  const languageServerState = input.languageServerStarting
    ? "starting"
    : input.languageServerAvailable === false
      ? "unavailable"
      : "ready";
  const lines = [
    `Interpreter: ${input.interpreterPath}`,
    `Language server: ${input.languageServerEngine ?? "rust-v2"}${input.languageServerPath ? ` (${input.languageServerPath})` : ""}`,
    `Language server state: ${languageServerState}`,
    `Analysis mode: ${input.analysisMode}`,
    `Indexed source roots: ${input.sourceRoots.length}`,
    `Extra paths: ${input.extraPaths?.length ?? 0}`,
    `Index mode: ${input.indexMode ?? "default"}`,
    `Workspace mode: ${input.workspaceRuntimeState ? formatWorkspaceRuntimeMode(input.workspaceRuntimeState) : "trusted local workspace"}`,
    `Runtime introspection: ${input.runtimeIntrospectionEnabled === false ? "off" : "on"}`,
    `Preferred docs: ${input.docsSource}`,
    `Lightweight .pyx parsing: ${input.enablePyxParsing ? "on" : "off"}`,
    `Python files: ${input.pythonFilesEnabled ? "Sage-aware" : "Python-only"}`,
  ];
  if (input.indexStatus) {
    lines.push(
      `Index status: ${formatIndexStatusMessage(input.indexStatus)}`,
    );
  }
  if (input.docsStatus) {
    lines.push(
      `Docs status: ${formatDocsStatusMessage(input.docsStatus)}`,
    );
  }
  return lines.join("\n");
}

export function formatEnvironmentDetails(input: EnvironmentPresentationInput): string {
  const languageServerState = input.languageServerStarting
    ? "starting"
    : input.languageServerAvailable === false
      ? "unavailable"
      : "ready";
  const roots = input.sourceRoots.length > 0 ? input.sourceRoots.join(", ") : "none";
  const extraPaths = input.extraPaths && input.extraPaths.length > 0 ? input.extraPaths.join(", ") : "none";
  return [
    `Interpreter: ${input.interpreterPath}`,
    `Language server: ${input.languageServerEngine ?? "rust-v2"}${input.languageServerPath ? ` (${input.languageServerPath})` : ""}`,
    `Language server state: ${languageServerState}`,
    `Analysis: ${input.analysisMode}`,
    `Source roots: ${roots}`,
    `Extra paths: ${extraPaths}`,
    `Index mode: ${input.indexMode ?? "default"}`,
    `Workspace mode: ${input.workspaceRuntimeState ? formatWorkspaceRuntimeMode(input.workspaceRuntimeState) : "trusted local workspace"}`,
    `Runtime introspection: ${input.runtimeIntrospectionEnabled === false ? "off" : "on"}`,
    `.pyx parsing: ${input.enablePyxParsing ? "on" : "off"}`,
    `Python files: ${input.pythonFilesEnabled ? "Sage-aware" : "Python-only"}`,
    `Docs: ${input.docsSource}`,
    ...(input.indexStatus ? [`Index status: ${formatIndexStatusMessage(input.indexStatus)}`] : []),
    ...(input.docsStatus ? [`Docs status: ${formatDocsStatusMessage(input.docsStatus)}`] : []),
  ].join("\n");
}

export function formatIndexStatusMessage(input: IndexStatusSummary | null | undefined): string {
  if (!input) {
    return "Index: unavailable";
  }
  const parts = [
    `${input.indexed_file_count ?? 0} files`,
    `${input.symbol_count ?? 0} symbols`,
    `${input.doc_count ?? 0} docs`,
    `generation ${input.generation ?? 0}`,
    `${input.pending_jobs ?? 0} pending`,
    input.pending_task ? `task ${input.pending_task}` : undefined,
    `cache ${input.cache_hit_count ?? 0} hit/${input.cache_miss_count ?? 0} miss`,
    input.cache_namespace ? `namespace ${input.cache_namespace}` : undefined,
    formatRootFingerprints(input),
    formatStaleRootSummary(input),
    input.last_operation ? `op ${input.last_operation}` : undefined,
    input.peer_seed_file_count !== undefined ? `peer-seed ${input.peer_seed_file_count}` : undefined,
    input.hot_symbol_cache_count !== undefined ? `hot-cache ${input.hot_symbol_cache_count}` : undefined,
    formatMethodCacheProvenance(input),
    formatIndexTiming(input),
  ].filter((part): part is string => Boolean(part));
  if (input.last_error) {
    parts.push(`last error: ${input.last_error}`);
  }
  return `Index: ${parts.join(" | ")}`;
}

function formatMethodCacheProvenance(input: IndexStatusSummary): string | undefined {
  const total = input.sage_method_cache_count;
  if (total === undefined) {
    return undefined;
  }
  const sourceDerived = input.source_derived_method_cache_count ?? 0;
  const staticFallback = input.static_method_cache_count ?? Math.max(0, total - sourceDerived);
  return `method-cache ${sourceDerived}/${total} source-derived, ${staticFallback} static`;
}

function formatRootFingerprints(input: IndexStatusSummary): string | undefined {
  const fingerprints = input.source_root_fingerprints ?? [];
  if (fingerprints.length === 0) {
    return undefined;
  }
  const rootSummary = fingerprints.slice(0, 3).map((fingerprint) => {
    const label = fingerprint.root ? compactPathLabel(fingerprint.root) : "root";
    const digest = fingerprint.digest ?? "unknown";
    return `${label}:${digest}${fingerprint.exists === false ? ":missing" : ""}`;
  });
  const extra = fingerprints.length > rootSummary.length ? ` +${fingerprints.length - rootSummary.length}` : "";
  return `roots ${rootSummary.join(",")}${extra}`;
}

function compactPathLabel(value: string): string {
  const normalized = value.split(/[\\/]+/).filter(Boolean);
  if (normalized.length >= 2) {
    return normalized.slice(-2).join("/");
  }
  return path.basename(value) || value;
}

function formatStaleRootSummary(input: IndexStatusSummary): string | undefined {
  if (!input.cache_stale) {
    return undefined;
  }
  const stale = input.stale_source_roots ?? [];
  if (stale.length === 0) {
    return "cache stale";
  }
  const rootSummary = stale.slice(0, 3).map((entry) => {
    const label = entry.root ? compactPathLabel(entry.root) : "root";
    return `${label}:${entry.cached_digest ?? "unknown"}->${entry.current_digest ?? "unknown"}`;
  });
  const extra = stale.length > rootSummary.length ? ` +${stale.length - rootSummary.length}` : "";
  return `stale ${rootSummary.join(",")}${extra}`;
}

function formatIndexTiming(input: IndexStatusSummary): string {
  const timings = [
    formatTiming("index", input.last_index_ms),
    formatTiming("hydrate", input.last_hydrate_ms),
    formatTiming("reconcile", input.last_reconcile_ms),
    formatTiming("persist", input.last_persist_ms),
    formatTiming("peer-seed", input.last_peer_seed_ms),
    formatTiming("hot-cache", input.last_hot_cache_ms),
  ].filter((part): part is string => Boolean(part));
  return timings.length > 0 ? `timing ${timings.join(", ")}` : "timing unavailable";
}

function formatTiming(label: string, value: number | undefined): string | undefined {
  return value !== undefined && Number.isFinite(value) ? `${label} ${value}ms` : undefined;
}

export function buildIndexMaintenanceNotice(input: IndexStatusSummary | null | undefined): IndexMaintenanceNotice | undefined {
  if (!input || input.last_error || (input.pending_jobs ?? 0) > 0) {
    return undefined;
  }

  const cacheHits = input.cache_hit_count ?? 0;
  const cacheMisses = input.cache_miss_count ?? 0;
  const indexedFiles = input.indexed_file_count ?? 0;
  const symbols = input.symbol_count ?? 0;
  if (indexedFiles === 0 && symbols === 0) {
    return undefined;
  }
  const staleRoots = input.stale_source_roots?.length ?? 0;
  if (input.cache_stale !== true && staleRoots === 0) {
    return undefined;
  }

  const key = [
    input.cache_path ?? "default-cache",
    indexedFiles,
    symbols,
    input.doc_count ?? 0,
    cacheHits,
    cacheMisses,
    input.cache_stale ? "stale" : "fresh",
    staleRoots,
  ].join(":");
  const message = `Sage index cache was built for different source roots (${staleRoots || "unknown"} changed). Rebuild the full index now?`;
  return { key, message };
}

export function formatDocsStatusMessage(input: DocsStatusSummary | null | undefined): string {
  if (!input) {
    return "Docs: unavailable";
  }
  const state = input.runtime_worker_state ?? "unknown";
  const parts = [
    `${input.offline_doc_count ?? 0} offline docs`,
    `preferred ${input.preferred_source ?? "auto"}`,
    `runtime ${state}`,
    docsFallbackNote(state),
    `queue ${input.runtime_queue_depth ?? 0}`,
    `timeouts ${input.runtime_timeout_count ?? 0}`,
    `cache ${input.runtime_cache_hits ?? 0} hit/${input.runtime_cache_misses ?? 0} miss`,
  ].filter((part): part is string => Boolean(part));
  if (input.runtime_degraded_reason) {
    parts.push(`degraded: ${input.runtime_degraded_reason}`);
  }
  if (input.doc_db_path) {
    parts.push(`db ${input.doc_db_path}`);
  }
  return `Docs: ${parts.join(" | ")}`;
}

function docsFallbackNote(state: string): string | undefined {
  if (state === "idle-static-fallback" || state === "static-fallback" || state === "unconfigured-static-fallback") {
    return "static fallback active";
  }
  if (state === "disabled") {
    return "runtime lookup disabled; static fallback active";
  }
  if (state === "unavailable") {
    return "runtime lookup unavailable; static fallback active";
  }
  if (state === "degraded") {
    return "runtime lookup degraded; static fallback active";
  }
  return undefined;
}
