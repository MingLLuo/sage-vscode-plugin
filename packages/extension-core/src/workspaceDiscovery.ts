import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { pathToFileURL } from "node:url";

export interface WorkspaceInitializationData {
  rootUri: string | null;
  folders: string[];
  sourceRoots: string[];
}

export interface SourceRootDiscoveryOptions {
  exists?: (candidate: string) => boolean;
  listDir?: (candidate: string) => string[];
  interpreterPath?: string;
  interpreterArgs?: readonly string[];
  runtimeProbe?: (interpreterPath: string, interpreterArgs: readonly string[]) => string[];
}

const RUNTIME_SOURCE_ROOT_SCRIPT = [
  "import json",
  "from pathlib import Path",
  "roots = []",
  "try:",
  "    import sage",
  "except Exception:",
  "    sage = None",
  "if sage is not None:",
  "    package_root = Path(sage.__file__).resolve().parents[1]",
  "    if (package_root / 'sage').exists():",
  "        roots.append(str(package_root))",
  "unique = []",
  "seen = set()",
  "for entry in roots:",
  "    if entry not in seen:",
  "        seen.add(entry)",
  "        unique.append(entry)",
  "print(json.dumps(unique))",
].join("\n");

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
  options: SourceRootDiscoveryOptions = {},
): string[] {
  const exists = options.exists ?? fs.existsSync;
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

  const interpreterRoots = discoverInterpreterSourceRoots(
    options.interpreterPath ?? "",
    options.interpreterArgs ?? [],
    {
      exists,
      listDir: options.listDir,
      runtimeProbe: options.runtimeProbe,
    },
  );

  return dedupe([...discovered, ...interpreterRoots].map((candidate) => path.resolve(candidate)));
}

export function buildWorkspaceInitializationData(
  workspaceFolders: readonly string[],
  configuredSourceRoots: readonly string[],
  options: SourceRootDiscoveryOptions = {},
): WorkspaceInitializationData {
  const normalizedFolders = dedupe(workspaceFolders.map((folder) => path.resolve(folder)));
  const sourceRoots = discoverSourceRoots(normalizedFolders, configuredSourceRoots, options);

  return {
    rootUri: normalizedFolders[0] ? pathToFileURL(normalizedFolders[0]).toString() : null,
    folders: normalizedFolders.map((folder) => pathToFileURL(folder).toString()),
    sourceRoots: sourceRoots.map((sourceRoot) => pathToFileURL(sourceRoot).toString()),
  };
}

export function discoverInterpreterSourceRoots(
  interpreterPath: string,
  interpreterArgs: readonly string[],
  options: Pick<SourceRootDiscoveryOptions, "exists" | "listDir" | "runtimeProbe"> = {},
): string[] {
  const exists = options.exists ?? fs.existsSync;
  const listDir = options.listDir ?? defaultListDir;

  if (!interpreterPath) {
    return [];
  }

  const heuristicRoots = discoverHeuristicInterpreterRoots(interpreterPath, exists, listDir);
  const runtimeRoots = (
    options.runtimeProbe ?? probeRuntimeSourceRoots
  )(interpreterPath, interpreterArgs).filter((candidate) => exists(path.join(candidate, "sage")));

  return dedupe([...heuristicRoots, ...runtimeRoots].map((candidate) => path.resolve(candidate)));
}

function discoverHeuristicInterpreterRoots(
  interpreterPath: string,
  exists: (candidate: string) => boolean,
  listDir: (candidate: string) => string[],
): string[] {
  if (!looksLikeFilesystemPath(interpreterPath)) {
    return [];
  }

  const resolvedInterpreter = path.resolve(interpreterPath);
  const prefixCandidates = collectInterpreterPrefixes(resolvedInterpreter);
  const discovered: string[] = [];

  for (const prefix of prefixCandidates) {
    const srcRoot = path.join(prefix, "src");
    if (exists(path.join(srcRoot, "sage"))) {
      discovered.push(srcRoot);
    }

    for (const sitePackagesRoot of discoverSitePackagesRoots(prefix, exists, listDir)) {
      if (exists(path.join(sitePackagesRoot, "sage"))) {
        discovered.push(sitePackagesRoot);
      }
    }
  }

  return dedupe(discovered);
}

function collectInterpreterPrefixes(resolvedInterpreter: string): string[] {
  const prefixes: string[] = [];
  let current = path.dirname(resolvedInterpreter);
  for (let depth = 0; depth < 4; depth += 1) {
    prefixes.push(current);
    const parent = path.dirname(current);
    if (parent === current) {
      break;
    }
    current = parent;
  }
  return dedupe(prefixes);
}

function discoverSitePackagesRoots(
  prefix: string,
  exists: (candidate: string) => boolean,
  listDir: (candidate: string) => string[],
): string[] {
  const libraryBases = [path.join(prefix, "lib"), path.join(prefix, "local", "lib")];
  const roots: string[] = [];

  for (const libraryBase of libraryBases) {
    if (!exists(libraryBase)) {
      continue;
    }
    for (const entry of listDir(libraryBase)) {
      if (!entry.startsWith("python")) {
        continue;
      }
      const sitePackagesRoot = path.join(libraryBase, entry, "site-packages");
      if (exists(sitePackagesRoot)) {
        roots.push(sitePackagesRoot);
      }
    }
  }

  return dedupe(roots);
}

function probeRuntimeSourceRoots(interpreterPath: string, interpreterArgs: readonly string[]): string[] {
  const invocation = buildRuntimeProbeInvocation(interpreterPath, interpreterArgs);
  if (!invocation) {
    return [];
  }

  try {
    const stdout = execFileSync(invocation.command, invocation.args, {
      encoding: "utf-8",
      timeout: 2000,
      maxBuffer: 256 * 1024,
      stdio: ["ignore", "pipe", "ignore"],
    });
    return parseRuntimeProbeOutput(stdout);
  } catch {
    return [];
  }
}

function buildRuntimeProbeInvocation(
  interpreterPath: string,
  interpreterArgs: readonly string[],
): { command: string; args: string[] } | undefined {
  const baseName = path.basename(interpreterPath).toLowerCase();

  if (baseName.startsWith("python")) {
    return {
      command: interpreterPath,
      args: [...interpreterArgs, "-c", RUNTIME_SOURCE_ROOT_SCRIPT],
    };
  }

  if (baseName.startsWith("sage") || interpreterPath === "sage") {
    return {
      command: interpreterPath,
      args: [...interpreterArgs, "-python", "-c", RUNTIME_SOURCE_ROOT_SCRIPT],
    };
  }

  return undefined;
}

function parseRuntimeProbeOutput(stdout: string): string[] {
  const lines = stdout
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter(Boolean);
  const payload = lines.at(-1);
  if (!payload) {
    return [];
  }

  try {
    const parsed = JSON.parse(payload);
    return Array.isArray(parsed) ? parsed.filter((entry): entry is string => typeof entry === "string") : [];
  } catch {
    return [];
  }
}

function looksLikeFilesystemPath(candidate: string): boolean {
  return path.isAbsolute(candidate) || candidate.includes(path.sep) || (path.sep === "\\" && candidate.includes("/"));
}

function defaultListDir(candidate: string): string[] {
  try {
    return fs.readdirSync(candidate);
  } catch {
    return [];
  }
}

function dedupe(values: readonly string[]): string[] {
  return [...new Set(values)];
}
