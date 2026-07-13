import {
  ExecuteCommandRequest,
  type CancellationToken,
  type ExecuteCommandParams,
} from "vscode-languageserver-protocol";

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
