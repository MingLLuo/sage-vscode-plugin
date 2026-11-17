import * as vscode from "vscode";

import type { readSettings } from "./configuration";
import type { LanguageClientLifecycleController } from "./languageClientLifecycleController";
import type { StartableLanguageClient } from "./languageClientOperations";
import { shouldAutoStartLanguageClient, shouldExposeSageExperience } from "./activationPolicy";
import {
  WORKSPACE_CONFIGURATION_PROFILES,
  type WorkspaceConfigurationProfile,
  type WorkspaceConfigurationProfileId,
} from "./workspaceConfigurator";

export interface TestCommandDependencies {
  lifecycleSnapshot(): Record<string, boolean | number>;
  lifecycleController(): LanguageClientLifecycleController<StartableLanguageClient> | undefined;
  activeEditorSettings(): ReturnType<typeof readSettings>;
  activationPolicyInput(): Parameters<typeof shouldAutoStartLanguageClient>[0];
  isSageDocument(document: vscode.TextDocument | undefined): boolean;
  applyWorkspaceConfigurationProfile(
    profile: WorkspaceConfigurationProfile,
    reason: string,
    notify: boolean,
  ): Promise<unknown>;
}

/**
 * `sage.__test.*` hooks for the extension-host suite. Registered only outside production mode, so packaged users never
 * see them.
 */
export function registerTestCommands(deps: TestCommandDependencies): vscode.Disposable[] {
  return [
    vscode.commands.registerCommand("sage.__test.getLifecycleSnapshot", () => deps.lifecycleSnapshot()),
    vscode.commands.registerCommand("sage.__test.getCurrentSageContext", () => {
      const activeDocument = vscode.window.activeTextEditor?.document;
      const settings = deps.activeEditorSettings();
      const policyInput = deps.activationPolicyInput();
      return {
        languageId: activeDocument?.languageId,
        pythonFilesEnabled: settings.pythonFilesEnabled,
        sourceRootCount: settings.sourceRoots.length,
        extraPathCount: settings.extraPaths.length,
        isSageEditor: deps.isSageDocument(activeDocument),
        shouldAutoStartLanguageClient: shouldAutoStartLanguageClient(policyInput),
        shouldExposeSageExperience: shouldExposeSageExperience(policyInput),
      };
    }),
    vscode.commands.registerCommand("sage.__test.awaitLanguageClientStable", async () => {
      if (deps.lifecycleController()?.operation) {
        await deps.lifecycleController()?.operation;
      }
      return deps.lifecycleSnapshot();
    }),
    vscode.commands.registerCommand("sage.__test.restartLanguageServerAndWait", async () => {
      await deps.lifecycleController()?.start();
      return deps.lifecycleSnapshot();
    }),
    vscode.commands.registerCommand(
      "sage.__test.configureWorkspaceProfile",
      async (profileId: WorkspaceConfigurationProfileId = "research") => {
        const profile = WORKSPACE_CONFIGURATION_PROFILES.find((entry) => entry.id === profileId);
        if (!profile) {
          throw new Error(`Unknown Sage workspace profile: ${profileId}`);
        }
        return deps.applyWorkspaceConfigurationProfile(profile, "test-configure-workspace", false);
      },
    ),
  ];
}
