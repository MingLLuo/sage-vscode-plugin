export interface SaveableRunFileDocument {
  readonly isDirty: boolean;
  save(): PromiseLike<boolean>;
}

export type RunFilePreparationResult =
  | { ready: true; saved: boolean }
  | { ready: false; reason: "save-not-completed" }
  | { ready: false; reason: "save-failed"; error: unknown };

export async function prepareRunFileDocument(
  document: SaveableRunFileDocument,
): Promise<RunFilePreparationResult> {
  if (!document.isDirty) {
    return { ready: true, saved: false };
  }

  try {
    const saved = await document.save();
    if (!saved || document.isDirty) {
      return { ready: false, reason: "save-not-completed" };
    }
    return { ready: true, saved: true };
  } catch (error) {
    return { ready: false, reason: "save-failed", error };
  }
}
