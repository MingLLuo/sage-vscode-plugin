import * as vscode from "vscode";

import { SAGE_INSTALL_URL, type SageRuntimeChecker } from "./sageRuntimeCheck";

/**
 * Resolves false after one actionable error when the configured Sage cannot be started, so a
 * run command fails up front with a way out instead of as a cryptic task error.
 */
export async function ensureSageRuntimeAvailable(
  checker: SageRuntimeChecker,
  interpreterPath: string,
  action: string,
): Promise<boolean> {
  const state = await checker.ensure(interpreterPath);
  if (state.status !== "missing") return true;
  const install = vscode.l10n.t("Install SageMath");
  const select = vscode.l10n.t("Select Sage Interpreter");
  void vscode.window.showErrorMessage(
    vscode.l10n.t("{0} needs SageMath. {1} Editing features keep working without it.", action, state.reason),
    install,
    select,
  ).then((choice) => {
    if (choice === install) void vscode.env.openExternal(vscode.Uri.parse(SAGE_INSTALL_URL));
    if (choice === select) void vscode.commands.executeCommand("sage.selectInterpreter");
  });
  return false;
}
