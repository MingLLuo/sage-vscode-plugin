export interface WorkspaceRuntimeState {
  trusted: boolean;
  hasVirtualWorkspace: boolean;
}

export function isWorkspaceRuntimeAvailable(state: WorkspaceRuntimeState): boolean {
  return state.trusted && !state.hasVirtualWorkspace;
}

export function formatWorkspaceRuntimeMode(state: WorkspaceRuntimeState): string {
  if (!state.trusted) {
    return "restricted workspace";
  }
  if (state.hasVirtualWorkspace) {
    return "virtual workspace";
  }
  return "trusted local workspace";
}

export function formatWorkspaceRuntimeUnavailableMessage(state: WorkspaceRuntimeState, action: string): string {
  if (!state.trusted) {
    return `${action} needs a trusted workspace because Sage tooling starts local processes and can execute workspace code.`;
  }
  if (state.hasVirtualWorkspace) {
    return `${action} needs a local file workspace because Sage tooling starts local processes and indexes files from disk.`;
  }
  return `${action} is not available in the current workspace.`;
}
