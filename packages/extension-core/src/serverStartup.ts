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

/** Oldest server whose protocol and initialization options this client understands. */
export const MINIMUM_SERVER_VERSION = "0.2.0";

export function isCompatibleServerVersion(version: string, minimum = MINIMUM_SERVER_VERSION): boolean {
  const parse = (value: string): number[] => value.split(/[-+]/, 1)[0].split(".").map(Number);
  const [actual, required] = [parse(version), parse(minimum)];
  for (let index = 0; index < 3; index += 1) {
    if ((actual[index] ?? 0) !== (required[index] ?? 0)) return (actual[index] ?? 0) > (required[index] ?? 0);
  }
  return true;
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
    if (!isCompatibleServerVersion(match[1])) throw new LanguageServerLaunchError(
      `The selected sage-ls ${match[1]} is older than ${MINIMUM_SERVER_VERSION}, which this extension requires. Update the language server path or reinstall the VSIX.`,
      launch.command,
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
