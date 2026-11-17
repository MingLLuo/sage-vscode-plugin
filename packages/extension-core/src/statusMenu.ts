import type { StatusTranslator } from "./environmentPresentation";

export interface StatusMenuAction {
  label: string;
  description: string;
  detail: string;
  command: string;
}

export const STATUS_MENU_COMMAND = "sage.__internal.showStatusMenu";
export const SHOW_LOGS_COMMAND = "sage.__internal.showLogs";

const english: StatusTranslator = (message) => message;

/**
 * Status menu entries. Recovery actions come first: the menu is where users land when the status
 * bar shows a problem, so restarting, picking another Sage, and reading logs must be one click.
 */
export function statusMenuActions(t: StatusTranslator = english): StatusMenuAction[] {
  const action = (icon: string, label: string, description: string, detail: string, command: string): StatusMenuAction => ({
    label: `$(${icon}) ${t(label)}`,
    description: t(description),
    detail: t(detail),
    command,
  });
  return [
    action("debug-restart", "Restart Language Server", "Recover from a stopped or stuck server",
      "Stop and start sage-ls for the current project.", "sage.restartLanguageServer"),
    action("gear", "Select Sage Interpreter", "Choose the Sage used for running code",
      "Pick a detected Sage installation or enter a path.", "sage.selectInterpreter"),
    action("output", "Show Logs", "Extension and language server output",
      "Open the Sage output channel.", SHOW_LOGS_COMMAND),
    action("info", "Environment Details", "Runtime, source roots, and workspace mode",
      "Open the Sage output channel with the current workspace configuration.", "sage.showEnvironmentDetails"),
    action("database", "Index Status", "Files, symbols, cache, and pending jobs",
      "Inspect Rust index health, cache timings, and stale source-root state.", "sage.showIndexStatus"),
    action("book", "Documentation Status", "Offline docs and runtime fallback",
      "Inspect documentation cache, runtime worker state, and degraded mode.", "sage.showDocsStatus"),
    action("refresh", "Rebuild Index", "Refresh cached Sage/project symbols",
      "Rebuild the Rust index when source roots or Sage internals changed.", "sage.rebuildIndex"),
    action("archive", "Build Persistent Database", "Save Sage sources and documentation for fast lookup",
      "Discover Sage sources, build the on-disk index, and keep it across restarts and automatic cleanup.", "sage.buildDatabase"),
    action("checklist", "Run UX Self Check", "Hover, docs, definition, references, and diagnostics",
      "Run the current-file edit-loop health check and print a readable report.", "sage.runUxSelfCheck"),
    action("clippy", "Copy Support Bundle", "Troubleshooting snapshot",
      "Copy settings, status, and lifecycle data without source contents.", "sage.copySupportBundle"),
  ];
}
