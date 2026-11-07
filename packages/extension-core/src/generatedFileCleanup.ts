import fs from "node:fs";

// Only an uncontended run that began without a helper owns its cleanup.
export class GeneratedFileCleanup {
  private readonly active = new Map<string, { count: number; owned: boolean }>();

  begin(file: string, requested: boolean): void {
    const existing = this.active.get(file);
    if (existing) {
      existing.count += 1;
      existing.owned = false;
      return;
    }
    let absent = false;
    try {
      fs.lstatSync(file);
    } catch (error) {
      absent = (error as NodeJS.ErrnoException).code === "ENOENT";
    }
    this.active.set(file, { count: 1, owned: requested && absent });
  }

  end(file: string, failedLaunch = false): void {
    const entry = this.active.get(file);
    if (!entry) return;
    if (failedLaunch) entry.owned = false;
    if (--entry.count > 0) return;
    this.active.delete(file);
    if (entry.owned) {
      try {
        // Never follow or remove a replacement symlink/directory.
        if (fs.lstatSync(file).isFile()) fs.unlinkSync(file);
      } catch { /* Missing or inaccessible helpers may be retained safely. */ }
    }
  }
}
