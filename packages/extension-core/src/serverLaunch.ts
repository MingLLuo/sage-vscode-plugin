import fs from "node:fs";
import os from "node:os";
import path from "node:path";

export interface LanguageServerLaunch {
  command: string;
  args: string[];
}

export interface LanguageServerLaunchInput {
  interpreterPath: string;
  interpreterArgs: readonly string[];
  languageServerPythonPath: string;
  languageServerPythonArgs: readonly string[];
  environment?: NodeJS.ProcessEnv;
  platform?: NodeJS.Platform;
  homeDir?: string;
  exists?: (candidate: string) => boolean;
}

const COMMON_HOME_PYTHON_DIRS = [
  "miniforge3",
  "mambaforge",
  "miniconda3",
  "anaconda3",
];

export function buildLanguageServerLaunch(
  input: LanguageServerLaunchInput,
): LanguageServerLaunch {
  const environment = input.environment ?? process.env;
  const platform = input.platform ?? process.platform;
  const configuredRuntime = input.languageServerPythonPath.trim();

  if (configuredRuntime && configuredRuntime !== "auto") {
    return {
      command: configuredRuntime,
      args: [...input.languageServerPythonArgs, "-m", "sage_lsp"],
    };
  }

  if (looksLikePython(input.interpreterPath)) {
    return {
      command: input.interpreterPath,
      args: [...input.interpreterArgs, ...input.languageServerPythonArgs, "-m", "sage_lsp"],
    };
  }

  const fallbackPython = resolveDefaultLanguageServerPython(environment, platform, {
    interpreterPath: input.interpreterPath,
    homeDir: input.homeDir,
    exists: input.exists,
  });
  return {
    command: fallbackPython,
    args: [...input.languageServerPythonArgs, "-m", "sage_lsp"],
  };
}

export function resolveDefaultLanguageServerPython(
  environment: NodeJS.ProcessEnv = process.env,
  platform: NodeJS.Platform = process.platform,
  options: {
    interpreterPath?: string;
    homeDir?: string;
    exists?: (candidate: string) => boolean;
  } = {},
): string {
  const exists = options.exists ?? fs.existsSync;
  const homeDir = options.homeDir ?? os.homedir();

  const override = environment.SAGE_LSP_PYTHON;
  if (override) {
    return override;
  }

  if (looksLikeLocalSageCheckout(options.interpreterPath ?? "", exists)) {
    const localDevelopmentPython = resolveLocalDevelopmentPython(environment, platform, homeDir, exists);
    if (localDevelopmentPython) {
      return localDevelopmentPython;
    }
  }

  const pythonFromPrefix = resolvePythonFromPrefix(environment.VIRTUAL_ENV, platform, exists)
    ?? resolvePythonFromPrefix(environment.CONDA_PREFIX, platform, exists);
  if (pythonFromPrefix) {
    return pythonFromPrefix;
  }

  const localDevelopmentPython = resolveLocalDevelopmentPython(environment, platform, homeDir, exists);
  if (localDevelopmentPython) {
    return localDevelopmentPython;
  }

  return "python";
}

function looksLikePython(command: string): boolean {
  return path.basename(command).toLowerCase().startsWith("python");
}

function looksLikeLocalSageCheckout(
  interpreterPath: string,
  exists: (candidate: string) => boolean,
): boolean {
  if (!interpreterPath) {
    return false;
  }

  const runtimeRoot = path.dirname(path.resolve(interpreterPath));
  return exists(path.join(runtimeRoot, "src", "bin", "sage"))
    || exists(path.join(runtimeRoot, "src", "sage"));
}

function resolveLocalDevelopmentPython(
  environment: NodeJS.ProcessEnv,
  platform: NodeJS.Platform,
  homeDir: string,
  exists: (candidate: string) => boolean,
): string | undefined {
  const activeCondaPrefix = environment.CONDA_PREFIX;
  if (activeCondaPrefix && path.basename(activeCondaPrefix) === "sage-dev") {
    return resolvePythonFromPrefix(activeCondaPrefix, platform, exists);
  }

  for (const directory of COMMON_HOME_PYTHON_DIRS) {
    const candidatePrefix = path.join(homeDir, directory, "envs", "sage-dev");
    const candidate = resolvePythonFromPrefix(candidatePrefix, platform, exists);
    if (candidate) {
      return candidate;
    }
  }

  return undefined;
}

function resolvePythonFromPrefix(
  prefix: string | undefined,
  platform: NodeJS.Platform,
  exists: (candidate: string) => boolean,
): string | undefined {
  if (!prefix) {
    return undefined;
  }

  const candidate = platform === "win32"
    ? path.join(prefix, "python.exe")
    : path.join(prefix, "bin", "python");
  return exists(candidate) ? candidate : undefined;
}
