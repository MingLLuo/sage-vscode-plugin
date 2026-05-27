import * as vscode from "vscode";

import {
  buildInterpreterCommand,
  buildReplLoadCommand,
  buildReplPathBootstrapCommand,
  buildRunFileCommand,
  shouldRunInRepl,
} from "./executionPlan";
import type { RunTarget } from "./settingsModel";

export interface TerminalSettings {
  interpreterPath: string;
  interpreterArgs: readonly string[];
  runTarget: RunTarget;
  cleanupGeneratedPython: boolean;
  runtimePythonPaths?: readonly string[];
}

export class SageTerminalManager {
  private replTerminal: vscode.Terminal | undefined;
  private runTerminal: vscode.Terminal | undefined;
  private replBootstrapped = false;

  handleClosedTerminal(terminal: vscode.Terminal): void {
    if (this.replTerminal === terminal) {
      this.replTerminal = undefined;
      this.replBootstrapped = false;
    }
    if (this.runTerminal === terminal) {
      this.runTerminal = undefined;
    }
  }

  resetReplTerminal(): void {
    if (this.replTerminal) {
      this.replTerminal.dispose();
      this.replTerminal = undefined;
    }
    this.replBootstrapped = false;
  }

  runFile(settings: TerminalSettings, filePath: string): vscode.Terminal {
    const terminal = shouldRunInRepl(settings.runTarget)
      ? this.ensureReplTerminal(settings)
      : this.getOrCreateRunTerminal();
    const command = shouldRunInRepl(settings.runTarget)
      ? buildReplLoadCommand(filePath, settings.runtimePythonPaths, settings.cleanupGeneratedPython)
      : buildRunFileCommand(
          {
            interpreterPath: settings.interpreterPath,
            interpreterArgs: settings.interpreterArgs,
            cleanupGeneratedPython: settings.cleanupGeneratedPython,
            runtimePythonPaths: settings.runtimePythonPaths,
          },
          filePath,
        );
    terminal.sendText(command, true);
    return terminal;
  }

  runSelection(settings: TerminalSettings, selection: string): vscode.Terminal {
    const terminal = this.ensureReplTerminal(settings);
    const bootstrapCommand = buildReplPathBootstrapCommand(settings.runtimePythonPaths ?? []);
    if (bootstrapCommand) {
      terminal.sendText(bootstrapCommand, true);
    }
    terminal.sendText(selection, true);
    return terminal;
  }

  startRepl(settings: Pick<TerminalSettings, "interpreterPath" | "interpreterArgs">): vscode.Terminal {
    return this.ensureReplTerminal(settings);
  }

  private ensureReplTerminal(settings: {
    interpreterPath: string;
    interpreterArgs: readonly string[];
  }): vscode.Terminal {
    if (!this.replTerminal) {
      this.replTerminal = vscode.window.createTerminal("Sage REPL");
      this.replBootstrapped = false;
    }

    if (!this.replBootstrapped) {
      this.replTerminal.sendText(buildInterpreterCommand(settings), true);
      this.replBootstrapped = true;
    }

    return this.replTerminal;
  }

  private getOrCreateRunTerminal(): vscode.Terminal {
    if (!this.runTerminal) {
      this.runTerminal = vscode.window.createTerminal("Sage Run");
    }
    return this.runTerminal;
  }
}
