import { execFile, execFileSync } from "node:child_process";
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
  runtimeProbe?: ((interpreterPath: string, interpreterArgs: readonly string[]) => string[]) | false;
}

export interface AsyncSourceRootDiscoveryOptions {
  exists?: (candidate: string) => boolean;
  listDir?: (candidate: string) => string[];
  interpreterPath?: string;
  interpreterArgs?: readonly string[];
  runtimeProbe?: ((interpreterPath: string, interpreterArgs: readonly string[]) => Promise<string[]>) | false;
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

export function resolveRuntimePythonPaths(
  workspaceFolders: readonly string[],
  sourceRoots: readonly string[],
  extraPaths: readonly string[],
  activeFilePath?: string,
): string[] {
  const configuredPaths = resolveConfiguredPaths(workspaceFolders, [...sourceRoots, ...extraPaths]);
  const activeDirectory = activeFilePath ? path.resolve(path.dirname(activeFilePath)) : undefined;
  return dedupe([...(activeDirectory ? [activeDirectory] : []), ...configuredPaths]);
}

export function discoverSourceRoots(
  workspaceFolders: readonly string[],
  configuredSourceRoots: readonly string[],
  options: SourceRootDiscoveryOptions = {},
): string[] {
  const exists = options.exists ?? fs.existsSync;
  const interpreterRoots = discoverInterpreterSourceRoots(
    options.interpreterPath ?? "",
    options.interpreterArgs ?? [],
    {
      exists,
      listDir: options.listDir,
      runtimeProbe: options.runtimeProbe,
    },
  );

  return mergeDiscoveredSourceRoots(workspaceFolders, configuredSourceRoots, interpreterRoots, exists);
}

export async function discoverSourceRootsAsync(
  workspaceFolders: readonly string[],
  configuredSourceRoots: readonly string[],
  options: AsyncSourceRootDiscoveryOptions = {},
): Promise<string[]> {
  const exists = options.exists ?? fs.existsSync;
  const interpreterRoots = await discoverInterpreterSourceRootsAsync(
    options.interpreterPath ?? "",
    options.interpreterArgs ?? [],
    {
      exists,
      listDir: options.listDir,
      runtimeProbe: options.runtimeProbe,
    },
  );

  return mergeDiscoveredSourceRoots(workspaceFolders, configuredSourceRoots, interpreterRoots, exists);
}

function mergeDiscoveredSourceRoots(
  workspaceFolders: readonly string[],
  configuredSourceRoots: readonly string[],
  interpreterRoots: readonly string[],
  exists: (candidate: string) => boolean,
): string[] {
  const configured = resolveConfiguredPaths(workspaceFolders, configuredSourceRoots);
  const workspaceDiscovered = workspaceFolders.flatMap((folder) => {
    const sageSrcRoot = path.join(folder, "src", "sage");
    if (exists(sageSrcRoot)) {
      return [path.join(folder, "src")];
    }
    return [folder];
  });
  const nearbySageRoots = discoverNearbySageSourceRoots(workspaceFolders, exists);
  const projectRoots = configured.length > 0 ? configured : workspaceDiscovered;
  const hasPreferredSageSourceRoot = [...projectRoots, ...nearbySageRoots].some((candidate) =>
    !isPythonPackageRoot(candidate) && exists(path.join(candidate, "sage"))
  );
  const effectiveInterpreterRoots = hasPreferredSageSourceRoot
    ? interpreterRoots.filter((candidate) => !isPythonPackageRoot(candidate))
    : interpreterRoots;
  return dedupe(
    [...projectRoots, ...nearbySageRoots, ...effectiveInterpreterRoots].map((candidate) => path.resolve(candidate)),
  );
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
    options.runtimeProbe === false ? () => [] : (options.runtimeProbe ?? probeRuntimeSourceRoots)
  )(interpreterPath, interpreterArgs).filter((candidate) => exists(path.join(candidate, "sage")));

  return dedupe([...heuristicRoots, ...runtimeRoots].map((candidate) => path.resolve(candidate)));
}

export async function discoverInterpreterSourceRootsAsync(
  interpreterPath: string,
  interpreterArgs: readonly string[],
  options: Pick<AsyncSourceRootDiscoveryOptions, "exists" | "listDir" | "runtimeProbe"> = {},
): Promise<string[]> {
  const exists = options.exists ?? fs.existsSync;
  const listDir = options.listDir ?? defaultListDir;

  if (!interpreterPath) {
    return [];
  }

  const heuristicRoots = discoverHeuristicInterpreterRoots(interpreterPath, exists, listDir);
  const runtimeProbe = options.runtimeProbe === false ? async () => [] : (options.runtimeProbe ?? probeRuntimeSourceRootsAsync);
  const runtimeRoots = (await runtimeProbe(interpreterPath, interpreterArgs))
    .filter((candidate) => exists(path.join(candidate, "sage")));

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

export function discoverNearbySageSourceRoots(
  workspaceFolders: readonly string[],
  exists: (candidate: string) => boolean = fs.existsSync,
): string[] {
  const roots: string[] = [];
  for (const folder of workspaceFolders) {
    let current = path.resolve(folder);
    for (let depth = 0; depth < 8; depth += 1) {
      const directSrc = path.join(current, "src");
      if (exists(path.join(directSrc, "sage"))) {
        roots.push(directSrc);
      }

      const siblingSageSrc = path.join(current, "sage", "src");
      if (exists(path.join(siblingSageSrc, "sage"))) {
        roots.push(siblingSageSrc);
      }

      const parent = path.dirname(current);
      if (parent === current) {
        break;
      }
      current = parent;
    }
  }
  return dedupe(roots.map((candidate) => path.resolve(candidate)));
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

function isPythonPackageRoot(candidate: string): boolean {
  const basename = path.basename(path.resolve(candidate));
  return basename === "site-packages" || basename === "dist-packages";
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

function probeRuntimeSourceRootsAsync(interpreterPath: string, interpreterArgs: readonly string[]): Promise<string[]> {
  const invocation = buildRuntimeProbeInvocation(interpreterPath, interpreterArgs);
  if (!invocation) {
    return Promise.resolve([]);
  }

  return new Promise((resolve) => {
    execFile(
      invocation.command,
      invocation.args,
      {
        encoding: "utf-8",
        timeout: 2000,
        maxBuffer: 256 * 1024,
      },
      (error, stdout) => {
        if (error) {
          resolve([]);
          return;
        }
        resolve(parseRuntimeProbeOutput(stdout));
      },
    );
  });
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
