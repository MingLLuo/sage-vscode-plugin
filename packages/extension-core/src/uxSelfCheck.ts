import path from "node:path";

import {
  formatDocsStatusMessage,
  formatIndexStatusMessage,
  type DocsStatusSummary,
  type IndexStatusSummary,
} from "./environmentPresentation";

export interface QueryRequestPayload {
  textDocument: {
    uri: string;
  };
  position: {
    line: number;
    character: number;
  };
  symbol?: string;
  renameTo?: string;
  mode?: "full" | "navigation" | "hover";
  features?: {
    completions?: boolean;
    references?: boolean;
    renamePreview?: boolean;
    signature?: boolean;
    diagnostics?: boolean;
  };
}

export interface QueryResponse {
  target?: {
    symbol?: string;
    dotted_symbol?: string;
    range?: unknown;
  };
  hover?: {
    markdown?: string;
    range?: unknown;
  };
  documentation?: {
    name?: string;
    module_name?: string;
    kind?: string;
    summary?: string;
    uri?: string;
  };
  definition?: {
    name?: string;
    module?: string;
    detail?: string;
    path?: string;
    range?: unknown;
  };
  completions?: Array<{
    label?: string;
    kind?: string;
    detail?: string;
  }>;
  references?: Array<{
    path?: string;
    range?: unknown;
  }>;
  rename_preview?: Array<{
    path?: string;
    range?: unknown;
    new_text?: string;
  }>;
  signature?: {
    label?: string;
    active_parameter?: number;
    documentation?: string;
  };
  diagnostics?: Array<{
    message?: string;
    code?: string;
    severity?: string;
    range?: unknown;
  }>;
  fallback_reason?: string | null;
  resolutionConfidence?: string | null;
  resolutionReason?: string | null;
  ownerType?: string | null;
  candidateCount?: number;
}

export interface EditorDiagnosticSummary {
  source?: string;
  code?: string | number;
  severity?: string | number;
  range?: string;
  message?: string;
}

export interface UxSelfCheckTimings {
  queryMs?: number;
  fullQueryMs?: number;
  indexStatusMs?: number;
  docsStatusMs?: number;
  totalMs?: number;
}

export interface UxSelfCheckInput {
  documentUri: string;
  symbol?: string;
  query: QueryResponse | null;
  indexStatus?: IndexStatusSummary | null;
  docsStatus?: DocsStatusSummary | null;
  editorDiagnostics?: readonly EditorDiagnosticSummary[];
  timings?: UxSelfCheckTimings;
}

export interface UxSelfCheckResult {
  passed: number;
  total: number;
  report: string;
}

export function shouldRunFullUxSelfCheckQuery(
  query: QueryResponse | null | undefined,
  workspaceFolders: readonly string[],
): boolean {
  const definitionPath = query?.definition?.path;
  if (!definitionPath) {
    return false;
  }
  const normalizedDefinition = path.resolve(definitionPath);
  return workspaceFolders
    .map((folder) => path.resolve(folder))
    .some((folder) => normalizedDefinition === folder
      || normalizedDefinition.startsWith(`${folder}${path.sep}`));
}

export function diagnosticCodeLabel(code: unknown): string | number | undefined {
  if (typeof code === "string" || typeof code === "number") {
    return code;
  }
  if (code && typeof code === "object" && "value" in code) {
    const value = code.value;
    return typeof value === "string" || typeof value === "number" ? value : String(value);
  }
  return undefined;
}

export function diagnosticRangeLabel(range: {
  start: { line: number; character: number };
  end: { line: number; character: number };
}): string {
  return `${range.start.line}:${range.start.character}-${range.end.line}:${range.end.character}`;
}

export async function measureAsync<T>(
  operation: () => Promise<T>,
): Promise<{ value: T; elapsedMs: number }> {
  const started = Date.now();
  const value = await operation();
  return { value, elapsedMs: Date.now() - started };
}

interface UxCheck {
  name: string;
  pass: boolean;
  detail: string;
}

export function buildQueryRequestPayload(
  uri: string,
  line: number,
  character: number,
  symbol?: string,
  renameOrOptions: string | {
    renameTo?: string;
    mode?: QueryRequestPayload["mode"];
    features?: QueryRequestPayload["features"];
  } = "sage_ux_preview",
): QueryRequestPayload {
  const options = typeof renameOrOptions === "string"
    ? { renameTo: renameOrOptions }
    : renameOrOptions;
  return {
    textDocument: { uri },
    position: { line, character },
    ...(symbol ? { symbol } : {}),
    ...(options.renameTo ? { renameTo: options.renameTo } : {}),
    ...(options.mode && options.mode !== "full" ? { mode: options.mode } : {}),
    ...(options.features ? { features: options.features } : {}),
  };
}

