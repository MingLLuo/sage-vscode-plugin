export function buildShellCommand(parts: readonly string[]): string {
  return parts.map(quoteShellArg).join(" ");
}

function quoteShellArg(value: string): string {
  return /[\s"'\\]/.test(value) ? JSON.stringify(value) : value;
}

