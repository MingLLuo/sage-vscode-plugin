import * as vscode from "vscode";

import { readSettings } from "./configuration";
import type { OutputLogger } from "./extensionLogger";
import { discoverInterpreterCandidates, resolveInterpreterConfigurationUpdates } from "./interpreterDiscovery";
import { localizedProfileText } from "./localizedProfiles";
import type { SageTerminalManager } from "./terminalManager";
import {
  recommendedWorkspaceProfile,
  WORKSPACE_CONFIGURATION_PROFILES,
  type WorkspaceConfigurationProfile,
} from "./workspaceConfigurator";
import { workspaceConfigurationTarget, workspaceFolderPaths } from "./workspaceScope";

export interface SetupCommandDependencies {
  ensureWorkspaceRuntimeAvailable(action: string): Promise<boolean>;
  currentProjectFolder(): vscode.WorkspaceFolder | undefined;
  terminalManager: SageTerminalManager;
  logger: OutputLogger;
  showExecutionStatus(message: string, fields?: Record<string, unknown>): void;
  applyWorkspaceConfigurationProfile(
    profile: WorkspaceConfigurationProfile,
    reason: string,
    notify: boolean,
  ): Promise<unknown>;
}

/** Select Sage Interpreter and Configure Workspace: the commands that write Sage settings. */
export function registerSetupCommands(deps: SetupCommandDependencies): vscode.Disposable[] {
  return [
    vscode.commands.registerCommand("sage.selectInterpreter", async () => {
      if (!(await deps.ensureWorkspaceRuntimeAvailable("Selecting a Sage interpreter"))) {
        return;
      }
      const workspaceFolder = deps.currentProjectFolder();
      const settings = readSettings(workspaceFolder);
      const detectedOptions = discoverInterpreterCandidates({
        currentPath: settings.interpreterPath,
        workspaceFolders: workspaceFolder ? [workspaceFolder.uri.fsPath] : workspaceFolderPaths(),
      });
      const localizedOptions = detectedOptions.map((option) => option.selectionTarget === "runtimeCustom"
        ? {
          ...option,
          label: vscode.l10n.t("Enter custom Sage path..."),
          detail: vscode.l10n.t("Update sage.interpreter.path manually."),
        }
        : option);
      const picked = await vscode.window.showQuickPick(localizedOptions, {
        title: vscode.l10n.t("Select Sage environment"),
        placeHolder: vscode.l10n.t("Choose a detected Sage runtime, or enter a custom path."),
        matchOnDescription: true,
        matchOnDetail: true,
      });

      if (!picked) {
        return;
      }

      const updates = await resolveInterpreterConfigurationUpdates(picked, settings, {
        runtimePath: (initialValue) => vscode.window.showInputBox({
          title: vscode.l10n.t("Sage runtime path"),
          value: initialValue,
          prompt: vscode.l10n.t("Enter the Sage executable used for run commands and the managed REPL."),
        }),
      });
      if (!updates || updates.length === 0) {
        return;
      }

      let shouldResetRepl = false;
      for (const update of updates) {
        await vscode.workspace
          .getConfiguration("sage", workspaceFolder?.uri ?? null)
          .update(update.section, update.value, workspaceConfigurationTarget());
        deps.logger.info("configuration", "updated setting", {
          setting: `sage.${update.section}`,
          value: update.value,
        });
        shouldResetRepl ||= update.section === "interpreter.path";
      }
      if (shouldResetRepl) {
        deps.terminalManager.resetReplTerminal(workspaceFolder?.uri.fsPath);
      }
      deps.showExecutionStatus("Sage environment updated", {
        settings: updates.map((update) => `sage.${update.section}`).join(","),
      });
    }),
    vscode.commands.registerCommand("sage.configureWorkspace", async () => {
      if (!(await deps.ensureWorkspaceRuntimeAvailable("Configuring the Sage workspace"))) {
        return;
      }
      const workspaceFolder = deps.currentProjectFolder();
      if (!workspaceFolder) {
        void vscode.window.showWarningMessage(vscode.l10n.t("Open a workspace folder before configuring Sage."));
        return;
      }
      const recommended = recommendedWorkspaceProfile(vscode.window.activeTextEditor?.document.languageId);
      const picked = await vscode.window.showQuickPick(
        WORKSPACE_CONFIGURATION_PROFILES.map((profile) => {
          const text = localizedProfileText(profile);
          return {
            ...text,
            label: profile.id === recommended.id ? vscode.l10n.t("{0} (Recommended)", text.label) : text.label,
            profile,
          };
        }),
        {
          title: vscode.l10n.t("Configure Sage workspace"),
          placeHolder: vscode.l10n.t("Choose the closest profile for this workspace."),
          matchOnDescription: true,
          matchOnDetail: true,
        },
      );
      if (!picked) {
        return;
      }
      await deps.applyWorkspaceConfigurationProfile(picked.profile as WorkspaceConfigurationProfile, "configure-workspace", true);
    }),
  ];
}
