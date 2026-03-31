const SERVER_RESTART_SECTIONS = [
  "sage.interpreter.path",
  "sage.interpreter.args",
  "sage.analysis",
  "sage.indexing",
  "sage.docs",
  "sage.logging",
  "sage.experimental.notebookSupport",
];

export function shouldRestartLanguageServer(
  affectsConfiguration: (section: string) => boolean,
): boolean {
  return SERVER_RESTART_SECTIONS.some((section) => affectsConfiguration(section));
}
