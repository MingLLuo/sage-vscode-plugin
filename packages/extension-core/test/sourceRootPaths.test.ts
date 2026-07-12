import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { pathToFileURL } from "node:url";

import {
  effectiveSourceRootPaths,
  normalizeSourceRootPath,
  sourceRootContainsDocument,
  workspaceAliasedSourcePath,
} from "../src/sourceRootPaths";

test("normalizeSourceRootPath canonicalizes file URIs and symlinks", (t) => {
  const temporary = fs.mkdtempSync(path.join(os.tmpdir(), "sage-source-roots-"));
  t.after(() => fs.rmSync(temporary, { recursive: true, force: true }));
  const source = path.join(temporary, "source");
  const alias = path.join(temporary, "alias");
  fs.mkdirSync(source);
  fs.symlinkSync(source, alias, "dir");

  assert.equal(normalizeSourceRootPath(pathToFileURL(alias).toString()), fs.realpathSync(source));
  assert.equal(normalizeSourceRootPath(`${source}${path.sep}`), fs.realpathSync(source));
  assert.equal(normalizeSourceRootPath("file://%invalid"), undefined);
});

test("effectiveSourceRootPaths expands relative roots per workspace and deduplicates aliases", (t) => {
  const temporary = fs.mkdtempSync(path.join(os.tmpdir(), "sage-effective-roots-"));
  t.after(() => fs.rmSync(temporary, { recursive: true, force: true }));
  const workspaceA = path.join(temporary, "a");
  const workspaceB = path.join(temporary, "b");
  const shared = path.join(temporary, "shared");
  fs.mkdirSync(workspaceA);
  fs.mkdirSync(workspaceB);
  fs.mkdirSync(shared);

  assert.deepEqual(
    effectiveSourceRootPaths({
      configuredRoots: ["src", shared],
      indexedRoots: [pathToFileURL(shared).toString()],
      workspaceFolders: [workspaceA, workspaceB],
    }),
    [
      path.join(workspaceA, "src"),
      path.join(workspaceB, "src"),
      normalizeSourceRootPath(shared),
    ],
  );
});

test("sourceRootContainsDocument respects path boundaries", () => {
  const root = path.resolve("/tmp", "sage-root");
  assert.equal(sourceRootContainsDocument([root], path.join(root, "sage", "all.py")), true);
  assert.equal(sourceRootContainsDocument([root], root), true);
  assert.equal(sourceRootContainsDocument([root], `${root}-copy${path.sep}sage${path.sep}all.py`), false);
});

test("workspaceAliasedSourcePath preserves a lexical workspace path through an external symlink", (t) => {
  const temporary = fs.mkdtempSync(path.join(os.tmpdir(), "sage-workspace-link-"));
  t.after(() => fs.rmSync(temporary, { recursive: true, force: true }));
  const workspace = path.join(temporary, "workspace");
  const external = path.join(temporary, "external");
  const linkedRoot = path.join(workspace, "linked-sage");
  fs.mkdirSync(workspace);
  fs.mkdirSync(external);
  fs.symlinkSync(external, linkedRoot, "dir");
  const source = path.join(linkedRoot, "module.py");
  fs.writeFileSync(source, "value = 1\n");

  assert.equal(workspaceAliasedSourcePath(source, [workspace]), path.resolve(source));
  assert.notEqual(fs.realpathSync(source), path.resolve(source));
});

test("workspaceAliasedSourcePath maps canonical and symlink-root paths in both directions", (t) => {
  const temporary = fs.mkdtempSync(path.join(os.tmpdir(), "sage-workspace-alias-"));
  t.after(() => fs.rmSync(temporary, { recursive: true, force: true }));
  const physicalWorkspace = path.join(temporary, "physical-workspace");
  const workspaceAlias = path.join(temporary, "workspace-alias");
  fs.mkdirSync(physicalWorkspace);
  fs.symlinkSync(physicalWorkspace, workspaceAlias, "dir");
  const relativeSource = path.join("sage", "all.py");
  const physicalSource = path.join(physicalWorkspace, relativeSource);
  const aliasedSource = path.join(workspaceAlias, relativeSource);
  fs.mkdirSync(path.dirname(physicalSource), { recursive: true });
  fs.writeFileSync(physicalSource, "value = 1\n");

  assert.equal(
    workspaceAliasedSourcePath(physicalSource, [workspaceAlias]),
    path.join(path.resolve(workspaceAlias), relativeSource),
  );
  assert.equal(
    workspaceAliasedSourcePath(aliasedSource, [physicalWorkspace]),
    path.join(path.resolve(physicalWorkspace), relativeSource),
  );
});
