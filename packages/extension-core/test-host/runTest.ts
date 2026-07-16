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
  const smokeWorkspacePath = await cloneSmokeWorkspace(repositoryRoot, tempRoot);
  const externalSageSourceRoot = await createExternalSageSourceRoot(tempRoot);
  const plainPythonWorkspacePath = await createPlainPythonWorkspace(tempRoot);
  let completedSuccessfully = false;

  try {
    await runExtensionHostSuite({
      vscodeExecutablePath,
      extensionDevelopmentPath,
      extensionTestsPath,
      workspacePath: plainPythonWorkspacePath,
      userDataDir: path.join(tempRoot, "userdata-plain-python"),
      extensionsDir: path.join(tempRoot, "extensions-plain-python"),
      mode: "plain-python",
    });

    await runExtensionHostSuite({
      vscodeExecutablePath,
      extensionDevelopmentPath,
      extensionTestsPath,
      workspacePath: smokeWorkspacePath,
      userDataDir: path.join(tempRoot, "userdata-smoke"),
      extensionsDir: path.join(tempRoot, "extensions-smoke"),
      mode: "smoke",
      nativeSage,
      externalSageSourceRoot,
    });

    completedSuccessfully = true;
  } finally {
    if (completedSuccessfully && process.env.SAGE_TEST_KEEP_TEMP !== "1") {
      await fs.rm(tempRoot, { recursive: true, force: true });
    } else if (!completedSuccessfully) {
      console.error(`Preserved extension-host temp workspace for debugging: ${tempRoot}`);
    }
  }
}

async function runExtensionHostSuite(options: {
  vscodeExecutablePath: string;
  extensionDevelopmentPath: string;
  extensionTestsPath: string;
  workspacePath: string;
  userDataDir: string;
  extensionsDir: string;
  mode: "plain-python" | "smoke";
  nativeSage?: { sourceRoot?: string; executable?: string };
  externalSageSourceRoot?: string;
}): Promise<void> {
  const exitCode = await runTests({
    vscodeExecutablePath: options.vscodeExecutablePath,
    extensionDevelopmentPath: options.extensionDevelopmentPath,
    extensionTestsPath: options.extensionTestsPath,
    launchArgs: [
      options.workspacePath,
      "--disable-extensions",
      "--skip-release-notes",
      "--skip-welcome",
      "--disable-workspace-trust",
      `--user-data-dir=${options.userDataDir}`,
      `--extensions-dir=${options.extensionsDir}`,
    ],
    extensionTestsEnv: {
      SAGE_TEST_HOST_MODE: options.mode,
      SAGE_TEST_LSP_PYTHON: process.env.SAGE_TEST_LSP_PYTHON ?? "python",
      SAGE_TEST_WORKSPACE: options.workspacePath,
      ...(options.mode === "smoke"
        ? {
          SAGE_LS_TEST_BACKGROUND_DELAY_MS:
            process.env.SAGE_LS_TEST_BACKGROUND_DELAY_MS ?? "3000",
        }
        : {}),
      ...(options.nativeSage?.sourceRoot ? { SAGE_TEST_NATIVE_SOURCE_ROOT: options.nativeSage.sourceRoot } : {}),
      ...(options.nativeSage?.executable ? { SAGE_TEST_NATIVE_SAGE_EXECUTABLE: options.nativeSage.executable } : {}),
      ...(options.externalSageSourceRoot ? { SAGE_TEST_EXTERNAL_SOURCE_ROOT: options.externalSageSourceRoot } : {}),
    },
  });

  if (exitCode !== 0) {
    throw new Error(`extension-host ${options.mode} smoke tests failed with exit code ${exitCode}`);
  }

  await assertCleanLogs(options.userDataDir);
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
  await fs.writeFile(
    path.join(targetWorkspace, "src", "__external_navigation_bridge.sage"),
    [
      "external_navigation_value = ExternalSmokeCombinations([1, 2, 3], 2)",
      "",
    ].join("\n"),
  );
  return targetWorkspace;
}

async function createPlainPythonWorkspace(tempRoot: string): Promise<string> {
  const targetWorkspace = path.join(tempRoot, "plain-python-workspace");
  await fs.mkdir(targetWorkspace, { recursive: true });
  await fs.writeFile(
    path.join(targetWorkspace, "plain.py"),
    [
      "\"\"\"Ordinary Python file used to prove Sage stays quiet by default.\"\"\"",
      "",
      "def add(left: int, right: int) -> int:",
      "    return left + right",
      "",
    ].join("\n"),
  );
  return targetWorkspace;
}

async function createExternalSageSourceRoot(tempRoot: string): Promise<string> {
  const sourceRoot = path.join(tempRoot, "external-sage-src");
  const sagePackage = path.join(sourceRoot, "sage");
  const combinatPackage = path.join(sagePackage, "combinat");
  await fs.mkdir(combinatPackage, { recursive: true });
  await fs.writeFile(path.join(sagePackage, "__init__.py"), "\n");
  await fs.writeFile(path.join(combinatPackage, "__init__.py"), "\n");
  await fs.writeFile(
    path.join(combinatPackage, "linked.sage"),
    "linked_external_source_value = 1\n",
  );
  await fs.writeFile(
    path.join(sagePackage, "all.py"),
    [
      "\"\"\"Minimal Sage public export surface used by the extension-host smoke.\"\"\"",
      "",
      "from sage.combinat.combination import Combinations",
      "from sage.combinat.combination import ExternalSmokeCombinations",
      "",
      "__all__ = [\"Combinations\", \"ExternalSmokeCombinations\"]",
      "",
    ].join("\n"),
  );
  await fs.writeFile(
    path.join(combinatPackage, "combination.py"),
    [
      "\"\"\"Minimal external Sage source fixture used outside the workspace.\"\"\"",
      "",
      "load(\"linked.sage\")",
      "",
      "def Combinations(n, k=None):",
      "    \"\"\"Return combinations of ``n`` objects, optionally of length ``k``.\"\"\"",
      "    return []",
      "",
      "def ExternalSmokeLeaf(value):",
      "    \"\"\"Return the external leaf value used by call hierarchy smoke tests.\"\"\"",
      "    return value",
      "",
      "def ExternalSmokeCombinations(n, k=None):",
      "    \"\"\"Unique external symbol used to verify read-only navigation bridges.\"\"\"",
      "    return ExternalSmokeLeaf(n)",
      "",
      "def ExternalSmokeCaller():",
      "    \"\"\"Call the external combination helper for hierarchy and signature tests.\"\"\"",
      "    return ExternalSmokeCombinations([1, 2, 3], 2)",
      "",
    ].join("\n"),
  );
  return sourceRoot;
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
    || logPath.endsWith("main.log")
    || logPath.endsWith("exthost.log"),
  );

  const failures: string[] = [];
  const failurePatterns = [
    /Accessing a window scoped configuration for a resource/g,
    /\[sage-vscode\.sage-vscode-extension\] Accessing a resource scoped configuration without providing a resource/g,
    /Error sending data/g,
    /Traceback \(most recent call last\):/g,
    /Server process exited with code (?!0(?:[\s.,]|$))/g,
    /language server connection closed unexpectedly; restarting\./g,
    /Extension host .* is unresponsive/g,
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
