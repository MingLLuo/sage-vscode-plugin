#!/usr/bin/env node
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const repositoryRoot = path.resolve(__dirname, "..");

const PROFILES = {
  standard: {
    id: "standard",
    analysisMode: "default",
    enablePythonFiles: false,
    enablePyxParsing: true,
  },
  python: {
    id: "python",
    analysisMode: "full",
    enablePythonFiles: true,
    enablePyxParsing: true,
  },
  native: {
    id: "native",
    analysisMode: "full",
    enablePythonFiles: false,
    enablePyxParsing: true,
  },
  research: {
    id: "research",
    analysisMode: "full",
    enablePythonFiles: true,
    enablePyxParsing: true,
  },
};

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

const args = parseArgs(process.argv.slice(2));
const workspaceRoot = path.resolve(args.workspace ?? process.cwd());
const profile = resolveProfile(args.profile ?? "auto", workspaceRoot);
const sageRuntime = resolveSageRuntime(args.sage);
const sourceRoots = resolveSourceRoots(workspaceRoot, sageRuntime.path, args.sourceRoots);
const settingsPath = path.join(workspaceRoot, ".vscode", "settings.json");
const existingSettings = readJsonIfExists(settingsPath) ?? {};
const nextSettings = buildSettings(existingSettings, {
  profile,
  sageRuntime,
  sourceRoots,
  workspaceRoot,
});
const changed = JSON.stringify(existingSettings) !== JSON.stringify(nextSettings);
const report = {
  schema_version: 1,
  status: args.dryRun ? "dry-run" : changed ? "updated" : "unchanged",
  workspace: workspaceRoot,
  settings_path: settingsPath,
  profile: profile.id,
  platform: {
    platform: process.platform,
    arch: process.arch,
    release: os.release(),
  },
  sage_runtime: sageRuntime,
  source_roots: sourceRoots,
  changed,
  next_settings: nextSettings,
};

if (!args.dryRun && changed) {
  fs.mkdirSync(path.dirname(settingsPath), { recursive: true });
  fs.writeFileSync(settingsPath, `${JSON.stringify(nextSettings, null, 2)}\n`, "utf8");
}

if (args.json) {
  console.log(JSON.stringify(report, null, 2));
} else {
  printHuman(report);
}

if (args.requireSage && !sageRuntime.path) {
  process.exitCode = 1;
}
if (args.requireSourceRoot && sourceRoots.length === 0) {
  process.exitCode = 1;
}

function buildSettings(existing, input) {
  const sourceRootSettings = compactWorkspacePaths(input.workspaceRoot, input.sourceRoots);
  const externalSourceRoots = input.sourceRoots
    .filter((sourceRoot) => !isPathInsideOrEqual(path.resolve(sourceRoot), input.workspaceRoot))
    .map((sourceRoot) => path.resolve(sourceRoot));
  const externalSourceRootSettings = compactWorkspacePaths(input.workspaceRoot, externalSourceRoots);
  const pythonExtraPaths = sourceRootSettings.filter(
    (entry) => !externalSourceRootSettings.includes(entry),
  );
  const next = { ...existing };

  next["sage.languageServer.rustPath"] = "auto";
  next["sage.languageServer.pythonPath"] = "auto";
  next["sage.analysis.mode"] = input.profile.analysisMode;
  next["sage.analysis.enablePythonFiles"] = input.profile.enablePythonFiles;
  next["sage.analysis.enablePyxParsing"] = input.profile.enablePyxParsing;
  next["sage.analysis.enableDiagnostics"] = true;
  next["sage.analysis.enableRuntimeIntrospection"] = true;
  next["sage.analysis.sourceRoots"] = sourceRootSettings;
  next["sage.analysis.extraPaths"] = sourceRootSettings;
  next["sage.docs.preferredSource"] = "auto";
  next["sage.docs.showOnHover"] = true;
  next["sage.indexing.exclude"] = DEFAULT_INDEX_EXCLUDES;

  if (input.sageRuntime.path) {
    next["sage.interpreter.path"] = input.sageRuntime.path;
  }

  if (input.profile.enablePythonFiles) {
    next["python.analysis.extraPaths"] = pythonExtraPaths.length > 0 ? pythonExtraPaths : ["."];
    next["python.analysis.diagnosticSeverityOverrides"] = {
      ...(isPlainObject(existing["python.analysis.diagnosticSeverityOverrides"])
        ? existing["python.analysis.diagnosticSeverityOverrides"]
        : {}),
      reportMissingImports: "none",
      reportMissingModuleSource: "none",
    };
  }

  if (externalSourceRootSettings.length > 0) {
    next["python.analysis.exclude"] = mergeStringArrays(
      arrayOfStrings(existing["python.analysis.exclude"]),
      externalSourceRootSettings,
    );
    next["python.analysis.ignore"] = mergeStringArrays(
      arrayOfStrings(existing["python.analysis.ignore"]),
      externalSourceRootSettings,
    );
    next["ruff.exclude"] = mergeStringArrays(
      arrayOfStrings(existing["ruff.exclude"]),
      externalSourceRootSettings,
    );
    if (!isPlainObject(existing["ruff.configuration"]) && typeof existing["ruff.configuration"] !== "string") {
      next["ruff.configuration"] = {
        exclude: externalSourceRootSettings,
        "force-exclude": true,
      };
    } else if (isPlainObject(existing["ruff.configuration"])) {
      next["ruff.configuration"] = {
        ...existing["ruff.configuration"],
        exclude: mergeStringArrays(
          arrayOfStrings(existing["ruff.configuration"].exclude),
          externalSourceRootSettings,
        ),
        "force-exclude": true,
      };
    }
  }

  return sortObject(next);
}

