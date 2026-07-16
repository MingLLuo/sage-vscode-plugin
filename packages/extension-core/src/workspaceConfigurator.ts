import path from "node:path";

import type { AnalysisMode } from "./settingsModel";

export type WorkspaceConfigurationProfileId =
  | "standard"
  | "python"
  | "native"
  | "research";

export interface WorkspaceConfigurationProfile {
  id: WorkspaceConfigurationProfileId;
  label: string;
  description: string;
  detail: string;
  analysisMode: AnalysisMode;
  enablePythonFiles: boolean;
  enablePyxParsing: boolean;
}

export interface WorkspaceConfigurationUpdate {
  namespace?: string;
  section: string;
  value: boolean | string | string[] | Record<string, unknown>;
}

export interface WorkspaceConfigurationInput {
  workspaceFolders: readonly string[];
  discoveredSourceRoots: readonly string[];
  configuredExtraPaths?: readonly string[];
  configuredPythonExtraPaths?: readonly string[];
  configuredPythonDiagnosticSeverityOverrides?: unknown;
  configuredPythonExclude?: readonly string[];
  configuredPythonIgnore?: readonly string[];
  configuredRuffExclude?: readonly string[];
  configuredRuffConfiguration?: unknown;
  profile: WorkspaceConfigurationProfile;
}

export const WORKSPACE_CONFIGURATION_PROFILES: readonly WorkspaceConfigurationProfile[] = [
  {
    id: "standard",
    label: "Standard Sage workspace",
    description: ".sage files and Sage source indexing",
    detail: "Best for worksheets, scripts, and libraries that primarily use .sage files.",
    analysisMode: "default",
    enablePythonFiles: false,
    enablePyxParsing: true,
  },
  {
    id: "python",
    label: "Sage-heavy Python workspace",
    description: ".py files that import sage.all",
    detail: "Keeps Python language mode, but attaches the Sage LSP for hover, docs, definition, references, and rename.",
    analysisMode: "full",
    enablePythonFiles: true,
    enablePyxParsing: true,
  },
  {
    id: "native",
    label: "Sage native/Cython workspace",
    description: ".pyx, .pxd, .pxi native code",
    detail: "Enables Cython indexing and native Sage navigation while leaving ordinary Python files alone.",
    analysisMode: "full",
    enablePythonFiles: false,
    enablePyxParsing: true,
  },
  {
    id: "research",
    label: "Full Sage research workspace",
    description: ".sage, Sage-heavy .py, and Cython",
    detail: "Best for mixed research projects, notebooks with helper modules, and local Sage source checkouts.",
    analysisMode: "full",
    enablePythonFiles: true,
    enablePyxParsing: true,
  },
];

const DEFAULT_INDEX_EXCLUDES = [
  "**/.git/**",
  "**/__pycache__/**",
  "**/.venv/**",
  "**/.ruff_cache/**",
  "**/.quarto/**",
  "**/.quarto-cache/**",
  "**/.quarto-deno/**",
  "**/.quarto-home/**",
  "**/build/**",
  "**/tmp/**",
];

export function recommendedWorkspaceProfile(activeLanguageId: string | undefined): WorkspaceConfigurationProfile {
  if (activeLanguageId === "python") {
    return WORKSPACE_CONFIGURATION_PROFILES.find((profile) => profile.id === "python")!;
  }
  if (activeLanguageId === "sagemath-cython") {
    return WORKSPACE_CONFIGURATION_PROFILES.find((profile) => profile.id === "native")!;
  }
  return WORKSPACE_CONFIGURATION_PROFILES.find((profile) => profile.id === "standard")!;
}

export function buildWorkspaceConfigurationUpdates(
  input: WorkspaceConfigurationInput,
): WorkspaceConfigurationUpdate[] {
  const sourceRoots = compactWorkspacePaths(input.workspaceFolders, input.discoveredSourceRoots);
  const extraPaths = compactWorkspacePaths(
    input.workspaceFolders,
    [
      ...input.discoveredSourceRoots,
      ...resolveWorkspaceSettingPaths(input.workspaceFolders, input.configuredExtraPaths ?? []),
    ],
  );
  const externalSourceRootPaths = input.discoveredSourceRoots.filter((sourceRoot) => !isInsideAnyWorkspace(input.workspaceFolders, sourceRoot));
  const pythonExtraPaths = compactWorkspacePaths(
    input.workspaceFolders,
    [
      ...input.discoveredSourceRoots,
      ...resolveWorkspaceSettingPaths(input.workspaceFolders, input.configuredExtraPaths ?? []),
      ...resolveWorkspaceSettingPaths(input.workspaceFolders, input.configuredPythonExtraPaths ?? []),
    ].filter((candidate) => !isSamePathAsAny(candidate, externalSourceRootPaths)),
  );
  const externalSourceRoots = externalSourceRootPaths.length > 0
    ? compactWorkspacePaths(input.workspaceFolders, externalSourceRootPaths)
    : [];

  const updates: WorkspaceConfigurationUpdate[] = [
    { section: "languageServer.rustPath", value: "auto" },
    { section: "analysis.mode", value: input.profile.analysisMode },
    { section: "analysis.sourceRoots", value: sourceRoots },
    { section: "analysis.extraPaths", value: extraPaths },
    { namespace: "python", section: "analysis.extraPaths", value: pythonExtraPaths },
    {
      namespace: "python",
      section: "analysis.diagnosticSeverityOverrides",
      value: {
        ...plainObjectOrEmpty(input.configuredPythonDiagnosticSeverityOverrides),
        reportMissingImports: "none",
        reportMissingModuleSource: "none",
      },
    },
    { section: "analysis.enablePythonFiles", value: input.profile.enablePythonFiles },
    { section: "analysis.enablePyxParsing", value: input.profile.enablePyxParsing },
    { section: "analysis.enableDiagnostics", value: true },
    { section: "docs.preferredSource", value: "auto" },
    { section: "docs.showOnHover", value: true },
    { section: "indexing.exclude", value: DEFAULT_INDEX_EXCLUDES },
  ];

  if (externalSourceRoots.length > 0) {
    updates.push(
      {
        namespace: "python",
        section: "analysis.exclude",
        value: mergeStringArrays(input.configuredPythonExclude ?? [], externalSourceRoots),
      },
      {
        namespace: "python",
        section: "analysis.ignore",
        value: mergeStringArrays(input.configuredPythonIgnore ?? [], externalSourceRoots),
      },
      {
        namespace: "ruff",
        section: "exclude",
        value: mergeStringArrays(input.configuredRuffExclude ?? [], externalSourceRoots),
      },
    );
    const ruffConfiguration = mergeRuffExternalSourceExcludes(
      input.configuredRuffConfiguration,
      externalSourceRoots,
    );
    if (ruffConfiguration) {
      updates.push({ namespace: "ruff", section: "configuration", value: ruffConfiguration });
    }
  }

  return updates;
}