export function formatUxSelfCheckReport(input: UxSelfCheckInput): UxSelfCheckResult {
  const query = input.query;
  const editorDiagnostics = dedupeEditorDiagnostics(input.editorDiagnostics);
  const rawEditorDiagnosticCount = input.editorDiagnostics?.length ?? 0;
  const checks = buildChecks({ ...input, editorDiagnostics });
  const passed = checks.filter((check) => check.pass).length;
  const target = query?.target?.dotted_symbol ?? query?.target?.symbol ?? input.symbol ?? "unknown";
  const lines = [
    "Sage UX Self Check",
    `Document: ${input.documentUri}`,
    `Target: ${target}`,
    `Result: ${passed}/${checks.length} checks passing`,
    "",
    formatIndexStatusMessage(input.indexStatus),
    formatDocsStatusMessage(input.docsStatus),
    formatEditorDiagnostics(editorDiagnostics, rawEditorDiagnosticCount),
    formatTimings(input.timings),
    "",
    "Checks:",
    ...checks.map((check) => `${check.pass ? "PASS" : "WARN"} ${check.name}: ${check.detail}`),
    "",
    "Query summary:",
    `Hover: ${truncate(query?.hover?.markdown)}`,
    `Docs: ${truncate(query?.documentation?.summary)}`,
    `Definition: ${formatDefinition(query)}`,
    `Signature: ${query?.signature?.label ?? "none"}`,
    `Fallback: ${query?.fallback_reason ?? "none"}`,
  ];
  return {
    passed,
    total: checks.length,
    report: lines.join("\n"),
  };
}

function buildChecks(input: UxSelfCheckInput): UxCheck[] {
  const query = input.query;
  if (!query) {
    return [
      {
        name: "query response",
        pass: false,
        detail: "language server returned no query payload",
      },
    ];
  }
  const diagnostics = query.diagnostics ?? [];
  const editorDiagnostics = input.editorDiagnostics ?? [];
  const references = query.references ?? [];
  const renameEdits = query.rename_preview ?? [];
  const completions = query.completions ?? [];
  const readOnlySageApi = isReadOnlySageApiTarget(input);
  const sageOwnedEditorDiagnostics = editorDiagnostics.filter(isSageOwnedEditorDiagnostic);
  const checks: UxCheck[] = [
    {
      name: "hover",
      pass: Boolean(query.hover?.markdown),
      detail: query.hover?.markdown ? "available" : "missing",
    },
    {
      name: "documentation",
      pass: Boolean(query.documentation?.summary || query.documentation?.name),
      detail: query.documentation?.summary ?? query.documentation?.name ?? "missing",
    },
    {
      name: "definition",
      pass: Boolean(query.definition?.path) || Boolean(query.documentation?.summary),
      detail: query.definition?.path ?? "no source path; docs fallback used",
    },
    {
      name: "completion",
      pass: completions.length > 0 || readOnlySageApi,
      detail: completions.length > 0 ? `${completions.length} items` : readOnlySageApi ? "not applicable for read-only Sage API" : "0 items",
    },
    {
      name: "references",
      pass: references.length > 0 || readOnlySageApi,
      detail: references.length > 0 ? `${references.length} references` : readOnlySageApi ? "not applicable for read-only Sage API" : "0 references",
    },
    {
      name: "rename preview",
      pass: (renameEdits.length === references.length && references.length > 0) || readOnlySageApi,
      detail: renameEdits.length > 0 ? `${renameEdits.length} edits` : readOnlySageApi ? "not applicable for read-only Sage API" : "0 edits",
    },
    {
      name: "signature",
      pass: Boolean(query.signature?.label) || Boolean(query.documentation?.summary),
      detail: query.signature?.label ?? "no signature; docs fallback used",
    },
    {
      name: "diagnostics",
      pass: diagnostics.length === 0,
      detail: diagnostics.length === 0 ? "none" : diagnostics.map((diagnostic) => diagnostic.message ?? diagnostic.code ?? "diagnostic").join(" | "),
    },
    {
      name: "editor diagnostic ownership",
      pass: sageOwnedEditorDiagnostics.length === 0,
      detail: formatEditorDiagnosticOwnership(editorDiagnostics, sageOwnedEditorDiagnostics),
    },
    {
      name: "fallback reason",
      pass: !query.fallback_reason,
      detail: query.fallback_reason ?? "none",
    },
  ];
  if (typeof input.timings?.queryMs === "number") {
    checks.push({
      name: "query latency",
      pass: input.timings.queryMs <= 250,
      detail: `${input.timings.queryMs}ms`,
    });
  }
  if (typeof input.timings?.fullQueryMs === "number") {
    checks.push({
      name: "full edit-loop latency",
      pass: input.timings.fullQueryMs <= 1000,
      detail: `${input.timings.fullQueryMs}ms`,
    });
  }
  return checks;
}

