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
  });
  try {
    return await Promise.race([Promise.resolve(operation), timeout]);
  } finally {
    if (timeoutHandle) {
      clearTimeout(timeoutHandle);
    }
  }
}

export interface OperationCancellationToken {
  readonly isCancellationRequested: boolean;
  onCancellationRequested(listener: () => void): { dispose(): void };
}

/**
 * Lets a user dismiss a wait without cancelling shared background work. The
 * operation keeps a rejection handler after cancellation, preventing a later
 * failure from becoming an unhandled promise rejection.
 */
export async function waitForOperationOrCancellation(
  operation: PromiseLike<unknown>,
  token: OperationCancellationToken,
): Promise<"completed" | "cancelled"> {
  return new Promise((resolve, reject) => {
    let settled = false;
    let subscription: { dispose(): void } | undefined;
    const finish = (result: "completed" | "cancelled"): void => {
      if (settled) {
        return;
      }
      settled = true;
      subscription?.dispose();
      resolve(result);
    };
    subscription = token.onCancellationRequested(() => finish("cancelled"));
    Promise.resolve(operation).then(
      () => finish("completed"),
      (error) => {
        if (settled) {
          return;
        }
        settled = true;
        subscription?.dispose();
        reject(error);
      },
    );
    if (token.isCancellationRequested) {
      finish("cancelled");
    }
  });
}
