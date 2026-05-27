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

function numericField(record: Record<string, unknown>, key: string): number | undefined {
  const value = record[key];
  return typeof value === "number" && Number.isFinite(value) ? value : undefined;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}
