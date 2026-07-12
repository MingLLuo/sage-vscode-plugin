import type { IndexStatusSummary } from "./environmentPresentation";

export const DEFAULT_INDEX_REBUILD_TIMEOUT_MS = 120_000;
export const DEFAULT_INDEX_REBUILD_POLL_INTERVAL_MS = 250;
export const DEFAULT_INDEX_REBUILD_MAX_RESCHEDULES = 3;

export interface WaitForIndexRebuildOptions {
  baselineGeneration: number;
  readStatus: () => Promise<IndexStatusSummary | null | undefined>;
  reschedule?: () => Promise<void>;
  onStatus?: (status: IndexStatusSummary) => void;
  onReschedule?: (attempt: number, supersededStatus: IndexStatusSummary) => void;
  timeoutMs?: number;
  pollIntervalMs?: number;
  maxReschedules?: number;
  sleep?: (milliseconds: number) => Promise<void>;
}

export function isIndexRebuildComplete(
  status: IndexStatusSummary | null | undefined,
  baselineGeneration: number,
): boolean {
  return Boolean(
    status
    && status.pending_jobs === 0
    && status.last_operation === "rebuild"
    && typeof status.generation === "number"
    && Number.isFinite(status.generation)
    && status.generation > baselineGeneration,
  );
}

export async function waitForIndexRebuild(
  options: WaitForIndexRebuildOptions,
): Promise<IndexStatusSummary> {
  const timeoutMs = options.timeoutMs ?? DEFAULT_INDEX_REBUILD_TIMEOUT_MS;
  const pollIntervalMs = options.pollIntervalMs ?? DEFAULT_INDEX_REBUILD_POLL_INTERVAL_MS;
  const maxReschedules = options.maxReschedules ?? DEFAULT_INDEX_REBUILD_MAX_RESCHEDULES;
  const sleep = options.sleep ?? delay;
  let lastStatus: IndexStatusSummary | undefined;
  let attemptBaselineGeneration = options.baselineGeneration;
  let reschedules = 0;
  let timedOut = false;
  let timeoutHandle: ReturnType<typeof setTimeout> | undefined;

  const timeout = new Promise<never>((_resolve, reject) => {
    timeoutHandle = setTimeout(() => {
      timedOut = true;
      reject(indexRebuildTimeoutError(timeoutMs, options.baselineGeneration, lastStatus));
    }, timeoutMs);
  });

  const poll = async (): Promise<IndexStatusSummary> => {
    while (!timedOut) {
      const status = await options.readStatus();
      if (timedOut) {
        throw indexRebuildTimeoutError(timeoutMs, options.baselineGeneration, lastStatus);
      }
      if (status) {
        lastStatus = status;
        options.onStatus?.(status);
      }
      if (isIndexRebuildComplete(status, attemptBaselineGeneration)) {
        return status as IndexStatusSummary;
      }
      if (indexAdvancedWithoutInstalledRebuild(status, attemptBaselineGeneration)) {
        if (!options.reschedule || reschedules >= maxReschedules) {
          throw indexRebuildSupersededError(
            options.baselineGeneration,
            status,
            reschedules,
          );
        }
        attemptBaselineGeneration = status.generation;
        reschedules += 1;
        options.onReschedule?.(reschedules, status);
        await options.reschedule();
        continue;
      }
      await sleep(pollIntervalMs);
    }
    throw indexRebuildTimeoutError(timeoutMs, options.baselineGeneration, lastStatus);
  };

  try {
    return await Promise.race([poll(), timeout]);
  } finally {
    if (timeoutHandle) {
      clearTimeout(timeoutHandle);
    }
  }
}

function indexAdvancedWithoutInstalledRebuild(
  status: IndexStatusSummary | null | undefined,
  baselineGeneration: number,
): status is IndexStatusSummary & { generation: number } {
  return Boolean(
    status
    && status.pending_jobs === 0
    && status.last_operation !== "rebuild"
    && typeof status.generation === "number"
    && Number.isFinite(status.generation)
    && status.generation > baselineGeneration,
  );
}

function indexRebuildSupersededError(
  originalBaselineGeneration: number,
  status: IndexStatusSummary,
  reschedules: number,
): Error {
  return new Error(
    `Sage index rebuild was superseded ${reschedules + 1} times by ${status.last_operation ?? "another index operation"} `
      + `(baseline generation ${originalBaselineGeneration}, latest ${status.generation ?? "unknown"}).`,
  );
}

function indexRebuildTimeoutError(
  timeoutMs: number,
  baselineGeneration: number,
  status: IndexStatusSummary | undefined,
): Error {
  const latestGeneration = status?.generation ?? "unknown";
  const pendingJobs = status?.pending_jobs ?? "unknown";
  const lastError = status?.last_error ? ` Last server error: ${status.last_error}` : "";
  return new Error(
    `Timed out after ${timeoutMs} ms waiting for the Sage index to rebuild `
      + `(baseline generation ${baselineGeneration}, latest ${latestGeneration}, pending jobs ${pendingJobs}).${lastError}`,
  );
}

async function delay(milliseconds: number): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, milliseconds));
}
