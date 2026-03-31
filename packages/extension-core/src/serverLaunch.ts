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
}

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

  const fallbackPython = resolveDefaultLanguageServerPython(environment, platform);
  return {
    command: fallbackPython,
    args: [...input.languageServerPythonArgs, "-m", "sage_lsp"],
  };
}

export function resolveDefaultLanguageServerPython(
  environment: NodeJS.ProcessEnv = process.env,
  platform: NodeJS.Platform = process.platform,
): string {
  const override = environment.SAGE_LSP_PYTHON;
  if (override) {
    return override;
  }

  const pythonFromPrefix = resolvePythonFromPrefix(environment.VIRTUAL_ENV, platform)
    ?? resolvePythonFromPrefix(environment.CONDA_PREFIX, platform);
  if (pythonFromPrefix) {
    return pythonFromPrefix;
  }

  return "python";
}

function looksLikePython(command: string): boolean {
  return path.basename(command).toLowerCase().startsWith("python");
}

function resolvePythonFromPrefix(
  prefix: string | undefined,
  platform: NodeJS.Platform,
): string | undefined {
  if (!prefix) {
    return undefined;
  }

  if (platform === "win32") {
    return path.join(prefix, "python.exe");
  }

  return path.join(prefix, "bin", "python");
}
