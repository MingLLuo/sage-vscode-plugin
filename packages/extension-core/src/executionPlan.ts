import type { RunTarget } from "./settingsModel";
import { buildShellCommand } from "./runtimeCommand";

export interface InterpreterCommandInput {
  interpreterPath: string;
  interpreterArgs: readonly string[];
}

export function buildInterpreterCommand(input: InterpreterCommandInput): string {
  return buildShellCommand([input.interpreterPath, ...input.interpreterArgs]);
}

export function buildRunFileCommand(input: InterpreterCommandInput, filePath: string): string {
  return buildShellCommand([input.interpreterPath, ...input.interpreterArgs, filePath]);
}

export function buildReplLoadCommand(filePath: string): string {
  return `load(${JSON.stringify(filePath)})`;
}

export function shouldRunInRepl(runTarget: RunTarget): boolean {
  return runTarget === "repl";
}