function resolveProfile(rawProfile, workspace) {
  if (rawProfile !== "auto") {
    const profile = PROFILES[rawProfile];
    if (!profile) {
      fail(`unknown profile: ${rawProfile}`);
    }
    return profile;
  }
  const counts = countWorkspaceFiles(workspace);
  if (counts.cython > 0 && counts.python === 0 && counts.sage === 0) {
    return PROFILES.native;
  }
  if (counts.python > 0 && counts.sageImports > 0 && counts.cython > 0) {
    return PROFILES.research;
  }
  if (counts.python > 0 && counts.sageImports > 0) {
    return PROFILES.python;
  }
  if (counts.sage > 0) {
    return PROFILES.standard;
  }
  return PROFILES.research;
}

function countWorkspaceFiles(workspace) {
  const counts = {
    sage: 0,
    python: 0,
    cython: 0,
    sageImports: 0,
  };
  walkWorkspace(workspace, (filePath) => {
    const extension = path.extname(filePath).toLowerCase();
    if (extension === ".sage") {
      counts.sage += 1;
      return;
    }
    if ([".pyx", ".pxd", ".pxi", ".spyx"].includes(extension)) {
      counts.cython += 1;
      return;
    }
    if (extension === ".py") {
      counts.python += 1;
      const text = safeReadText(filePath, 32 * 1024);
      if (/\bfrom\s+sage\.all\s+import\b|\bimport\s+sage\b|\bsage\.all\b/.test(text)) {
        counts.sageImports += 1;
      }
    }
  });
  return counts;
}

function walkWorkspace(root, visit) {
  const stack = [root];
  const ignored = new Set([
    ".git",
    ".hg",
    ".svn",
    "__pycache__",
    ".venv",
    "venv",
    "node_modules",
    "target",
    "build",
    "dist",
    ".ruff_cache",
    ".pytest_cache",
    ".quarto",
  ]);
  let visited = 0;
  while (stack.length > 0 && visited < 5000) {
    const current = stack.pop();
    let stat;
    try {
      stat = fs.statSync(current);
    } catch {
      continue;
    }
    if (stat.isDirectory()) {
      const base = path.basename(current);
      if (ignored.has(base)) {
        continue;
      }
      for (const entry of safeReadDir(current)) {
        stack.push(path.join(current, entry));
      }
      continue;
    }
    if (stat.isFile()) {
      visited += 1;
      visit(current);
    }
  }
}

function resolveSageRuntime(configured) {
  const candidates = [
    configured,
    process.env.SAGE_PATH,
    process.env.SAGE,
    findOnPath(process.platform === "win32" ? "sage.bat" : "sage"),
    findOnPath("sage"),
    ...platformSageCandidates(),
  ].filter(Boolean);

  for (const candidate of dedupe(candidates.map((entry) => path.resolve(entry)))) {
    if (!fs.existsSync(candidate)) {
      continue;
    }
    const result = spawnSync(candidate, ["--version"], {
      encoding: "utf8",
      timeout: 8000,
      maxBuffer: 1024 * 1024,
    });
    if (result.status === 0) {
      return {
        path: candidate,
        version: firstLine(result.stdout) || firstLine(result.stderr),
        discovered: true,
      };
    }
  }
  return {
    path: null,
    version: null,
    discovered: false,
    reason: "Sage executable not found. Pass --sage PATH or set SAGE_PATH.",
  };
}

