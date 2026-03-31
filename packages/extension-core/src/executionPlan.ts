import type { RunTarget } from "./settingsModel";
import { buildShellCommand } from "./runtimeCommand";

export interface InterpreterCommandInput {
  interpreterPath: string;
  interpreterArgs: readonly string[];
  cleanupGeneratedPython?: boolean;
  platform?: NodeJS.Platform;
}

export function buildInterpreterCommand(input: InterpreterCommandInput): string {
  return buildShellCommand([input.interpreterPath, ...input.interpreterArgs]);
}

export function buildRunFileCommand(input: InterpreterCommandInput, filePath: string): string {
  const command = buildShellCommand([input.interpreterPath, ...input.interpreterArgs, filePath]);
  if (!input.cleanupGeneratedPython || !filePath.endsWith(".sage") || input.platform === "win32") {
    return command;
  }

  const generatedPath = buildShellCommand([`${filePath}.py`]);
  return `__sage_status=0; ${command} || __sage_status=$?; rm -f ${generatedPath}; exit $__sage_status`;
}

export function buildReplLoadCommand(filePath: string): string {
  return `load(${JSON.stringify(filePath)})`;
}

export function shouldRunInRepl(runTarget: RunTarget): boolean {
  return runTarget === "repl";
}
