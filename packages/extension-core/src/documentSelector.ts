import type { DocumentFilter, DocumentSelector } from "vscode-languageserver-protocol";

export interface SageDocumentSelectorSettings {
  pythonFilesEnabled: boolean;
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
