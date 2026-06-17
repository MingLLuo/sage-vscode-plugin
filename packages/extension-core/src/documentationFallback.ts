export const DOCUMENTATION_FALLBACK_ACTIONS = {
  showDocsStatus: "Show Docs Status",
  showIndexStatus: "Show Index Status",
  runUxSelfCheck: "Run UX Self Check",
} as const;

export type DocumentationFallbackAction =
  (typeof DOCUMENTATION_FALLBACK_ACTIONS)[keyof typeof DOCUMENTATION_FALLBACK_ACTIONS];

const ACTION_COMMANDS: Record<DocumentationFallbackAction, string> = {
  [DOCUMENTATION_FALLBACK_ACTIONS.showDocsStatus]: "sage.showDocsStatus",
  [DOCUMENTATION_FALLBACK_ACTIONS.showIndexStatus]: "sage.showIndexStatus",
  [DOCUMENTATION_FALLBACK_ACTIONS.runUxSelfCheck]: "sage.runUxSelfCheck",
};

export function documentationFallbackActions(): DocumentationFallbackAction[] {
  return [
    DOCUMENTATION_FALLBACK_ACTIONS.showDocsStatus,
    DOCUMENTATION_FALLBACK_ACTIONS.showIndexStatus,
    DOCUMENTATION_FALLBACK_ACTIONS.runUxSelfCheck,
  ];
}

export function documentationFallbackCommand(action: DocumentationFallbackAction): string {
  return ACTION_COMMANDS[action];
}

export function documentationFallbackMessage(symbol?: string): string {
  const target = symbol?.trim();
  return target
    ? `No Sage documentation available for \`${target}\`.`
    : "No Sage documentation available for the current symbol.";
}
