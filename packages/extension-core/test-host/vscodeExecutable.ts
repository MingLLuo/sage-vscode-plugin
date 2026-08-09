import fs from "node:fs/promises";

export interface ResolveVSCodeExecutableOptions {
  override?: string;
  platform?: NodeJS.Platform;
  pathExists?: (candidate: string) => Promise<boolean>;
}

export function vscodeExecutableCandidates(
  platform: NodeJS.Platform = process.platform,
): readonly string[] {
  if (platform === "darwin") {
    return [
      "/Applications/Visual Studio Code.app/Contents/MacOS/Code",
      "/Applications/Visual Studio Code.app/Contents/MacOS/Electron",
      "/Applications/Visual Studio Code - Insiders.app/Contents/MacOS/Code - Insiders",
      "/Applications/Visual Studio Code - Insiders.app/Contents/MacOS/Electron",
    ];
  }
  if (platform === "win32") {
    return [
      "C:\\Program Files\\Microsoft VS Code\\Code.exe",
      "C:\\Program Files\\Microsoft VS Code Insiders\\Code - Insiders.exe",
    ];
  }
  return [
    "/usr/share/code/code",
    "/snap/bin/code",
    "/usr/share/code-insiders/code-insiders",
  ];
}

export async function resolveVSCodeExecutablePath(
  options: ResolveVSCodeExecutableOptions = {},
): Promise<string> {
  const pathExists = options.pathExists ?? defaultPathExists;
  if (options.override && await pathExists(options.override)) {
    return options.override;
  }

  const candidates = vscodeExecutableCandidates(options.platform);
  for (const candidate of candidates) {
    if (await pathExists(candidate)) {
      return candidate;
    }
  }

  throw new Error(
    "Could not locate a local VS Code executable. Set SAGE_TEST_VSCODE_EXECUTABLE to override the path.",
  );
}

async function defaultPathExists(candidate: string): Promise<boolean> {
  try {
    await fs.access(candidate);
    return true;
  } catch {
    return false;
  }
}
