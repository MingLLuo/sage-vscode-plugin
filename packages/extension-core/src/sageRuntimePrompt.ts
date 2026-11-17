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
  void vscode.window.showErrorMessage(
    `${action} needs SageMath, but ${state.reason} Editing features keep working without it.`,
    "Install SageMath",
    "Select Sage Interpreter",
  ).then((choice) => {
    if (choice === "Install SageMath") void vscode.env.openExternal(vscode.Uri.parse(SAGE_INSTALL_URL));
    if (choice === "Select Sage Interpreter") void vscode.commands.executeCommand("sage.selectInterpreter");
  });
  return false;
}
