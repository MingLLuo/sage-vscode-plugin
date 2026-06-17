export interface StatusMenuAction {
  label: string;
  description: string;
  detail: string;
  command: string;
}

export const STATUS_MENU_COMMAND = "sage.__internal.showStatusMenu";

export function statusMenuActions(): StatusMenuAction[] {
  return [
    {
      label: "$(info) Environment Details",
      description: "Runtime, source roots, and workspace mode",
      detail: "Open the Sage output channel with the current workspace configuration.",
      command: "sage.showEnvironmentDetails",
    },
    {
      label: "$(database) Index Status",
      description: "Files, symbols, cache, and pending jobs",
      detail: "Inspect Rust index health, cache timings, and stale source-root state.",
      command: "sage.showIndexStatus",
    },
    {
      label: "$(book) Documentation Status",
      description: "Offline docs and runtime fallback",
      detail: "Inspect documentation cache, runtime worker state, and degraded mode.",
      command: "sage.showDocsStatus",
    },
    {
      label: "$(checklist) Run UX Self Check",
      description: "Hover, docs, definition, references, and diagnostics",
      detail: "Run the current-file edit-loop health check and print a readable report.",
      command: "sage.runUxSelfCheck",
    },
    {
      label: "$(refresh) Rebuild Index",
      description: "Refresh cached Sage/project symbols",
      detail: "Rebuild the Rust index when source roots or Sage internals changed.",
      command: "sage.rebuildIndex",
    },
    {
      label: "$(clippy) Copy Support Bundle",
      description: "Troubleshooting snapshot",
      detail: "Copy settings, status, and lifecycle data without source contents.",
      command: "sage.copySupportBundle",
    },
  ];
}
