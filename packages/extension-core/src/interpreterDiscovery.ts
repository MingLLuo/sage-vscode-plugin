import { existsSync, statSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import * as vscode from "vscode";

export type InterpreterSelectionTarget =
  | "runtime"
  | "languageServer"
  | "runtimeCustom"
  | "languageServerCustom"
  | "languageServerAuto";

export interface InterpreterCandidate extends vscode.QuickPickItem {
  interpreterPath?: string;
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

interface RawCandidate {
  path: string;
  source: string;
  selectionTarget: "runtime" | "languageServer";
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

const COMMON_WORKSPACE_PYTHON_DIRS = [
  ".venv",
  "venv",
  "env",
];

export function discoverInterpreterCandidates(input: DiscoveryInput): InterpreterCandidate[] {
  const environment = input.environment ?? process.env;
  const homeDir = input.homeDir ?? os.homedir();
  const candidates: RawCandidate[] = [];

  if (input.currentPath) {
    candidates.push({
      path: input.currentPath,
      source: "Configured for Sage execution",
      selectionTarget: "runtime",
    });
  }

  if (input.languageServerPythonPath && input.languageServerPythonPath !== "auto") {
    candidates.push({
      path: input.languageServerPythonPath,
      source: "Configured for the language server",
      selectionTarget: "languageServer",
    });
  }

  const pathCommands: Array<[string, "runtime" | "languageServer"]> = [
    ["sage", "runtime"],
    ["python3", "languageServer"],
    ["python", "languageServer"],
  ];
  for (const [command, selectionTarget] of pathCommands) {
    const resolved = findExecutableOnPath(command, input.envPath);
    if (resolved) {
      candidates.push({ path: resolved, source: "Detected on PATH", selectionTarget });
    }
  }

  for (const candidatePath of COMMON_SAGE_PATHS) {
    if (pathExists(candidatePath)) {
      candidates.push({
        path: candidatePath,
        source: "Detected in common system locations",
        selectionTarget: "runtime",
      });
    }
  }

  for (const candidatePath of COMMON_PYTHON_PATHS) {
    if (pathExists(candidatePath)) {
      candidates.push({
        path: candidatePath,
        source: "Detected in common system locations",
        selectionTarget: "languageServer",
      });
    }
  }

  for (const candidatePath of discoverManagedEnvironmentCandidates(environment, homeDir)) {
    if (pathExists(candidatePath)) {
      candidates.push({
        path: candidatePath,
        source: "Detected from local Python environments",
        selectionTarget: "languageServer",
      });
    }
  }

  for (const workspaceFolder of input.workspaceFolders) {
    for (const candidatePath of discoverWorkspaceRuntimeCandidates(workspaceFolder)) {
      if (pathExists(candidatePath)) {
        candidates.push({
          path: candidatePath,
          source: "Detected near the workspace",
          selectionTarget: "runtime",
        });
      }
    }

    for (const candidatePath of discoverWorkspacePythonCandidates(workspaceFolder)) {
      if (pathExists(candidatePath)) {
        candidates.push({
          path: candidatePath,
          source: "Detected from workspace virtual environments",
          selectionTarget: "languageServer",
        });
      }
    }
  }

  const items = dedupeCandidates(candidates).map((candidate) =>
    toQuickPickItem(candidate, input.currentPath, input.languageServerPythonPath),
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

function dedupeCandidates(candidates: RawCandidate[]): RawCandidate[] {
  const seen = new Set<string>();
  const unique: RawCandidate[] = [];
  for (const candidate of candidates) {
    const key = `${candidate.selectionTarget}:${normalizeCandidateKey(candidate.path)}`;
    if (seen.has(key)) {
      continue;
    }
    seen.add(key);
    unique.push(candidate);
  }
  return unique;
}

function normalizeCandidateKey(candidatePath: string): string {
  return process.platform === "win32" ? candidatePath.toLowerCase() : candidatePath;
}

function toQuickPickItem(
  candidate: RawCandidate,
  currentRuntimePath: string,
  currentLanguageServerPythonPath: string,
): InterpreterCandidate {
  const isCurrent =
    candidate.selectionTarget === "runtime"
      ? candidate.path === currentRuntimePath
      : candidate.path === currentLanguageServerPythonPath;
  const label = buildLabel(candidate, isCurrent);
  const settingName =
    candidate.selectionTarget === "runtime"
      ? "sage.interpreter.path"
      : "sage.languageServer.pythonPath";
  return {
    label,
    description: candidate.path,
    detail: `${candidate.source}. Updates ${settingName}.`,
    interpreterPath: candidate.path,
    selectionTarget: candidate.selectionTarget,
  };
}

function buildLabel(candidate: RawCandidate, isCurrent: boolean): string {
  if (candidate.selectionTarget === "runtime") {
    if (classifyInterpreter(candidate.path) === "sage") {
      return isCurrent ? "Current Sage runtime" : "Detected Sage runtime";
    }
    return isCurrent ? "Current execution runtime" : "Detected execution runtime";
  }

  return isCurrent ? "Current language-server Python" : "Detected language-server Python";
}

function classifyInterpreter(candidatePath: string): "sage" | "python" | "interpreter" {
  const base = path.basename(candidatePath).toLowerCase();
  if (base.startsWith("sage")) {
    return "sage";
  }
  if (base.startsWith("python")) {
    return "python";
  }
  return "interpreter";
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

function discoverWorkspacePythonCandidates(workspaceFolder: string): string[] {
  return COMMON_WORKSPACE_PYTHON_DIRS.map((directory) =>
    process.platform === "win32"
      ? path.join(workspaceFolder, directory, "Scripts", "python.exe")
      : path.join(workspaceFolder, directory, "bin", "python"),
  );
}

function discoverManagedEnvironmentCandidates(
  environment: NodeJS.ProcessEnv,
  homeDir: string,
): string[] {
  const candidates: string[] = [];

  const environmentVariables = [
    environment.SAGE_LSP_PYTHON,
    resolvePythonFromPrefix(environment.VIRTUAL_ENV),
    resolvePythonFromPrefix(environment.CONDA_PREFIX),
  ];
  for (const candidate of environmentVariables) {
    if (candidate) {
      candidates.push(candidate);
    }
  }

  for (const directory of COMMON_HOME_PYTHON_DIRS) {
    candidates.push(resolvePythonFromPrefix(path.join(homeDir, directory)));
  }

  const pyenvShim =
    process.platform === "win32"
      ? path.join(homeDir, ".pyenv", "pyenv-win", "shims", "python.exe")
      : path.join(homeDir, ".pyenv", "shims", "python3");
  candidates.push(pyenvShim);

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
