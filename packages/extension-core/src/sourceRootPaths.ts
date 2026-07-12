import { realpathSync } from "node:fs";
import { fileURLToPath } from "node:url";
import * as path from "node:path";

export interface EffectiveSourceRootPathsInput {
  configuredRoots: readonly string[];
  indexedRoots: readonly string[];
  workspaceFolders: readonly string[];
}

export function effectiveSourceRootPaths(input: EffectiveSourceRootPathsInput): string[] {
  const configuredRoots = input.configuredRoots.flatMap((root) => {
    if (path.isAbsolute(root)) {
      return [root];
    }
    if (input.workspaceFolders.length === 0) {
      return [root];
    }
    return input.workspaceFolders.map((folder) => path.resolve(folder, root));
  });
  const normalized = [...configuredRoots, ...input.indexedRoots]
    .map(normalizeSourceRootPath)
    .filter((root): root is string => Boolean(root));
  return [...new Set(normalized)];
}

export function normalizeSourceRootPath(root: string): string | undefined {
  let candidate = root;
  if (candidate.startsWith("file://")) {
    try {
      candidate = fileURLToPath(candidate);
    } catch {
      return undefined;
    }
  }
  const unresolved = path.resolve(candidate);
  let resolved = unresolved;
  try {
    resolved = realpathSync.native(unresolved);
  } catch {
    // Keep the lexical path for configured roots that do not exist yet.
  }
  const trimmed = resolved.replace(/[\\/]+$/, "");
  return trimmed || resolved;
}

export function sourceRootContainsDocument(
  sourceRoots: readonly string[],
  documentPath: string,
): boolean {
  const normalizedDocumentPath = normalizeSourceRootPath(documentPath);
  if (!normalizedDocumentPath) {
    return false;
  }
  return sourceRoots.some((root) => {
    const normalizedRoot = normalizeSourceRootPath(root);
    return Boolean(
      normalizedRoot
      && (normalizedDocumentPath === normalizedRoot
        || normalizedDocumentPath.startsWith(`${normalizedRoot}${path.sep}`)),
    );
  });
}

export function workspaceAliasedSourcePath(
  sourcePath: string,
  workspaceFolders: readonly string[],
): string | undefined {
  const resolvedSource = path.resolve(sourcePath);

  // Preserve an existing workspace URI identity before resolving symlinks. A
  // workspace-local symlink may intentionally point at an external source tree.
  if (workspaceFolders.some((folder) => isPathInsideOrEqual(resolvedSource, path.resolve(folder)))) {
    return resolvedSource;
  }

  const canonicalSource = canonicalExistingPath(resolvedSource);
  for (const folder of workspaceFolders) {
    const resolvedFolder = path.resolve(folder);
    const canonicalFolder = canonicalExistingPath(resolvedFolder);
    if (!isPathInsideOrEqual(canonicalSource, canonicalFolder)) {
      continue;
    }
    const relative = path.relative(canonicalFolder, canonicalSource);
    return path.join(resolvedFolder, relative);
  }
  return undefined;
}

function isPathInsideOrEqual(targetPath: string, folder: string): boolean {
  const relative = path.relative(folder, targetPath);
  return relative === "" || (!relative.startsWith("..") && !path.isAbsolute(relative));
}

function canonicalExistingPath(candidate: string): string {
  const resolved = path.resolve(candidate);
  try {
    return realpathSync.native(resolved);
  } catch {
    return resolved;
  }
}
