import { readProcessProbe, ProbeTimeoutError } from "./processProbe";

/** Whether the configured Sage executable can actually be started. */
export type SageRuntimeState =
  | { status: "checking" }
  | { status: "found"; version: string }
  | { status: "missing"; reason: string }
  | { status: "unknown"; reason: string };

export const SAGE_INSTALL_URL = "https://doc.sagemath.org/html/en/installation/index.html";

export type VersionProbe = (command: string) => Promise<{ stdout: string; code: number | null }>;

const defaultProbe: VersionProbe = (command) =>
  readProcessProbe(command, ["--version"], { timeoutMs: 15_000, maxBytes: 4096 });

/** `SageMath version 11.0, Release Date: ...` -> `SageMath 11.0`; other output -> first line. */
export function describeVersionOutput(stdout: string): string | undefined {
  const sage = /SageMath version ([^\s,]+)/.exec(stdout);
  if (sage) return `SageMath ${sage[1]}`;
  return stdout.split(/\r?\n/).map((line) => line.trim()).find(Boolean);
}

/**
 * Runs `<interpreter> --version` once per interpreter path (no Sage import, about a second)
 * and remembers the answer, so the status bar and run commands can tell "Sage is missing"
 * apart from "Sage is ready" instead of discovering it when a run fails.
 */
export class SageRuntimeChecker {
  private readonly states = new Map<string, SageRuntimeState>();
  private readonly pending = new Map<string, Promise<SageRuntimeState>>();

  constructor(
    private readonly onChange: () => void,
    private readonly probe: VersionProbe = defaultProbe,
  ) {}

  state(interpreterPath: string): SageRuntimeState | undefined {
    return this.states.get(interpreterPath);
  }

  ensure(interpreterPath: string): Promise<SageRuntimeState> {
    const known = this.states.get(interpreterPath);
    if (known && known.status !== "checking") return Promise.resolve(known);
    const running = this.pending.get(interpreterPath);
    if (running) return running;
    this.states.set(interpreterPath, { status: "checking" });
    const check = this.check(interpreterPath).then((state) => {
      this.pending.delete(interpreterPath);
      this.states.set(interpreterPath, state);
      this.onChange();
      return state;
    });
    this.pending.set(interpreterPath, check);
    return check;
  }

  /** Forgets every answer, for example after the interpreter setting changed. */
  invalidate(): void {
    this.states.clear();
    this.pending.clear();
  }

  private async check(interpreterPath: string): Promise<SageRuntimeState> {
    if (!interpreterPath.trim()) return { status: "missing", reason: "No Sage interpreter is configured." };
    try {
      const result = await this.probe(interpreterPath);
      const version = describeVersionOutput(result.stdout);
      if (result.code === 0 && version) return { status: "found", version };
      return { status: "unknown", reason: `"${interpreterPath} --version" exited with code ${result.code}.` };
    } catch (error) {
      const code = (error as NodeJS.ErrnoException).code;
      if (code === "ENOENT") return { status: "missing", reason: `No executable was found at "${interpreterPath}".` };
      if (code === "EACCES") return { status: "missing", reason: `"${interpreterPath}" cannot be executed.` };
      // A slow or unusual launcher is not proof that Sage is absent.
      return {
        status: "unknown",
        reason: error instanceof ProbeTimeoutError ? "The Sage version check timed out." : String(error),
      };
    }
  }
}
