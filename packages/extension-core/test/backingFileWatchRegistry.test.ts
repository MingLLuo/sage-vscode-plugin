import assert from "node:assert/strict";
import path from "node:path";
import test from "node:test";

import {
  BackingFileWatchRegistry,
  type DirectoryWatchHandle,
  type WatchDirectory,
} from "../src/backingFileWatchRegistry";

interface FakeDirectoryWatch extends DirectoryWatchHandle {
  directory: string;
  closed: number;
  change(entryName?: string): void;
}

function watcherHarness(): { watchDirectory: WatchDirectory; watches: FakeDirectoryWatch[] } {
  const watches: FakeDirectoryWatch[] = [];
  return {
    watches,
    watchDirectory: (directory, onEntryChanged) => {
      const watcher: FakeDirectoryWatch = {
        directory,
        closed: 0,
        change: onEntryChanged,
        close() {
          this.closed += 1;
        },
      };
      watches.push(watcher);
      return watcher;
    },
  };
}

test("BackingFileWatchRegistry shares directory watches and filters changed entries", () => {
  const harness = watcherHarness();
  const changed: string[] = [];
  const registry = new BackingFileWatchRegistry<string>(
    (key) => changed.push(key),
    harness.watchDirectory,
  );
  const directory = path.resolve("/tmp", "sage-source-watch");

  registry.track(path.join(directory, "first.py"), "first-uri");
  registry.track(path.join(directory, "second.py"), "second-uri");
  registry.track(path.join(directory, "first.py"), "first-uri");

  assert.equal(harness.watches.length, 1);
  harness.watches[0]?.change("first.py");
  assert.deepEqual(changed, ["first-uri"]);

  harness.watches[0]?.change();
  assert.deepEqual(new Set(changed.slice(1)), new Set(["first-uri", "second-uri"]));
});

test("BackingFileWatchRegistry releases empty directories and disposes remaining watches", () => {
  const harness = watcherHarness();
  const registry = new BackingFileWatchRegistry<string>(() => undefined, harness.watchDirectory);
  const firstDirectory = path.resolve("/tmp", "sage-source-watch-a");
  const secondDirectory = path.resolve("/tmp", "sage-source-watch-b");

  registry.track(path.join(firstDirectory, "first.py"), "first-uri");
  registry.track(path.join(firstDirectory, "second.py"), "second-uri");
  registry.track(path.join(secondDirectory, "third.py"), "third-uri");
  assert.equal(harness.watches.length, 2);

  registry.release(path.join(firstDirectory, "first.py"), "first-uri");
  assert.equal(harness.watches[0]?.closed, 0);
  registry.release(path.join(firstDirectory, "second.py"), "second-uri");
  assert.equal(harness.watches[0]?.closed, 1);

  registry.dispose();
  assert.equal(harness.watches[0]?.closed, 1);
  assert.equal(harness.watches[1]?.closed, 1);
});

test("BackingFileWatchRegistry keeps content usable when a directory cannot be watched", () => {
  const registry = new BackingFileWatchRegistry<string>(
    () => assert.fail("an unavailable watcher must not emit changes"),
    () => { throw new Error("watch denied"); },
  );

  assert.doesNotThrow(() => registry.track("/unavailable/source.py", "source-uri"));
  assert.doesNotThrow(() => registry.release("/unavailable/source.py", "source-uri"));
  assert.doesNotThrow(() => registry.dispose());
});
