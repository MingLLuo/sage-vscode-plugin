import type { DocumentFilter, DocumentSelector } from "vscode-languageserver-protocol";

export interface SageDocumentSelectorSettings {
  pythonFilesEnabled: boolean;
}

/** Escapes VS Code glob metacharacters so a filesystem path matches literally. */
export function escapeGlobPath(fsPath: string): string {
  return fsPath.replaceAll("\\", "/").replace(/[\[\]{}*?]/g, (character) => `[${character}]`);
}

/**
 * Selector for Sage files that live outside every workspace folder. Each directory is matched
 * non-recursively so a loose-file server never claims documents owned by a folder server.
 */
export function buildLooseDocumentSelector(
  settings: SageDocumentSelectorSettings,
  directories: readonly string[],
): DocumentSelector {
  const languages = ["sagemath", "sagemath-cython", ...(settings.pythonFilesEnabled ? ["python"] : [])];
  return directories.flatMap((directory) =>
    languages.map((language) => ({ language, scheme: "file", pattern: `${escapeGlobPath(directory)}/*` })),
  );
}

export function buildDocumentSelector(
  settings: SageDocumentSelectorSettings,
): DocumentSelector {
  // Keep the supported schemes explicit so the read-only sage-source scheme is
  // owned exclusively by externalSourceNavigation. Untitled documents retain
  // their existing routing; path-dependent server features activate after save.
  const selector: DocumentFilter[] = [
    { language: "sagemath", scheme: "file" },
    { language: "sagemath", scheme: "untitled" },
    { language: "sagemath-cython", scheme: "file" },
    { language: "sagemath-cython", scheme: "untitled" },
  ];
  if (settings.pythonFilesEnabled) {
    selector.push({ language: "python", scheme: "file" });
  }
  return selector;
}
