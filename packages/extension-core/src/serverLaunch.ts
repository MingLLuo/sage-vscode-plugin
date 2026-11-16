import fs from "node:fs";
import path from "node:path";

export interface LanguageServerLaunch {
  command: string;
  args: string[];
}

export interface LanguageServerLaunchInput {
  interpreterPath: string;
  interpreterArgs: readonly string[];
  languageServerRustPath: string;
  extensionPath?: string;
  repositoryRoot?: string;
  environment?: NodeJS.ProcessEnv;
  platform?: NodeJS.Platform;
  arch?: string;
  exists?: (candidate: string) => boolean;
}

export function buildLanguageServerLaunch(
  input: LanguageServerLaunchInput,
): LanguageServerLaunch {
  const environment = input.environment ?? process.env;
  const platform = input.platform ?? process.platform;
  const configuredRust = input.languageServerRustPath.trim();

  const environmentOverride = environment.SAGE_LS_PATH;
  if (environmentOverride) {
    return { command: environmentOverride, args: [] };
  }

  if (configuredRust && configuredRust !== "auto") {
    return { command: configuredRust, args: [] };
  }

  const localRustBinary = resolveLocalRustLanguageServer(input, platform);
  if (localRustBinary) {
    return { command: localRustBinary, args: [] };
  }

  const packagedRustBinary = resolvePackagedRustLanguageServer(input, platform, input.arch ?? process.arch);
  if (packagedRustBinary) {
    return { command: packagedRustBinary, args: [] };
  }

  return { command: platform === "win32" ? "sage-ls.exe" : "sage-ls", args: [] };
}

export function resolveLocalRustLanguageServer(
  input: Pick<LanguageServerLaunchInput, "extensionPath" | "repositoryRoot" | "exists">,
  platform: NodeJS.Platform = process.platform,
): string | undefined {
  const exists = input.exists ?? fs.existsSync;
  const executable = platform === "win32" ? "sage-ls.exe" : "sage-ls";
  const roots = [
    input.repositoryRoot,
    input.extensionPath ? path.resolve(input.extensionPath, "../..") : undefined,
    process.cwd(),
  ].filter((candidate): candidate is string => Boolean(candidate));

  for (const root of roots) {
    for (const profile of ["debug", "release"]) {
      const candidate = path.resolve(root, "target", profile, executable);
      if (exists(candidate)) {
        return candidate;
      }
    }
  }
  return undefined;
}

export function resolvePackagedRustLanguageServer(
  input: Pick<LanguageServerLaunchInput, "extensionPath" | "exists">,
  platform: NodeJS.Platform = process.platform,
  arch: string = process.arch,
): string | undefined {
  if (!input.extensionPath || platform !== "darwin") {
    return undefined;
  }

  const exists = input.exists ?? fs.existsSync;
  const executable = "sage-ls";
  const platformDirectory = `${platform}-${arch}`;
  const candidate = path.resolve(input.extensionPath, "resources", "bin", platformDirectory, executable);
  return exists(candidate) ? candidate : undefined;
}
