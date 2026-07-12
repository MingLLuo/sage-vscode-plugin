import { randomUUID } from "node:crypto";
import fs from "node:fs/promises";
import path from "node:path";

import * as vscode from "vscode";

import {
  buildReplLoadCommand,
  buildReplPathBootstrapCommand,
  buildRunFileProcessPlan,
  shouldRunInRepl,
} from "./executionPlan";
import { buildRunTaskDefinition, RunTaskLifecycle } from "./runTaskLifecycle";
import type { RunTarget } from "./settingsModel";

export interface TerminalSettings {
  interpreterPath: string;
  interpreterArgs: readonly string[];
  runTarget: RunTarget;
  cleanupGeneratedPython: boolean;
  runtimePythonPaths?: readonly string[];
}

export class SageTerminalManager implements vscode.Disposable {
  private replTerminal: vscode.Terminal | undefined;
  private readonly runTaskLifecycle: RunTaskLifecycle<vscode.TaskExecution>;
  private readonly taskEndSubscription: vscode.Disposable;
  private readonly runInvocationNonce = randomUUID();
  private runInvocationId = 0;

  constructor() {
    this.runTaskLifecycle = new RunTaskLifecycle(
      (cleanupPath) => this.removeGeneratedFile(cleanupPath),
      () => this.taskEndSubscription.dispose(),
    );
    this.taskEndSubscription = vscode.tasks.onDidEndTask(({ execution }) => {
      this.runTaskLifecycle.end(execution);
    });
  }

  handleClosedTerminal(terminal: vscode.Terminal): void {
    if (this.replTerminal === terminal) {
      this.replTerminal = undefined;
    }
  }

  resetReplTerminal(): void {
    if (this.replTerminal) {
      this.replTerminal.dispose();
      this.replTerminal = undefined;
    }
  }

  dispose(): void {
    this.replTerminal?.dispose();
    this.replTerminal = undefined;
    this.runTaskLifecycle.dispose();
  }

  async runFile(settings: TerminalSettings, filePath: string): Promise<vscode.Terminal | undefined> {
    this.runTaskLifecycle.assertActive();
    if (shouldRunInRepl(settings.runTarget)) {
      const terminal = this.ensureReplTerminal(settings);
      terminal.sendText(
        buildReplLoadCommand(filePath, settings.runtimePythonPaths, settings.cleanupGeneratedPython),
        true,
      );
      return terminal;
    }

    const plan = buildRunFileProcessPlan(
      {
        interpreterPath: settings.interpreterPath,
        interpreterArgs: settings.interpreterArgs,
        cleanupGeneratedPython: settings.cleanupGeneratedPython,
        runtimePythonPaths: settings.runtimePythonPaths,
      },
      filePath,
    );
    const workspaceFolder = vscode.workspace.getWorkspaceFolder(vscode.Uri.file(filePath));
    const task = new vscode.Task(
      buildRunTaskDefinition(filePath, this.runInvocationNonce, ++this.runInvocationId),
      workspaceFolder ?? vscode.TaskScope.Workspace,
      `Run ${path.basename(filePath)}`,
      "Sage",
      new vscode.ProcessExecution(plan.command, plan.args, {
        cwd: plan.cwd,
        env: plan.environment,
      }),
      [],
    );
    task.presentationOptions = {
      reveal: vscode.TaskRevealKind.Always,
      panel: vscode.TaskPanelKind.Shared,
      showReuseMessage: false,
      clear: false,
    };
    this.runTaskLifecycle.beginLaunch();
    let execution: vscode.TaskExecution;
    try {
      execution = await vscode.tasks.executeTask(task);
    } catch (error) {
      this.runTaskLifecycle.failLaunch();
      throw error;
    }
    this.runTaskLifecycle.completeLaunch(execution, plan.cleanupPath);
    return undefined;
  }

  runSelection(settings: TerminalSettings, selection: string): vscode.Terminal {
    this.runTaskLifecycle.assertActive();
    const terminal = this.ensureReplTerminal(settings);
    const bootstrapCommand = buildReplPathBootstrapCommand(settings.runtimePythonPaths ?? []);
    if (bootstrapCommand) {
      terminal.sendText(bootstrapCommand, true);
    }
    terminal.sendText(selection, true);
    return terminal;
  }

  startRepl(settings: Pick<TerminalSettings, "interpreterPath" | "interpreterArgs">): vscode.Terminal {
    this.runTaskLifecycle.assertActive();
    return this.ensureReplTerminal(settings);
  }

  private ensureReplTerminal(settings: {
    interpreterPath: string;
    interpreterArgs: readonly string[];
  }): vscode.Terminal {
    if (!this.replTerminal) {
      this.replTerminal = vscode.window.createTerminal({
        name: "Sage REPL",
        shellPath: settings.interpreterPath,
        shellArgs: [...settings.interpreterArgs],
      });
    }

    return this.replTerminal;
  }

  private removeGeneratedFile(filePath: string | undefined): void {
    if (!filePath) {
      return;
    }
    void fs.rm(filePath, { force: true }).catch(() => undefined);
  }
}
