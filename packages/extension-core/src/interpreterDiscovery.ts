import { existsSync, statSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import * as vscode from "vscode";

export type InterpreterSelectionTarget =
  | "environment"
  | "runtimeCustom"
  | "languageServerCustom"
  | "languageServerAuto";

export interface InterpreterConfigurationUpdate {
  section: "interpreter.path" | "languageServer.pythonPath";
  value: string;
}

export interface InterpreterCandidate extends vscode.QuickPickItem {
  interpreterPath?: string;
  languageServerPythonPath?: string;
  updates?: InterpreterConfigurationUpdate[];
  selectionTarget: InterpreterSelectionTarget;
}

interface DiscoveryInput {
  currentPath: string;
  languageServerPythonPath: string;
  workspaceFolders: readonly string[];
  envPath?: string;
  environment?: NodeJS.ProcessEnv;
  homeDir?: string;
}

interface EnvironmentProfile {
  runtimePath: string;
  languageServerPythonPath: string;
  kind: "current" | "localDev" | "system";
  source: string;
}

const COMMON_SAGE_PATHS = [
  "/opt/homebrew/bin/sage",
  "/usr/local/bin/sage",
  "/usr/bin/sage",
  "/Applications/SageMath.app/Contents/MacOS/sage",
];

const COMMON_PYTHON_PATHS = [
  "/opt/homebrew/bin/python3",
  "/usr/local/bin/python3",
  "/usr/bin/python3",
];

const COMMON_HOME_PYTHON_DIRS = [
  "miniforge3",
  "mambaforge",
  "miniconda3",
  "anaconda3",
];

const PREFERRED_DEV_ENV_NAMES = ["sage-dev"];

export function discoverInterpreterCandidates(input: DiscoveryInput): InterpreterCandidate[] {
  const environment = input.environment ?? process.env;
  const homeDir = input.homeDir ?? os.homedir();
  const profiles: EnvironmentProfile[] = [];

  const preferredLanguageServerPython =
    input.languageServerPythonPath && input.languageServerPythonPath !== "auto"
      ? input.languageServerPythonPath
      : resolvePreferredLanguageServerPython(environment, input.envPath, homeDir);
  const localDevelopmentPython = discoverLocalDevelopmentPython(environment, homeDir);

  if (input.currentPath) {
    const currentUsesLocalDevelopmentPython =
      input.languageServerPythonPath === "auto"
      && isLocalSageCheckout(input.currentPath)
      && Boolean(localDevelopmentPython);
    profiles.push({
      runtimePath: input.currentPath,
      languageServerPythonPath:
        currentUsesLocalDevelopmentPython
          ? localDevelopmentPython!
          : input.languageServerPythonPath === "auto"
            ? preferredLanguageServerPython
          : input.languageServerPythonPath,
      kind: currentUsesLocalDevelopmentPython ? "localDev" : "current",
      source: "Current workspace configuration",
    });
  }

  const localDevelopmentRuntimes = dedupe(
    [
      ...input.workspaceFolders.flatMap((folder) => discoverWorkspaceRuntimeCandidates(folder)),
      input.currentPath,
    ].filter((candidate) => isLocalSageCheckout(candidate)),
  );
  for (const runtimePath of localDevelopmentRuntimes) {
    if (!localDevelopmentPython) {
      continue;
    }
    profiles.push({
      runtimePath,
      languageServerPythonPath: localDevelopmentPython,
      kind: "localDev",
      source: "Detected local Sage checkout with conda env sage-dev",
    });
  }

  const systemSageRuntimes = dedupe(
    [
      findExecutableOnPath("sage", input.envPath),
      ...COMMON_SAGE_PATHS,
      input.currentPath && !isLocalSageCheckout(input.currentPath) ? input.currentPath : "",
    ].filter(
      (candidate): candidate is string =>
        typeof candidate === "string" && candidate.length > 0 && pathExists(candidate),
    ),
  ).filter((candidate) => !isLocalSageCheckout(candidate));

  for (const runtimePath of systemSageRuntimes) {
    profiles.push({
      runtimePath,
      languageServerPythonPath: preferredLanguageServerPython,
      kind: "system",
      source: "Detected stable Sage runtime",
    });
  }

  const items = dedupeProfiles(profiles).map((profile) =>
    toQuickPickItem(profile, input.currentPath, input.languageServerPythonPath, preferredLanguageServerPython),
  );
  items.push(
    {
      label: "Enter custom Sage path...",
      detail: "Update sage.interpreter.path manually.",
      selectionTarget: "runtimeCustom",
      alwaysShow: true,
    },
    {
      label: "Enter custom language-server Python path...",
      detail: "Update sage.languageServer.pythonPath manually.",
      selectionTarget: "languageServerCustom",
      alwaysShow: true,
    },
    {
      label: "Use auto-detected language-server Python",
      detail: "Reset sage.languageServer.pythonPath to auto.",
      selectionTarget: "languageServerAuto",
      alwaysShow: true,
    },
  );
  return items;
}

function dedupeProfiles(profiles: EnvironmentProfile[]): EnvironmentProfile[] {
  const seen = new Set<string>();
  const unique: EnvironmentProfile[] = [];
  for (const profile of profiles) {
    if (!profile.runtimePath || !profile.languageServerPythonPath) {
      continue;
    }
    const key = `${normalizeCandidateKey(profile.runtimePath)}::${normalizeCandidateKey(profile.languageServerPythonPath)}`;
    if (seen.has(key)) {
      continue;
    }
    seen.add(key);
    unique.push(profile);
  }
  return unique;
}

function normalizeCandidateKey(candidatePath: string): string {
  return process.platform === "win32" ? candidatePath.toLowerCase() : candidatePath;
}

function toQuickPickItem(
  profile: EnvironmentProfile,
  currentRuntimePath: string,
  currentLanguageServerPythonPath: string,
  preferredLanguageServerPython: string,
): InterpreterCandidate {
  const effectiveCurrentLanguageServerPython =
    currentLanguageServerPythonPath === "auto"
      ? preferredLanguageServerPython
      : currentLanguageServerPythonPath;
  const isCurrent =
    profile.runtimePath === currentRuntimePath
    && profile.languageServerPythonPath === effectiveCurrentLanguageServerPython;

  return {
    label: buildLabel(profile, isCurrent),
    description: profile.runtimePath,
    detail: `${profile.source}. Language server Python: ${profile.languageServerPythonPath}.`,
    interpreterPath: profile.runtimePath,
    languageServerPythonPath: profile.languageServerPythonPath,
    updates: [
      { section: "interpreter.path", value: profile.runtimePath },
      { section: "languageServer.pythonPath", value: profile.languageServerPythonPath },
    ],
    selectionTarget: "environment",
  };
}

function buildLabel(profile: EnvironmentProfile, isCurrent: boolean): string {
  if (profile.kind === "localDev") {
    return isCurrent ? "Current local Sage development environment" : "Local Sage development environment";
  }
  if (profile.kind === "system") {
    return isCurrent ? "Current system Sage environment" : "System Sage (stable)";
  }
  return isCurrent ? "Current Sage environment" : "Detected Sage environment";
}

function discoverWorkspaceRuntimeCandidates(workspaceFolder: string): string[] {
  const folderName = path.basename(workspaceFolder);
  const parent = path.dirname(workspaceFolder);
  return [
    path.join(workspaceFolder, "sage"),
    path.join(parent, "sage", "sage"),
    path.join(parent, `${folderName}-sage`, "sage"),
  ];
}

function discoverLocalDevelopmentPython(
  environment: NodeJS.ProcessEnv,
  homeDir: string,
): string | undefined {
  const activeCondaPrefix = environment.CONDA_PREFIX;
  if (activeCondaPrefix && PREFERRED_DEV_ENV_NAMES.includes(path.basename(activeCondaPrefix))) {
    const candidate = resolvePythonFromPrefix(activeCondaPrefix);
    if (pathExists(candidate)) {
      return candidate;
    }
  }

  for (const root of COMMON_HOME_PYTHON_DIRS) {
    for (const envName of PREFERRED_DEV_ENV_NAMES) {
      const candidate = resolvePythonFromPrefix(path.join(homeDir, root, "envs", envName));
      if (pathExists(candidate)) {
        return candidate;
      }
    }
  }

  return undefined;
}

function resolvePreferredLanguageServerPython(
  environment: NodeJS.ProcessEnv,
  envPath: string | undefined,
  homeDir: string,
): string {
  const environmentVariables = [
    environment.SAGE_LSP_PYTHON,
    resolvePythonFromPrefix(environment.VIRTUAL_ENV),
    resolvePythonFromPrefix(environment.CONDA_PREFIX),
    findExecutableOnPath("python3", envPath),
    findExecutableOnPath("python", envPath),
    ...COMMON_PYTHON_PATHS,
    ...discoverManagedEnvironmentCandidates(environment, homeDir),
    discoverLocalDevelopmentPython(environment, homeDir),
  ];

  for (const candidate of environmentVariables) {
    if (candidate && pathExists(candidate)) {
      return candidate;
    }
  }

  return "python";
}

function discoverManagedEnvironmentCandidates(
  environment: NodeJS.ProcessEnv,
  homeDir: string,
): string[] {
  const candidates: string[] = [];

  for (const directory of COMMON_HOME_PYTHON_DIRS) {
    candidates.push(resolvePythonFromPrefix(path.join(homeDir, directory)));
  }

  const pyenvShim =
    process.platform === "win32"
      ? path.join(homeDir, ".pyenv", "pyenv-win", "shims", "python.exe")
      : path.join(homeDir, ".pyenv", "shims", "python3");
  candidates.push(pyenvShim);

  const activeCondaPrefix = environment.CONDA_PREFIX;
  if (activeCondaPrefix) {
    candidates.push(resolvePythonFromPrefix(activeCondaPrefix));
  }

  return candidates.filter(Boolean);
}

function resolvePythonFromPrefix(prefix: string | undefined): string {
  if (!prefix) {
    return "";
  }

  if (process.platform === "win32") {
    return path.join(prefix, "python.exe");
  }

  return path.join(prefix, "bin", "python");
}

function isLocalSageCheckout(candidatePath: string): boolean {
  if (!candidatePath || !pathExists(candidatePath)) {
    return false;
  }

  const resolvedPath = path.resolve(candidatePath);
  const runtimeRoot = path.dirname(resolvedPath);
  return entryExists(path.join(runtimeRoot, "src", "bin", "sage"))
    || entryExists(path.join(runtimeRoot, "src", "sage"));
}

function findExecutableOnPath(command: string, envPath = process.env.PATH): string | undefined {
  if (!envPath) {
    return undefined;
  }

  const extensions =
    process.platform === "win32"
      ? (process.env.PATHEXT?.split(";").filter(Boolean) ?? [".exe", ".cmd", ".bat"])
      : [""];

  for (const entry of envPath.split(path.delimiter).filter(Boolean)) {
    for (const extension of extensions) {
      const candidatePath = path.join(entry, `${command}${extension}`);
      if (pathExists(candidatePath)) {
        return candidatePath;
      }
    }
  }

  return undefined;
}

function dedupe(values: string[]): string[] {
  return [...new Set(values.filter(Boolean))];
}

function pathExists(candidatePath: string): boolean {
  if (!candidatePath) {
    return false;
  }
  try {
    return existsSync(candidatePath) && statSync(candidatePath).isFile();
  } catch {
    return false;
  }
}

function entryExists(candidatePath: string): boolean {
  if (!candidatePath) {
    return false;
  }
  try {
    return existsSync(candidatePath);
  } catch {
    return false;
  }
}
