import { withOperationTimeout } from "./boundedOperation";

export interface StartableLanguageClient {
  start(): PromiseLike<void>;
  stop(): PromiseLike<void>;
}

export interface LanguageClientStartOptions {
  startTimeoutMs: number;
  cleanupTimeoutMs: number;
  label?: string;
  onCleanupError?(error: unknown): void;
}

export async function startLanguageClientWithTimeout(
  client: StartableLanguageClient,
  options: LanguageClientStartOptions,
): Promise<void> {
  const label = options.label ?? "Sage language client start";
  try {
    await withOperationTimeout(
      Promise.resolve().then(() => client.start()),
      options.startTimeoutMs,
      label,
    );
  } catch (error) {
    try {
      await stopLanguageClientWithTimeout(
        client,
        options.cleanupTimeoutMs,
        `${label} cleanup`,
      );
    } catch (cleanupError) {
      options.onCleanupError?.(cleanupError);
    }
    throw error;
  }
}

export async function stopLanguageClientWithTimeout(
  client: Pick<StartableLanguageClient, "stop">,
  timeoutMs: number,
  label = "Sage language client stop",
): Promise<void> {
  await withOperationTimeout(
    Promise.resolve().then(() => client.stop()),
    timeoutMs,
    label,
  );
}
