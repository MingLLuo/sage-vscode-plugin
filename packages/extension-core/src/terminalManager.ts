import { randomUUID } from "node:crypto";
import { GeneratedFileCleanup } from "./generatedFileCleanup";
import path from "node:path";

import * as vscode from "vscode";

import {
  buildReplLoadCommand,
  buildReplInput,
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
  workspaceFolderPath?: string;
}

export class SageTerminalManager implements vscode.Disposable {
  private readonly staleScopes = new Set<string>();
  private readonly generatedCleanup = new GeneratedFileCleanup();
  private readonly replTerminals = new Map<string, { terminal: vscode.Terminal; configurationKey: string; bootstrap?: string }>();
  private readonly runTaskLifecycle: RunTaskLifecycle<vscode.TaskExecution>;
  private readonly taskEndSubscription: vscode.Disposable;
  private readonly runInvocationNonce = randomUUID();
  private runInvocationId = 0;

  constructor() {
    this.runTaskLifecycle = new RunTaskLifecycle(
      (cleanupPath) => { if (cleanupPath) this.generatedCleanup.end(cleanupPath); },
      () => this.taskEndSubscription.dispose(),
    );
    this.taskEndSubscription = vscode.tasks.onDidEndTask(({ execution }) => {
      this.runTaskLifecycle.end(execution);
    });
  }

  handleClosedTerminal(terminal: vscode.Terminal): void {
    for (const [scope, entry] of this.replTerminals) {
      if (entry.terminal === terminal) {
        this.replTerminals.delete(scope);
        this.staleScopes.delete(scope);
      }
    }
  }

  invalidateRepl(workspaceFolderPath: string): void {
    if (this.replTerminals.has(workspaceFolderPath)) this.staleScopes.add(workspaceFolderPath);
  }

  resetReplTerminal(workspaceFolderPath?: string): void {
    const scopes = workspaceFolderPath ? [workspaceFolderPath] : [...this.replTerminals.keys()];
    for (const scope of scopes) {
      const entry = this.replTerminals.get(scope);
      entry?.terminal.dispose();
      this.replTerminals.delete(scope);
      this.staleScopes.delete(scope);
    }
  }

  dispose(): void {
    this.resetReplTerminal();
    this.runTaskLifecycle.dispose();
  }

  async runFile(settings: TerminalSettings, filePath: string): Promise<vscode.Terminal | undefined> {
    this.runTaskLifecycle.assertActive();
    if (shouldRunInRepl(settings.runTarget)) {
      // Same REPL as Run Selection and Start REPL: one session per project, or one shared
      // session for files outside every folder.
      const terminal = this.ensureReplTerminal({ ...settings, workspaceFolderPath: settings.workspaceFolderPath ?? vscode.workspace.getWorkspaceFolder(vscode.Uri.file(filePath))?.uri.fsPath });
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
        workspaceFolderPath: settings.workspaceFolderPath,
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
    const helper = filePath.endsWith(".sage") ? `${filePath}.py` : undefined;
    if (helper) this.generatedCleanup.begin(helper, Boolean(plan.cleanupPath));
    this.runTaskLifecycle.beginLaunch();
    let execution: vscode.TaskExecution;
    try {
      execution = await vscode.tasks.executeTask(task);
    } catch (error) {
      if (helper) this.generatedCleanup.end(helper, true);
      this.runTaskLifecycle.failLaunch();
      throw error;
    }
    this.runTaskLifecycle.completeLaunch(execution, helper);
    return undefined;
  }

  runSelection(settings: TerminalSettings, selection: string): vscode.Terminal {
    this.runTaskLifecycle.assertActive();
    const terminal = this.ensureReplTerminal(settings);
    this.sendBootstrapOnce(settings);
    terminal.sendText(buildReplInput(selection), true);
    return terminal;
  }

  startRepl(settings: Pick<TerminalSettings, "interpreterPath" | "interpreterArgs" | "workspaceFolderPath" | "runtimePythonPaths">): vscode.Terminal {
    this.runTaskLifecycle.assertActive();
    const scope = settings.workspaceFolderPath ?? "__no_workspace__";
    if (this.staleScopes.has(scope)) this.resetReplTerminal(scope);
    const terminal = this.ensureReplTerminal(settings);
    this.sendBootstrapOnce(settings);
    return terminal;
  }

  /** Sends the import-path setup once per REPL session instead of before every snippet. */
  private sendBootstrapOnce(settings: { workspaceFolderPath?: string; runtimePythonPaths?: readonly string[] }): void {
    const entry = this.replTerminals.get(settings.workspaceFolderPath ?? "__no_workspace__");
    const bootstrap = buildReplPathBootstrapCommand(settings.runtimePythonPaths ?? []);
    if (!entry || !bootstrap || entry.bootstrap === bootstrap) return;
    entry.terminal.sendText(bootstrap, true);
    entry.bootstrap = bootstrap;
  }

  private ensureReplTerminal(settings: {
    interpreterPath: string;
    interpreterArgs: readonly string[];
    workspaceFolderPath?: string;
  }): vscode.Terminal {
    const scope = settings.workspaceFolderPath ?? "__no_workspace__";
    if (this.staleScopes.has(scope)) {
      throw new Error("Sage paths changed. Run Sage: Start REPL to restart this project’s session before running code. Restarting clears its variables.");
    }
    const configurationKey = JSON.stringify([
      settings.interpreterPath,
      ...settings.interpreterArgs,
    ]);
    const existing = this.replTerminals.get(scope);
    if (existing && existing.configurationKey !== configurationKey) {
      existing.terminal.dispose();
      this.replTerminals.delete(scope);
    }
    if (!this.replTerminals.has(scope)) {
      const terminal = vscode.window.createTerminal({
        name: settings.workspaceFolderPath ? `Sage REPL: ${path.basename(settings.workspaceFolderPath)}` : "Sage REPL",
        shellPath: settings.interpreterPath,
        shellArgs: [...settings.interpreterArgs],
        ...(settings.workspaceFolderPath ? { cwd: settings.workspaceFolderPath } : {}),
      });
      this.replTerminals.set(scope, { terminal, configurationKey });
    }

    return this.replTerminals.get(scope)!.terminal;
  }

}
