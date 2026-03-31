import path from "node:path";

export interface EnvironmentPresentationInput {
  interpreterPath: string;
  analysisMode: string;
  docsSource: string;
  sourceRoots: readonly string[];
  enablePyxParsing: boolean;
}

export function formatStatusBarText(input: EnvironmentPresentationInput): string {
  const interpreterLabel = path.basename(input.interpreterPath) || input.interpreterPath || "sage";
  return `$(beaker) Sage: ${interpreterLabel}`;
}

export function formatStatusBarTooltip(input: EnvironmentPresentationInput): string {
  return [
    `Interpreter: ${input.interpreterPath}`,
    `Analysis mode: ${input.analysisMode}`,
    `Indexed source roots: ${input.sourceRoots.length}`,
    `Preferred docs: ${input.docsSource}`,
    `Lightweight .pyx parsing: ${input.enablePyxParsing ? "on" : "off"}`,
  ].join("\n");
}

export function formatEnvironmentDetails(input: EnvironmentPresentationInput): string {
  const roots = input.sourceRoots.length > 0 ? input.sourceRoots.join(", ") : "none";
  return [
    `Interpreter: ${input.interpreterPath}`,
    `Analysis: ${input.analysisMode}`,
    `Source roots: ${roots}`,
    `.pyx parsing: ${input.enablePyxParsing ? "on" : "off"}`,
    `Docs: ${input.docsSource}`,
  ].join(" | ");
}
