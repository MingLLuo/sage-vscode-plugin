import * as vscode from "vscode";

import type { WorkspaceConfigurationProfile } from "./workspaceConfigurator";

/** Display text for a workspace profile in the user's VS Code language (the profile data stays English). */
export function localizedProfileText(profile: WorkspaceConfigurationProfile): {
  label: string;
  description: string;
  detail: string;
} {
  switch (profile.id) {
    case "standard":
      return {
        label: vscode.l10n.t("Standard Sage workspace"),
        description: vscode.l10n.t(".sage files and Sage source indexing"),
        detail: vscode.l10n.t("Best for worksheets, scripts, and libraries that primarily use .sage files."),
      };
    case "python":
      return {
        label: vscode.l10n.t("Sage-heavy Python workspace"),
        description: vscode.l10n.t(".py files that import sage.all"),
        detail: vscode.l10n.t(
          "Keeps Python language mode, but attaches the Sage LSP for hover, docs, definition, references, and rename.",
        ),
      };
    case "native":
      return {
        label: vscode.l10n.t("Sage native/Cython workspace"),
        description: vscode.l10n.t(".pyx, .pxd, .pxi native code"),
        detail: vscode.l10n.t(
          "Enables Cython indexing and native Sage navigation while leaving ordinary Python files alone.",
        ),
      };
    case "research":
      return {
        label: vscode.l10n.t("Full Sage research workspace"),
        description: vscode.l10n.t(".sage, Sage-heavy .py, and Cython"),
        detail: vscode.l10n.t(
          "Best for mixed research projects, notebooks with helper modules, and local Sage source checkouts.",
        ),
      };
    default:
      return { label: profile.label, description: profile.description, detail: profile.detail };
  }
}
