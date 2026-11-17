import { readProcessProbe, ProbeTimeoutError } from "./processProbe";
import type { LanguageServerLaunch } from "./serverLaunch";

export class LanguageServerLaunchError extends Error {
  readonly retryable = false;
  constructor(message: string, readonly command: string) {
    super(message);
    this.name = "LanguageServerLaunchError";
  }
}

export function isPermanentStartError(error: unknown): boolean {
  return error instanceof LanguageServerLaunchError;
}

/** Verify the selected executable before creating an LSP transport or entering recovery. */
export async function verifyLanguageServerLaunch(
  launch: LanguageServerLaunch,
  options: { signal?: AbortSignal; timeoutMs?: number; cwd?: string } = {},
): Promise<string> {
  try {
    const result = await readProcessProbe(launch.command, [...launch.args, "--version"], {
      ...options, timeoutMs: options.timeoutMs ?? 3_000, maxBytes: 4096,
    });
    const match = /^sage-ls (\d+\.\d+\.\d+(?:[-+][\w.-]+)?)$/.exec(result.stdout.trim());
    if (result.code !== 0 || !match) throw new LanguageServerLaunchError(
      "The selected executable did not identify itself as sage-ls. Check the language server path or reinstall the VSIX.", launch.command,
    );
    return match[1];
  } catch (error) {
    if (options.signal?.aborted || error instanceof LanguageServerLaunchError) throw error;
    const code = (error as NodeJS.ErrnoException).code;
    throw new LanguageServerLaunchError(
      error instanceof ProbeTimeoutError
        ? "The Sage language server did not finish its startup check. Check the selected executable or reinstall the matching VSIX."
        : code === "ENOENT"
          ? "The Sage language server executable was not found. Check its path or install the VSIX for this platform."
          : code === "EACCES"
            ? "The Sage language server executable cannot be run. Check its permissions or reinstall the VSIX."
            : "The Sage language server could not be launched. Check its path and platform compatibility.",
      launch.command,
    );
  }
}