function resolveSourceRoots(workspace, sageRuntimePath, configuredRoots) {
  const candidates = [
    ...configuredRoots,
    ...discoverWorkspaceRoots(workspace),
    ...discoverNearbySageSourceRoots(workspace),
    ...discoverInterpreterRoots(sageRuntimePath),
  ];
  const roots = candidates
    .filter(Boolean)
    .map((candidate) => path.resolve(candidate))
    .filter((candidate) => fs.existsSync(candidate))
    .filter((candidate) => isSageSourceRoot(candidate) || isProjectSourceRoot(candidate));
  return dedupe(roots);
}

function discoverWorkspaceRoots(workspace) {
  const roots = [];
  if (fs.existsSync(path.join(workspace, "src", "sage"))) {
    roots.push(path.join(workspace, "src"));
  } else {
    roots.push(workspace);
  }
  return roots;
}

function discoverNearbySageSourceRoots(workspace) {
  const roots = [];
  let current = path.resolve(workspace);
  for (let depth = 0; depth < 8; depth += 1) {
    const directSrc = path.join(current, "src");
    if (fs.existsSync(path.join(directSrc, "sage"))) {
      roots.push(directSrc);
    }
    const siblingSageSrc = path.join(current, "sage", "src");
    if (fs.existsSync(path.join(siblingSageSrc, "sage"))) {
      roots.push(siblingSageSrc);
    }
    const parent = path.dirname(current);
    if (parent === current) {
      break;
    }
    current = parent;
  }
  return roots;
}

function discoverInterpreterRoots(sageRuntimePath) {
  if (!sageRuntimePath) {
    return [];
  }
  const roots = [];
  let current = path.dirname(path.resolve(sageRuntimePath));
  for (let depth = 0; depth < 5; depth += 1) {
    const srcRoot = path.join(current, "src");
    if (fs.existsSync(path.join(srcRoot, "sage"))) {
      roots.push(srcRoot);
    }
    const localSitePackages = discoverSitePackagesRoots(current);
    roots.push(...localSitePackages);
    const parent = path.dirname(current);
    if (parent === current) {
      break;
    }
    current = parent;
  }
  return roots;
}

function discoverSitePackagesRoots(prefix) {
  const roots = [];
  for (const libraryBase of [path.join(prefix, "lib"), path.join(prefix, "local", "lib")]) {
    for (const entry of safeReadDir(libraryBase)) {
      if (!entry.startsWith("python")) {
        continue;
      }
      const sitePackagesRoot = path.join(libraryBase, entry, "site-packages");
      if (fs.existsSync(path.join(sitePackagesRoot, "sage"))) {
        roots.push(sitePackagesRoot);
      }
    }
  }
  return roots;
}

function isSageSourceRoot(candidate) {
  return fs.existsSync(path.join(candidate, "sage", "__init__.py"))
    || fs.existsSync(path.join(candidate, "sage", "all.py"))
    || fs.existsSync(path.join(candidate, "sage", "all.pyx"));
}

function isProjectSourceRoot(candidate) {
  return fs.existsSync(candidate) && fs.statSync(candidate).isDirectory();
}

function compactWorkspacePaths(workspace, targetPaths) {
  const resolvedWorkspace = path.resolve(workspace);
  const seen = new Set();
  const results = [];
  for (const targetPath of targetPaths) {
    const resolvedTarget = path.resolve(targetPath);
    const compacted = isPathInsideOrEqual(resolvedTarget, resolvedWorkspace)
      ? normalizeSettingPath(path.relative(resolvedWorkspace, resolvedTarget) || ".")
      : resolvedTarget;
    const key = process.platform === "win32" ? compacted.toLowerCase() : compacted;
    if (seen.has(key)) {
      continue;
    }
    seen.add(key);
    results.push(compacted);
  }
  return results.length > 0 ? results : ["."];
}

function platformSageCandidates() {
  if (process.platform === "darwin") {
    return [
      "/Applications/SageMath/sage",
      "/Applications/SageMath.app/Contents/MacOS/sage",
      "/Applications/Sage Math.app/Contents/MacOS/sage",
    ];
  }
  if (process.platform === "win32") {
    return [
      "C:\\Program Files\\SageMath\\sage.bat",
      "C:\\SageMath\\sage.bat",
    ];
  }
  return [
    "/usr/local/bin/sage",
    "/usr/bin/sage",
    "/opt/sage/sage",
  ];
}

