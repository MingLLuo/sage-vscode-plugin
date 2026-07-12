import test from "node:test";
import assert from "node:assert/strict";

import { updateWorkspaceSettingJson } from "../src/workspaceSettingsJson";

test("updateWorkspaceSettingJson adds a setting without removing JSONC comments", () => {
  const source = `{
  // Keep this explanation for collaborators.
  "editor.formatOnSave": true,
}
`;

  const updated = updateWorkspaceSettingJson(source, "ruff.configuration", {
    lint: { select: ["E", "F"] },
  });

  assert.match(updated, /Keep this explanation/);
  assert.match(updated, /"editor\.formatOnSave": true/);
  assert.deepEqual(parseJsonc(updated)["ruff.configuration"], {
    lint: { select: ["E", "F"] },
  });
});

test("updateWorkspaceSettingJson updates an existing setting and preserves indentation", () => {
  const source = "{\n\t\"ruff.configuration\": { \"old\": true }\n}\n";
  const updated = updateWorkspaceSettingJson(source, "ruff.configuration", { fresh: true });

  assert.match(updated, /\n\t"ruff\.configuration"/);
  assert.deepEqual(parseJsonc(updated)["ruff.configuration"], { fresh: true });
});

test("updateWorkspaceSettingJson creates an object for an empty settings file", () => {
  assert.deepEqual(
    parseJsonc(updateWorkspaceSettingJson("", "sage.analysis.mode", "full")),
    { "sage.analysis.mode": "full" },
  );
});

test("updateWorkspaceSettingJson accepts and preserves a UTF-8 BOM", () => {
  const source = "\uFEFF{\r\n  // Keep the BOM and line endings.\r\n  \"editor.formatOnSave\": true,\r\n}\r\n";
  const updated = updateWorkspaceSettingJson(source, "sage.analysis.mode", "full");

  assert.ok(updated.startsWith("\uFEFF"));
  assert.match(updated, /Keep the BOM and line endings/);
  assert.ok(updated.includes("\r\n"));
  assert.deepEqual(parseJsonc(updated), {
    "editor.formatOnSave": true,
    "sage.analysis.mode": "full",
  });
});

test("updateWorkspaceSettingJson rejects malformed JSONC instead of overwriting it", () => {
  assert.throws(
    () => updateWorkspaceSettingJson("{ invalid", "sage.analysis.mode", "full"),
    /settings\.json is invalid/,
  );
});

test("updateWorkspaceSettingJson rejects duplicate keys instead of updating an ineffective value", () => {
  assert.throws(
    () => updateWorkspaceSettingJson(
      '{ "sage.analysis.mode": "light", "sage.analysis.mode": "full" }',
      "sage.analysis.mode",
      "default",
    ),
    /contains the setting more than once/,
  );
});

function parseJsonc(source: string): Record<string, unknown> {
  const withoutComments = source.replace(/^\uFEFF/, "").replace(/^\s*\/\/.*$/gm, "");
  return JSON.parse(withoutComments.replace(/,\s*}/g, "}")) as Record<string, unknown>;
}
