export class OperationTimeoutError extends Error {
  constructor(
    readonly operation: string,
    readonly timeoutMs: number,
  ) {
    super(`${operation} timed out after ${timeoutMs} ms`);
    this.name = "OperationTimeoutError";
  }
}

export async function withOperationTimeout<T>(
  operation: PromiseLike<T>,
  timeoutMs: number,
  label: string,
  onTimeout?: () => void,
): Promise<T> {
  if (!Number.isFinite(timeoutMs) || timeoutMs < 0) {
    throw new Error(`Expected a non-negative finite timeout for ${label}, got ${timeoutMs}`);
  }
  let timeoutHandle: ReturnType<typeof setTimeout> | undefined;
  const timeout = new Promise<never>((_resolve, reject) => {
    timeoutHandle = setTimeout(() => {
      try {
        onTimeout?.();
      } catch {
        // Timeout remains the primary failure even if cancellation cleanup is
        // already unavailable during extension-host shutdown.
      }
      reject(new OperationTimeoutError(label, timeoutMs));
    }, timeoutMs);
    timeoutHandle.unref?.();
  });
  try {
    return await Promise.race([Promise.resolve(operation), timeout]);
  } finally {
    if (timeoutHandle) {
      clearTimeout(timeoutHandle);
    }
  }
}
