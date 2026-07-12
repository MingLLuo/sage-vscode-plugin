import * as path from "node:path";

import type { RunTarget } from "./settingsModel";

export interface RunFileProcessInput {
  interpreterPath: string;
  interpreterArgs: readonly string[];
  cleanupGeneratedPython?: boolean;
  runtimePythonPaths?: readonly string[];
  platform?: NodeJS.Platform;
  environment?: NodeJS.ProcessEnv;
}

export interface RunFileProcessPlan {
  command: string;
  args: string[];
  cwd: string;
  environment: Record<string, string>;
  cleanupPath?: string;
}

export function buildRunFileProcessPlan(
  input: RunFileProcessInput,
  filePath: string,
): RunFileProcessPlan {
  const platform = input.platform ?? process.platform;
  const platformPath = platform === "win32" ? path.win32 : path;
  const resolvedFilePath = platformPath.resolve(filePath);
  const pythonPaths = runtimePythonPaths(resolvedFilePath, input.runtimePythonPaths ?? [], platform);
  const inheritedPythonPath = input.environment === undefined
    ? process.env.PYTHONPATH
    : input.environment.PYTHONPATH;
  const delimiter = platform === "win32" ? ";" : ":";
  const pythonPath = [pythonPaths.join(delimiter), inheritedPythonPath]
    .filter((entry): entry is string => Boolean(entry))
    .join(delimiter);

  return {
    command: input.interpreterPath,
    args: [...input.interpreterArgs, resolvedFilePath],
    cwd: platformPath.dirname(resolvedFilePath),
    environment: pythonPath ? { PYTHONPATH: pythonPath } : {},
    cleanupPath: input.cleanupGeneratedPython && resolvedFilePath.endsWith(".sage")
      ? `${resolvedFilePath}.py`
      : undefined,
  };
}

export function buildReplLoadCommand(
  filePath: string,
  pythonPaths: readonly string[] = [],
  cleanupGeneratedPython = false,
  platform: NodeJS.Platform = process.platform,
): string {
  return [
    buildReplPathBootstrapCommand(runtimePythonPaths(filePath, pythonPaths, platform), platform),
    buildReplLoadExpression(filePath, cleanupGeneratedPython, platform),
  ].filter(Boolean).join("; ");
}

export function buildReplPathBootstrapCommand(
  pythonPaths: readonly string[],
  platform: NodeJS.Platform = process.platform,
): string {
  const platformPath = platform === "win32" ? path.win32 : path;
  const paths = dedupe(pythonPaths.map((candidate) => platformPath.resolve(candidate)));
  if (paths.length === 0) {
    return "";
  }
  return [
    "import sys as __sage_vscode_sys",
    `__sage_vscode_paths = ${JSON.stringify(paths)}`,
    "[__sage_vscode_sys.path.insert(0, __sage_vscode_path) for __sage_vscode_path in reversed(__sage_vscode_paths) if __sage_vscode_path not in __sage_vscode_sys.path]",
  ].join("; ");
}

export function shouldRunInRepl(runTarget: RunTarget): boolean {
  return runTarget === "repl";
}

function runtimePythonPaths(
  filePath: string,
  configuredPaths: readonly string[],
  platform: NodeJS.Platform,
): string[] {
  const platformPath = platform === "win32" ? path.win32 : path;
  return dedupe(
    [platformPath.dirname(filePath), ...configuredPaths]
      .map((candidate) => platformPath.resolve(candidate)),
  );
}

function buildReplLoadExpression(
  filePath: string,
  cleanupGeneratedPython: boolean,
  platform: NodeJS.Platform,
): string {
  if (!cleanupGeneratedPython || !filePath.endsWith(".sage") || platform === "win32") {
    return `load(${JSON.stringify(filePath)})`;
  }

  const generatedPath = `${filePath}.py`;
  const script = [
    "import os as __sage_vscode_os",
    `__sage_vscode_generated = ${JSON.stringify(generatedPath)}`,
    "try:",
    `    load(${JSON.stringify(filePath)})`,
    "finally:",
    "    if __sage_vscode_os.path.exists(__sage_vscode_generated):",
    "        __sage_vscode_os.remove(__sage_vscode_generated)",
  ].join("\n");
  return `exec(${JSON.stringify(script)})`;
}

function dedupe(values: readonly string[]): string[] {
  const seen = new Set<string>();
  const result: string[] = [];
  for (const value of values) {
    if (seen.has(value)) {
      continue;
    }
    seen.add(value);
    result.push(value);
  }
  return result;
}