function formatEditorDiagnostics(
  diagnostics: readonly EditorDiagnosticSummary[] | undefined,
  rawCount = diagnostics?.length ?? 0,
): string {
  if (!diagnostics || diagnostics.length === 0) {
    return "Editor diagnostics: none";
  }

  const bySource = new Map<string, number>();
  for (const diagnostic of diagnostics) {
    const source = diagnostic.source?.trim() || "unknown";
    bySource.set(source, (bySource.get(source) ?? 0) + 1);
  }
  const sourceSummary = [...bySource.entries()]
    .sort(([left], [right]) => left.localeCompare(right))
    .map(([source, count]) => `${source} ${count}`)
    .join(", ");
  const dedupeSuffix = rawCount > diagnostics.length ? `; deduped from ${rawCount} raw` : "";
  const examples = diagnostics
    .slice(0, 3)
    .map((diagnostic) =>
      [
        diagnostic.source?.trim() || "unknown",
        diagnostic.code === undefined ? undefined : String(diagnostic.code),
        truncate(diagnostic.message, 120),
      ].filter(Boolean).join(": ")
    )
    .join(" | ");
  return `Editor diagnostics: ${diagnostics.length} total (${sourceSummary}${dedupeSuffix})${examples ? ` | ${examples}` : ""}`;
}

function formatTimings(timings: UxSelfCheckTimings | undefined): string {
  if (!timings) {
    return "Timings: not measured";
  }
  const parts = [
    formatTimingPart("query", timings.queryMs),
    formatTimingPart("full edit-loop", timings.fullQueryMs),
    formatTimingPart("index", timings.indexStatusMs),
    formatTimingPart("docs", timings.docsStatusMs),
    formatTimingPart("total", timings.totalMs),
  ].filter(Boolean);
  return `Timings: ${parts.length > 0 ? parts.join(" | ") : "not measured"}`;
}

function formatTimingPart(label: string, value: number | undefined): string | undefined {
  return typeof value === "number" ? `${label} ${value}ms` : undefined;
}

function dedupeEditorDiagnostics(
  diagnostics: readonly EditorDiagnosticSummary[] | undefined,
): readonly EditorDiagnosticSummary[] {
  if (!diagnostics || diagnostics.length <= 1) {
    return diagnostics ?? [];
  }

  const seen = new Set<string>();
  const unique: EditorDiagnosticSummary[] = [];
  for (const diagnostic of diagnostics) {
    const key = [
      diagnostic.source?.trim() ?? "",
      diagnostic.code === undefined ? "" : String(diagnostic.code),
      diagnostic.severity === undefined ? "" : String(diagnostic.severity),
      diagnostic.range ?? "",
      diagnostic.message ?? "",
    ].join("\u{1f}");
    if (seen.has(key)) {
      continue;
    }
    seen.add(key);
    unique.push(diagnostic);
  }
  return unique;
}

function formatEditorDiagnosticOwnership(
  diagnostics: readonly EditorDiagnosticSummary[],
  sageOwnedDiagnostics: readonly EditorDiagnosticSummary[],
): string {
  if (diagnostics.length === 0) {
    return "none";
  }
  const thirdPartyDiagnostics = diagnostics.length - sageOwnedDiagnostics.length;
  if (sageOwnedDiagnostics.length === 0) {
    return `${thirdPartyDiagnostics} third-party diagnostics; Sage LSP diagnostics are clean`;
  }
  return `${sageOwnedDiagnostics.length} Sage LSP diagnostics, ${thirdPartyDiagnostics} third-party diagnostics`;
}

function isSageOwnedEditorDiagnostic(diagnostic: EditorDiagnosticSummary): boolean {
  const source = diagnostic.source?.toLowerCase().trim() ?? "";
  return source === "sage-ls" || source === "sage" || source === "sage language server";
}

function isReadOnlySageApiTarget(input: UxSelfCheckInput): boolean {
  const query = input.query;
  if (!query?.definition?.path) {
    return false;
  }
  const definitionPath = query.definition.path;
  const moduleName = query.definition.module ?? query.documentation?.module_name ?? "";
  const documentPath = pathFromFileUri(input.documentUri);
  if (documentPath && normalizePath(definitionPath) === normalizePath(documentPath)) {
    return false;
  }
  return moduleName.startsWith("sage.") || normalizePath(definitionPath).includes("/sage/src/sage/");
}

function pathFromFileUri(uri: string): string | null {
  if (!uri.startsWith("file://")) {
    return null;
  }
  try {
    return decodeURIComponent(uri.slice("file://".length));
  } catch {
    return uri.slice("file://".length);
  }
}

function normalizePath(path: string): string {
  return path.replace(/\\/g, "/");
}

function formatDefinition(query: QueryResponse | null): string {
  if (!query?.definition) {
    return "none";
  }
  return [
    query.definition.name,
    query.definition.module,
    query.definition.path,
  ].filter(Boolean).join(" | ");
}

function truncate(value: string | undefined, limit = 180): string {
  if (!value) {
    return "none";
  }
  const normalized = value.replace(/\s+/g, " ").trim();
  return normalized.length > limit ? `${normalized.slice(0, limit - 3)}...` : normalized;
}