function mergeRuffExternalSourceExcludes(
  configuredRuffConfiguration: unknown,
  externalSourceRoots: readonly string[],
): Record<string, unknown> | undefined {
  if (typeof configuredRuffConfiguration === "string") {
    return undefined;
  }
  const base = isPlainObject(configuredRuffConfiguration)
    ? { ...configuredRuffConfiguration }
    : {};
  const existingExclude = Array.isArray(base.exclude)
    ? base.exclude.filter((entry): entry is string => typeof entry === "string")
    : [];
  base.exclude = mergeStringArrays(existingExclude, externalSourceRoots);
  base["force-exclude"] = true;
  return base;
}

function mergeStringArrays(
  existing: readonly string[],
  additions: readonly string[],
): string[] {
  const result: string[] = [];
  const seen = new Set<string>();
  for (const value of [...existing, ...additions]) {
    if (seen.has(value)) {
      continue;
    }
    seen.add(value);
    result.push(value);
  }
  return result;
}

function isSamePathAsAny(candidate: string, targets: readonly string[]): boolean {
  const resolvedCandidate = normalizePathForComparison(path.resolve(candidate));
  return targets.some((target) => normalizePathForComparison(path.resolve(target)) === resolvedCandidate);
}

function normalizePathForComparison(value: string): string {
  return process.platform === "win32" ? value.toLowerCase() : value;
}

function isPlainObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function plainObjectOrEmpty(value: unknown): Record<string, unknown> {
  return isPlainObject(value) ? value : {};
}

function compactWorkspacePaths(workspaceFolders: readonly string[], targetPaths: readonly string[]): string[] {
  const folders = workspaceFolders.map((folder) => path.resolve(folder));
  const seen = new Set<string>();
  const results: string[] = [];

  for (const targetPath of targetPaths) {
    const compacted = compactWorkspacePath(folders, targetPath);
    const key = process.platform === "win32" ? compacted.toLowerCase() : compacted;
    if (seen.has(key)) {
      continue;
    }
    seen.add(key);
    results.push(compacted);
  }

  return results.length > 0 ? results : ["."];
}

function compactWorkspacePath(workspaceFolders: readonly string[], targetPath: string): string {
  const resolvedTarget = path.resolve(targetPath);
  const owner = workspaceFolders.find((folder) => isPathInsideOrEqual(resolvedTarget, folder));
  if (!owner) {
    return resolvedTarget;
  }

  const relative = path.relative(owner, resolvedTarget) || ".";
  return normalizeSettingPath(relative);
}

function resolveWorkspaceSettingPaths(workspaceFolders: readonly string[], settingPaths: readonly string[]): string[] {
  const folders = workspaceFolders.map((folder) => path.resolve(folder));
  return settingPaths.flatMap((settingPath) => {
    if (path.isAbsolute(settingPath)) {
      return [path.resolve(settingPath)];
    }
    if (folders.length === 0) {
      return [path.resolve(settingPath)];
    }
    return folders.map((folder) => path.resolve(folder, settingPath));
  });
}

function isPathInsideOrEqual(targetPath: string, folder: string): boolean {
  const relative = path.relative(folder, targetPath);
  return relative === "" || (!relative.startsWith("..") && !path.isAbsolute(relative));
}

function isInsideAnyWorkspace(workspaceFolders: readonly string[], targetPath: string): boolean {
  const resolvedTarget = path.resolve(targetPath);
  return workspaceFolders.some((folder) => isPathInsideOrEqual(resolvedTarget, path.resolve(folder)));
}

function normalizeSettingPath(value: string): string {
  return value.split(path.sep).join("/");
}