function findOnPath(command) {
  const pathValue = process.env.PATH ?? "";
  const pathExt = process.platform === "win32"
    ? (process.env.PATHEXT ?? ".EXE;.CMD;.BAT").split(";")
    : [""];
  for (const directory of pathValue.split(path.delimiter).filter(Boolean)) {
    for (const extension of pathExt) {
      const candidate = path.join(directory, command.endsWith(extension.toLowerCase()) ? command : `${command}${extension}`);
      if (fs.existsSync(candidate)) {
        return candidate;
      }
    }
  }
  return null;
}

function parseArgs(rawArgs) {
  const parsed = {
    dryRun: false,
    json: false,
    requireSage: false,
    requireSourceRoot: false,
    workspace: null,
    profile: "auto",
    sage: null,
    sourceRoots: [],
  };
  for (let index = 0; index < rawArgs.length; index += 1) {
    const item = rawArgs[index];
    if (item === "--dry-run") {
      parsed.dryRun = true;
      continue;
    }
    if (item === "--json") {
      parsed.json = true;
      continue;
    }
    if (item === "--require-sage") {
      parsed.requireSage = true;
      continue;
    }
    if (item === "--require-source-root") {
      parsed.requireSourceRoot = true;
      continue;
    }
    if (["--workspace", "--profile", "--sage", "--source-root"].includes(item)) {
      const value = rawArgs[index + 1];
      if (!value) {
        fail(`missing value for ${item}`);
      }
      if (item === "--source-root") {
        parsed.sourceRoots.push(value);
      } else {
        parsed[item.slice(2).replace(/-([a-z])/g, (_, letter) => letter.toUpperCase())] = value;
      }
      index += 1;
      continue;
    }
    if (item === "--help" || item === "-h") {
      console.log(`Usage: node scripts/configure-workspace.mjs [--workspace PATH] [--profile auto|standard|python|native|research]
       [--sage PATH] [--source-root PATH] [--dry-run] [--json]

Writes VS Code workspace settings for Sage language support. The script works on
macOS, Linux, and Windows and is safe to run repeatedly.`);
      process.exit(0);
    }
    fail(`unknown argument: ${item}`);
  }
  return parsed;
}

function printHuman(report) {
  console.log(`Sage workspace configure: ${report.status}`);
  console.log(`Workspace: ${report.workspace}`);
  console.log(`Profile: ${report.profile}`);
  console.log(`Settings: ${report.settings_path}`);
  console.log(`Sage: ${report.sage_runtime.path ?? "not found"}`);
  console.log(`Source roots: ${report.source_roots.length ? report.source_roots.join(", ") : "none"}`);
  console.log("");
  if (report.status === "dry-run") {
    console.log("Dry run only; no files were changed.");
  } else if (report.changed) {
    console.log("Workspace settings updated.");
  } else {
    console.log("Workspace settings were already up to date.");
  }
  console.log("Run `Sage: Restart Language Server` after reloading an open VS Code window.");
}

function readJsonIfExists(filePath) {
  try {
    return JSON.parse(fs.readFileSync(filePath, "utf8"));
  } catch {
    return null;
  }
}

function safeReadDir(directory) {
  try {
    return fs.readdirSync(directory);
  } catch {
    return [];
  }
}

function safeReadText(filePath, maxBytes) {
  try {
    const buffer = fs.readFileSync(filePath);
    return buffer.subarray(0, maxBytes).toString("utf8");
  } catch {
    return "";
  }
}

function arrayOfStrings(value) {
  return Array.isArray(value) ? value.filter((entry) => typeof entry === "string") : [];
}

function mergeStringArrays(existing, additions) {
  const result = [];
  const seen = new Set();
  for (const value of [...existing, ...additions]) {
    if (seen.has(value)) {
      continue;
    }
    seen.add(value);
    result.push(value);
  }
  return result;
}

function dedupe(values) {
  const seen = new Set();
  const results = [];
  for (const value of values) {
    const key = process.platform === "win32" ? value.toLowerCase() : value;
    if (seen.has(key)) {
      continue;
    }
    seen.add(key);
    results.push(value);
  }
  return results;
}

function isPlainObject(value) {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isPathInsideOrEqual(targetPath, folder) {
  const relative = path.relative(folder, targetPath);
  return relative === "" || (!relative.startsWith("..") && !path.isAbsolute(relative));
}

function normalizeSettingPath(value) {
  return value.split(path.sep).join("/");
}

function sortObject(value) {
  return Object.fromEntries(Object.entries(value).sort(([left], [right]) => left.localeCompare(right)));
}

function firstLine(text) {
  return String(text ?? "").trim().split(/\r?\n/).find(Boolean) ?? "";
}

function fail(message) {
  console.error(message);
  process.exit(1);
}
