import path from "node:path";

export interface LanguageServerLaunch {
  command: string;
  args: string[];
}

export function buildLanguageServerLaunch(
  interpreterPath: string,
  interpreterArgs: readonly string[],
): LanguageServerLaunch {
  const baseName = path.basename(interpreterPath).toLowerCase();
  const looksLikePython = baseName.startsWith("python");
  const args = [...interpreterArgs];

  if (looksLikePython) {
    return {
      command: interpreterPath,
      args: [...args, "-m", "sage_lsp"],
    };
  }

  if (!args.includes("-python")) {
    args.push("-python");
  }

  return {
    command: interpreterPath,
    args: [...args, "-m", "sage_lsp"],
  };
}

