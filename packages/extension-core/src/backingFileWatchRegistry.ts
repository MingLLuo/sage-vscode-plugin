import fs from "node:fs";
import path from "node:path";

export interface DirectoryWatchHandle {
  close(): void;
}

export type WatchDirectory = (
  directory: string,
  onEntryChanged: (entryName: string | undefined) => void,
) => DirectoryWatchHandle;

interface WatchedDirectory<Key> {
  handle: DirectoryWatchHandle;
  keysByEntry: Map<string, Set<Key>>;
}

/**
 * Shares one directory watcher between all virtual documents backed by files in
 * that directory. Watching the directory (rather than the file inode) keeps
 * refreshes working across atomic saves and Git checkouts.
 */
export class BackingFileWatchRegistry<Key> {
  private readonly directories = new Map<string, WatchedDirectory<Key>>();

  constructor(
    private readonly onChanged: (key: Key) => void,
    private readonly watchDirectory: WatchDirectory = watchDirectoryWithNode,
  ) {}

  track(filePath: string, key: Key): void {
    const resolvedPath = path.resolve(filePath);
    const directoryPath = path.dirname(resolvedPath);
    const entryName = path.basename(resolvedPath);
    let watched = this.directories.get(directoryPath);
    if (!watched) {
      const keysByEntry = new Map<string, Set<Key>>();
      let handle: DirectoryWatchHandle;
      try {
        handle = this.watchDirectory(directoryPath, (changedEntry) => {
          this.handleDirectoryChange(directoryPath, changedEntry);
        });
      } catch {
        // The content read remains useful even when the host cannot watch an
        // installed source directory. Navigation performs an additional
        // content-consistency check before forwarding positions to the LSP.
        return;
      }
      watched = { handle, keysByEntry };
      this.directories.set(directoryPath, watched);
    }
    const keys = watched.keysByEntry.get(entryName) ?? new Set<Key>();
    keys.add(key);
    watched.keysByEntry.set(entryName, keys);
  }

  release(filePath: string, key: Key): void {
    const resolvedPath = path.resolve(filePath);
    const directoryPath = path.dirname(resolvedPath);
    const entryName = path.basename(resolvedPath);
    const watched = this.directories.get(directoryPath);
    if (!watched) {
      return;
    }
    const keys = watched.keysByEntry.get(entryName);
    keys?.delete(key);
    if (keys?.size === 0) {
      watched.keysByEntry.delete(entryName);
    }
    if (watched.keysByEntry.size === 0) {
      watched.handle.close();
      this.directories.delete(directoryPath);
    }
  }

  dispose(): void {
    for (const watched of this.directories.values()) {
      watched.handle.close();
    }
    this.directories.clear();
  }

  private handleDirectoryChange(directoryPath: string, entryName: string | undefined): void {
    const watched = this.directories.get(directoryPath);
    if (!watched) {
      return;
    }
    const keys = entryName === undefined
      ? [...watched.keysByEntry.values()].flatMap((entries) => [...entries])
      : [...(watched.keysByEntry.get(entryName) ?? [])];
    for (const key of keys) {
      this.onChanged(key);
    }
  }
}

function watchDirectoryWithNode(
  directory: string,
  onEntryChanged: (entryName: string | undefined) => void,
): DirectoryWatchHandle {
  const watcher = fs.watch(
    directory,
    { persistent: false },
    (_eventType, fileName) => onEntryChanged(fileName?.toString()),
  );
  // Avoid an unhandled EventEmitter error if a removable source root vanishes.
  watcher.on("error", () => undefined);
  return watcher;
}
