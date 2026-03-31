import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";

import { runTests } from "@vscode/test-electron";

async function main(): Promise<void> {
  const repositoryRoot = path.resolve(__dirname, "../../../..");
  const extensionDevelopmentPath = path.join(repositoryRoot, "packages/extension-core");
  const extensionTestsPath = path.join(extensionDevelopmentPath, "out", "test-host", "smoke.js");
  const vscodeExecutablePath = await resolveVSCodeExecutablePath();
  const nativeSage = await discoverNativeSagePaths(repositoryRoot);
  const tempRoot = await fs.mkdtemp(path.join(os.tmpdir(), "sgvsc-"));
  const workspacePath = await cloneSmokeWorkspace(repositoryRoot, tempRoot);
  const userDataDir = path.join(tempRoot, "userdata");
  const extensionsDir = path.join(tempRoot, "extensions");
  let completedSuccessfully = false;

  try {
    const exitCode = await runTests({
      vscodeExecutablePath,
      extensionDevelopmentPath,
      extensionTestsPath,
      launchArgs: [
        workspacePath,
        "--disable-extensions",
        "--skip-release-notes",
        "--skip-welcome",
        "--disable-workspace-trust",
        `--user-data-dir=${userDataDir}`,
        `--extensions-dir=${extensionsDir}`,
      ],
      extensionTestsEnv: {
        SAGE_TEST_LSP_PYTHON: process.env.SAGE_TEST_LSP_PYTHON ?? "python",
        SAGE_TEST_WORKSPACE: workspacePath,
        ...(nativeSage.sourceRoot ? { SAGE_TEST_NATIVE_SOURCE_ROOT: nativeSage.sourceRoot } : {}),
        ...(nativeSage.executable ? { SAGE_TEST_NATIVE_SAGE_EXECUTABLE: nativeSage.executable } : {}),
      },
    });

    if (exitCode !== 0) {
      throw new Error(`extension-host smoke tests failed with exit code ${exitCode}`);
    }

    await assertCleanLogs(userDataDir);
    completedSuccessfully = true;
  } finally {
    if (completedSuccessfully && process.env.SAGE_TEST_KEEP_TEMP !== "1") {
      await fs.rm(tempRoot, { recursive: true, force: true });
    } else if (!completedSuccessfully) {
      console.error(`Preserved extension-host temp workspace for debugging: ${tempRoot}`);
    }
  }
}

async function resolveVSCodeExecutablePath(): Promise<string> {
  const override = process.env.SAGE_TEST_VSCODE_EXECUTABLE;
  if (override && (await pathExists(override))) {
    return override;
  }

  const candidates =
    process.platform === "darwin"
      ? [
          "/Applications/Visual Studio Code.app/Contents/MacOS/Electron",
          "/Applications/Visual Studio Code - Insiders.app/Contents/MacOS/Electron",
        ]
      : process.platform === "win32"
        ? [
            "C:\\Program Files\\Microsoft VS Code\\Code.exe",
            "C:\\Program Files\\Microsoft VS Code Insiders\\Code - Insiders.exe",
          ]
        : [
            "/usr/share/code/code",
            "/snap/bin/code",
            "/usr/share/code-insiders/code-insiders",
          ];

  for (const candidate of candidates) {
    if (await pathExists(candidate)) {
      return candidate;
    }
  }

  throw new Error(
    "Could not locate a local VS Code executable. Set SAGE_TEST_VSCODE_EXECUTABLE to override the path.",
  );
}

async function cloneSmokeWorkspace(repositoryRoot: string, tempRoot: string): Promise<string> {
  const sourceWorkspace = path.join(repositoryRoot, "examples", "manual-smoke-workspace");
  const targetWorkspace = path.join(tempRoot, "workspace");
  await fs.cp(sourceWorkspace, targetWorkspace, { recursive: true });
  return targetWorkspace;
}

async function discoverNativeSagePaths(
  repositoryRoot: string,
): Promise<{ sourceRoot?: string; executable?: string }> {
  const sourceRootCandidates = [
    process.env.SAGE_TEST_NATIVE_SOURCE_ROOT,
    path.join(repositoryRoot, "..", "sage", "src"),
  ].filter(Boolean) as string[];
  const executableCandidates = [
    process.env.SAGE_TEST_NATIVE_SAGE_EXECUTABLE,
    path.join(repositoryRoot, "..", "sage", "sage"),
  ].filter(Boolean) as string[];

  const sourceRoot = await firstExistingPath(sourceRootCandidates, async (candidate) =>
    pathExists(path.join(candidate, "sage")),
  );
  const executable = await firstExistingPath(executableCandidates, pathExists);

  return { sourceRoot, executable };
}

async function assertCleanLogs(userDataDir: string): Promise<void> {
  const logsRoot = path.join(userDataDir, "logs");
  if (!(await pathExists(logsRoot))) {
    return;
  }

  const candidateLogs = (await collectFiles(logsRoot)).filter((logPath) =>
    logPath.endsWith("1-Sage.log")
    || logPath.endsWith("2-Sage Language Server.log")
    || logPath.endsWith("exthost.log"),
  );

  const failures: string[] = [];
  const failurePatterns = [
    /Accessing a window scoped configuration for a resource/g,
    /Error sending data/g,
    /Traceback \(most recent call last\):/g,
    /Server process exited with code (?!0(?:[\s.,]|$))/g,
    /language server connection closed unexpectedly; restarting\./g,
  ];

  for (const logPath of candidateLogs) {
    const contents = await fs.readFile(logPath, "utf-8");
    const matches = failurePatterns.filter((pattern) => pattern.test(contents));
    if (matches.length > 0) {
      failures.push(`${logPath}: ${matches.map((pattern) => pattern.source).join(", ")}`);
    }
  }

  if (failures.length > 0) {
    throw new Error(`extension-host logs contained unexpected failures:\n${failures.join("\n")}`);
  }
}

async function collectFiles(root: string): Promise<string[]> {
  const entries = await fs.readdir(root, { withFileTypes: true });
  const files: string[] = [];

  for (const entry of entries) {
    const targetPath = path.join(root, entry.name);
    if (entry.isDirectory()) {
      files.push(...(await collectFiles(targetPath)));
      continue;
    }
    if (entry.isFile()) {
      files.push(targetPath);
    }
  }

  return files;
}

async function firstExistingPath(
  candidates: readonly string[],
  matcher: (candidate: string) => Promise<boolean>,
): Promise<string | undefined> {
  for (const candidate of candidates) {
    if (await matcher(candidate)) {
      return candidate;
    }
  }
  return undefined;
}

async function pathExists(targetPath: string): Promise<boolean> {
  try {
    await fs.access(targetPath);
    return true;
  } catch {
    return false;
  }
}

void main().catch((error) => {
  console.error(error);
  process.exit(1);
});
