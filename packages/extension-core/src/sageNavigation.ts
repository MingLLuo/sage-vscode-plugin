import type { QueryResponse } from "./uxSelfCheck";

export interface QuerySourceRange {
  start_line: number;
  start_character: number;
  end_line: number;
  end_character: number;
}

export interface SageDefinitionTarget {
  path: string;
  range?: QuerySourceRange;
  confidence?: string;
  reason?: string;
}

export interface LspLocationPayload {
  uri: string;
  range: {
    start: { line: number; character: number };
    end: { line: number; character: number };
  };
}

export interface ReferenceQuickPickLabel {
  label: string;
  description: string;
  detail: string;
}

export function definitionTargetFromQuery(
  query: QueryResponse | null | undefined,
  exists: (path: string) => boolean,
): SageDefinitionTarget | null {
  const definition = query?.definition;
  if (!definition?.path) {
    return null;
  }
  if (query?.resolutionConfidence === "low") {
    return null;
  }
  if (!exists(definition.path)) {
    return null;
  }
  return {
    path: definition.path,
    range: sourceRangeFromUnknown(definition.range),
    confidence: query?.resolutionConfidence ?? undefined,
    reason: query?.resolutionReason ?? undefined,
  };
}

export function sourceRangeFromUnknown(value: unknown): QuerySourceRange | undefined {
  if (!isRecord(value)) {
    return undefined;
  }
  const startLine = numericField(value, "start_line");
  const startCharacter = numericField(value, "start_character");
  const endLine = numericField(value, "end_line");
  const endCharacter = numericField(value, "end_character");
  if (
    startLine === undefined
    || startCharacter === undefined
    || endLine === undefined
    || endCharacter === undefined
  ) {
    return undefined;
  }
  return {
    start_line: startLine,
    start_character: startCharacter,
    end_line: endLine,
    end_character: endCharacter,
  };
}

export function isLspLocationPayload(value: unknown): value is LspLocationPayload {
  if (!isRecord(value) || typeof value.uri !== "string" || !isRecord(value.range)) {
    return false;
  }
  return isPositionLike(value.range.start) && isPositionLike(value.range.end);
}

export function sourceRangeFromLspLocation(value: LspLocationPayload): QuerySourceRange {
  return {
    start_line: value.range.start.line,
    start_character: value.range.start.character,
    end_line: value.range.end.line,
    end_character: value.range.end.character,
  };
}

export function referenceQuickPickLabel(
  uri: string,
  range: QuerySourceRange,
  asRelativePath: (uri: string) => string,
): ReferenceQuickPickLabel {
  const line = range.start_line + 1;
  const column = range.start_character + 1;
  const relativePath = asRelativePath(uri);
  return {
    label: `${relativePath}:${line}:${column}`,
    description: `${line}:${column}`,
    detail: uri,
  };
}

function numericField(record: Record<string, unknown>, key: string): number | undefined {
  const value = record[key];
  return typeof value === "number" && Number.isFinite(value) ? value : undefined;
}

function isPositionLike(value: unknown): value is { line: number; character: number } {
  return isRecord(value)
    && typeof value.line === "number"
    && Number.isFinite(value.line)
    && typeof value.character === "number"
    && Number.isFinite(value.character);
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}
