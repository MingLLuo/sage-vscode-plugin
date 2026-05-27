export interface SageActivationPolicyInput {
  activeLanguageId?: string;
  pythonFilesEnabled: boolean;
  sourceRoots: readonly string[];
  extraPaths: readonly string[];
}

export function isSageDocumentLanguage(languageId: string | undefined, pythonFilesEnabled: boolean): boolean {
  if (languageId === "sagemath" || languageId === "sagemath-cython") {
    return true;
  }
  return languageId === "python" && pythonFilesEnabled;
}

export function shouldExposeSageExperience(input: SageActivationPolicyInput): boolean {
  return isSageDocumentLanguage(input.activeLanguageId, input.pythonFilesEnabled)
    || input.pythonFilesEnabled
    || input.sourceRoots.length > 0
    || input.extraPaths.length > 0;
}

export function shouldAutoStartLanguageClient(input: SageActivationPolicyInput): boolean {
  return shouldExposeSageExperience(input);
}
