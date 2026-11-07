import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { GeneratedFileCleanup } from "../src/generatedFileCleanup";

test("cleanup preserves existing files, failed launches and overlapping runs", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "sage-cleanup-"));
  const file = path.join(root, "demo.sage.py");
  try {
    const cleanup = new GeneratedFileCleanup();
    fs.writeFileSync(file, "original");
    cleanup.begin(file, true); cleanup.end(file);
    assert.equal(fs.readFileSync(file, "utf8"), "original");
    fs.unlinkSync(file);
    cleanup.begin(file, true); cleanup.begin(file, false);
    fs.writeFileSync(file, "shared");
    cleanup.end(file); cleanup.end(file);
    assert.ok(fs.existsSync(file));
    fs.unlinkSync(file);
    cleanup.begin(file, true); fs.writeFileSync(file, "failed"); cleanup.end(file, true);
    assert.ok(fs.existsSync(file));
    fs.unlinkSync(file);
    cleanup.begin(file, true); fs.writeFileSync(file, "generated"); cleanup.end(file);
    assert.equal(fs.existsSync(file), false);
  } finally { fs.rmSync(root, { recursive: true, force: true }); }
});
