import {
  ExecuteCommandRequest,
  type CancellationToken,
  type ExecuteCommandParams,
} from "vscode-languageserver-protocol";

import { withOperationTimeout } from "./boundedOperation";

export interface SageCommandClient {
  sendRequest(
    type: typeof ExecuteCommandRequest.type,
    params: ExecuteCommandParams,
    token?: CancellationToken,
  ): Promise<unknown>;
}

export async function executeSageCommand<T>(
  client: SageCommandClient,
  command: string,
  args: unknown[] = [],
  token?: CancellationToken,
): Promise<T | null> {
  return client.sendRequest(
    ExecuteCommandRequest.type,
    {
      command,
      arguments: args,
    },
    token,
  ) as Promise<T | null>;
}

export interface SageCommandTimeoutOptions {
  timeoutMs: number;
  label: string;
  token?: CancellationToken;
  onTimeout?: () => void;
}

export async function executeSageCommandWithTimeout<T>(
  client: SageCommandClient,
  command: string,
  args: unknown[] = [],
  options: SageCommandTimeoutOptions,
): Promise<T | null> {
  return withOperationTimeout(
    executeSageCommand<T>(client, command, args, options.token),
    options.timeoutMs,
    options.label,
    options.onTimeout,
  );
}
