import fs from "node:fs";
import path from "node:path";
import { pathToFileURL } from "node:url";

export interface WorkspaceInitializationData {
  rootUri: string | null;
  folders: string[];
  sourceRoots: string[];
}

export function resolveConfiguredPaths(
  workspaceFolders: readonly string[],
  configuredPaths: readonly string[],
): string[] {
  if (configuredPaths.length === 0) {
    return [];
  }

  const normalizedFolders = dedupe(workspaceFolders.map((folder) => path.resolve(folder)));
  const resolved = configuredPaths.flatMap((candidate) => {
    if (path.isAbsolute(candidate)) {
      return [path.resolve(candidate)];
    }

    if (normalizedFolders.length > 0) {
      return normalizedFolders.map((folder) => path.resolve(folder, candidate));
    }

    return [path.resolve(candidate)];
  });

  return dedupe(resolved);
}

export function discoverSourceRoots(
  workspaceFolders: readonly string[],
  configuredSourceRoots: readonly string[],
  exists: (candidate: string) => boolean = fs.existsSync,
): string[] {
  if (configuredSourceRoots.length > 0) {
    return resolveConfiguredPaths(workspaceFolders, configuredSourceRoots);
  }

  const discovered = workspaceFolders.flatMap((folder) => {
    const sageSrcRoot = path.join(folder, "src", "sage");
    if (exists(sageSrcRoot)) {
      return [path.join(folder, "src")];
    }
    return [folder];
  });

  return dedupe(discovered.map((candidate) => path.resolve(candidate)));
}

export function buildWorkspaceInitializationData(
  workspaceFolders: readonly string[],
  configuredSourceRoots: readonly string[],
  exists: (candidate: string) => boolean = fs.existsSync,
): WorkspaceInitializationData {
  const normalizedFolders = dedupe(workspaceFolders.map((folder) => path.resolve(folder)));
  const sourceRoots = discoverSourceRoots(normalizedFolders, configuredSourceRoots, exists);

  return {
    rootUri: normalizedFolders[0] ? pathToFileURL(normalizedFolders[0]).toString() : null,
    folders: normalizedFolders.map((folder) => pathToFileURL(folder).toString()),
    sourceRoots: sourceRoots.map((sourceRoot) => pathToFileURL(sourceRoot).toString()),
  };
}

function dedupe(values: readonly string[]): string[] {
  return [...new Set(values)];
}
