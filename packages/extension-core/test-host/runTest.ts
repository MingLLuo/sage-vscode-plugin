import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";

import { runTests } from "@vscode/test-electron";

async function main(): Promise<void> {
  const repositoryRoot = path.resolve(__dirname, "../../../..");
  const extensionDevelopmentPath = path.join(repositoryRoot, "packages/extension-core");
  const extensionTestsPath = path.join(extensionDevelopmentPath, "out", "test-host", "smoke.js");
  const vscodeExecutablePath = await resolveVSCodeExecutablePath();
  const workspacePath = await cloneSmokeWorkspace(repositoryRoot);

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
    ],
    extensionTestsEnv: {
      SAGE_TEST_LSP_PYTHON: process.env.SAGE_TEST_LSP_PYTHON ?? "python",
      SAGE_TEST_WORKSPACE: workspacePath,
    },
  });

  if (exitCode !== 0) {
    throw new Error(`extension-host smoke tests failed with exit code ${exitCode}`);
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

async function cloneSmokeWorkspace(repositoryRoot: string): Promise<string> {
  const sourceWorkspace = path.join(repositoryRoot, "examples", "manual-smoke-workspace");
  const tempRoot = await fs.mkdtemp(path.join(os.tmpdir(), "sage-vscode-smoke-"));
  const targetWorkspace = path.join(tempRoot, "workspace");
  await fs.cp(sourceWorkspace, targetWorkspace, { recursive: true });
  return targetWorkspace;
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
