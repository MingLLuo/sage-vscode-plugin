import { existsSync, statSync } from "node:fs";
import path from "node:path";
import * as vscode from "vscode";

export type InterpreterSelectionTarget = "environment" | "runtimeCustom";

export interface InterpreterConfigurationUpdate {
  section: "interpreter.path";
  value: string;
}

export interface InterpreterSelectionSettings {
  interpreterPath: string;
}

export interface InterpreterSelectionPrompts {
  runtimePath(initialValue: string): PromiseLike<string | undefined>;
}

export interface InterpreterCandidate extends vscode.QuickPickItem {
  interpreterPath?: string;
  selectionTarget: InterpreterSelectionTarget;
}

interface DiscoveryInput {
  currentPath: string;
  workspaceFolders: readonly string[];
  envPath?: string;
}

interface EnvironmentProfile {
  runtimePath: string;
  kind: "current" | "localDev" | "system";
  source: string;
}

const COMMON_SAGE_PATHS = [
  "/opt/homebrew/bin/sage",
  "/usr/local/bin/sage",
  "/usr/bin/sage",
  "/Applications/SageMath.app/Contents/MacOS/sage",
];

export function discoverInterpreterCandidates(input: DiscoveryInput): InterpreterCandidate[] {
  const profiles: EnvironmentProfile[] = [];

  if (input.currentPath) {
    profiles.push({
      runtimePath: input.currentPath,
      kind: isLocalSageCheckout(input.currentPath) ? "localDev" : "current",
      source: "Current workspace configuration",
    });
  }

  const localDevelopmentRuntimes = dedupe(
    input.workspaceFolders
      .flatMap((folder) => discoverWorkspaceRuntimeCandidates(folder))
      .filter((candidate) => isLocalSageCheckout(candidate)),
  );
  for (const runtimePath of localDevelopmentRuntimes) {
    profiles.push({
      runtimePath,
      kind: "localDev",
      source: "Detected local Sage checkout",
    });
  }

  const systemSageRuntimes = dedupe(
    [
      findExecutableOnPath("sage", input.envPath),
      ...COMMON_SAGE_PATHS,
    ].filter(
      (candidate): candidate is string =>
        typeof candidate === "string" && candidate.length > 0 && pathExists(candidate),
    ),
  ).filter((candidate) => !isLocalSageCheckout(candidate));

  for (const runtimePath of systemSageRuntimes) {
    profiles.push({
      runtimePath,
      kind: "system",
      source: "Detected stable Sage runtime",
    });
  }

  const items = dedupeProfiles(profiles).map((profile) => toQuickPickItem(profile, input.currentPath));
  items.push({
    label: "Enter custom Sage path...",
    detail: "Update sage.interpreter.path manually.",
    selectionTarget: "runtimeCustom",
    alwaysShow: true,
  });
  return items;
}

export async function resolveInterpreterConfigurationUpdates(
  picked: InterpreterCandidate,
  settings: InterpreterSelectionSettings,
  prompts: InterpreterSelectionPrompts,
): Promise<InterpreterConfigurationUpdate[] | undefined> {
  if (picked.selectionTarget === "runtimeCustom") {
    const selection = (await prompts.runtimePath(settings.interpreterPath))?.trim();
    if (!selection) {
      return undefined;
    }
    return [{ section: "interpreter.path", value: selection }];
  }

  if (!picked.interpreterPath) {
    return undefined;
  }

  const interpreterPath = picked.interpreterPath.trim();
  return interpreterPath && interpreterPath !== settings.interpreterPath
    ? [{ section: "interpreter.path", value: interpreterPath }]
    : [];
}

function dedupeProfiles(profiles: EnvironmentProfile[]): EnvironmentProfile[] {
  const seen = new Set<string>();
  const unique: EnvironmentProfile[] = [];
  for (const profile of profiles) {
    if (!profile.runtimePath) {
      continue;
    }
    const key = normalizeCandidateKey(profile.runtimePath);
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

function toQuickPickItem(profile: EnvironmentProfile, currentRuntimePath: string): InterpreterCandidate {
  return {
    label: buildLabel(profile, profile.runtimePath === currentRuntimePath),
    description: profile.runtimePath,
    detail: profile.source,
    interpreterPath: profile.runtimePath,
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
