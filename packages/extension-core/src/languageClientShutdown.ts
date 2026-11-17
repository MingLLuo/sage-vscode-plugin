import { withOperationTimeout } from "./boundedOperation";
import type { StartableLanguageClient } from "./languageClientOperations";

/** Startup settlement and stop share one deadline, rather than each spending the full budget. */
export async function settleAndStopLanguageClient(options: {
  operation?: Promise<void>;
  timeoutMs: number;
  takeClient(): StartableLanguageClient | undefined;
  afterStop(): void;
}): Promise<void> {
  const deadline = Date.now() + options.timeoutMs;
  const remaining = (): number => Math.max(0, deadline - Date.now());
  if (options.operation) {
    try {
      await withOperationTimeout(options.operation, remaining(), "Sage startup during shutdown");
    } catch { /* Still claim and stop the best available handle. */ }
  }
  const client = options.takeClient();
  if (!client) return;
  try {
    await withOperationTimeout(Promise.resolve().then(() => client.stop()), remaining(), "Sage language client stop");
  } catch { /* VS Code must be able to finish deactivation. */ }
  finally { options.afterStop(); }
}
