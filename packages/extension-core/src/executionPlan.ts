import * as path from "node:path";

import type { RunTarget } from "./settingsModel";
import { buildShellCommand } from "./runtimeCommand";

export interface InterpreterCommandInput {
  interpreterPath: string;
  interpreterArgs: readonly string[];
  cleanupGeneratedPython?: boolean;
  runtimePythonPaths?: readonly string[];
  platform?: NodeJS.Platform;
}

export function buildInterpreterCommand(input: InterpreterCommandInput): string {
  return buildShellCommand([input.interpreterPath, ...input.interpreterArgs]);
}

export function buildRunFileCommand(input: InterpreterCommandInput, filePath: string): string {
  const platform = input.platform ?? process.platform;
  const command = withRuntimePythonPath(
    buildShellCommand([input.interpreterPath, ...input.interpreterArgs, filePath]),
    runtimePythonPaths(filePath, input.runtimePythonPaths ?? []),
    platform,
  );
  if (!input.cleanupGeneratedPython || !filePath.endsWith(".sage") || platform === "win32") {
    return command;
  }

  const generatedPath = buildShellCommand([`${filePath}.py`]);
  return `__sage_status=0; ${command} || __sage_status=$?; rm -f ${generatedPath}; exit $__sage_status`;
}

export function buildReplLoadCommand(
  filePath: string,
  pythonPaths: readonly string[] = [],
  cleanupGeneratedPython = false,
  platform: NodeJS.Platform = process.platform,
): string {
  return [
    buildReplPathBootstrapCommand(runtimePythonPaths(filePath, pythonPaths)),
    buildReplLoadExpression(filePath, cleanupGeneratedPython, platform),
  ].filter(Boolean).join("; ");
}

export function buildReplPathBootstrapCommand(pythonPaths: readonly string[]): string {
  const paths = dedupe(pythonPaths.map((candidate) => path.resolve(candidate)));
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

function runtimePythonPaths(filePath: string, configuredPaths: readonly string[]): string[] {
  return dedupe([path.dirname(filePath), ...configuredPaths].map((candidate) => path.resolve(candidate)));
}

function withRuntimePythonPath(command: string, pythonPaths: readonly string[], platform = process.platform): string {
  if (platform === "win32" || pythonPaths.length === 0) {
    return command;
  }

  const joinedPaths = pythonPaths.join(":");
  return `PYTHONPATH=${buildShellCommand([joinedPaths])}\${PYTHONPATH:+:$PYTHONPATH} ${command}`;
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
