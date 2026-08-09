import assert from "node:assert/strict";
import test from "node:test";

import {
  resolveVSCodeExecutablePath,
  vscodeExecutableCandidates,
} from "../test-host/vscodeExecutable";

test("macOS candidates prefer current Code binaries while retaining Electron compatibility", () => {
  assert.deepEqual(vscodeExecutableCandidates("darwin"), [
    "/Applications/Visual Studio Code.app/Contents/MacOS/Code",
    "/Applications/Visual Studio Code.app/Contents/MacOS/Electron",
    "/Applications/Visual Studio Code - Insiders.app/Contents/MacOS/Code - Insiders",
    "/Applications/Visual Studio Code - Insiders.app/Contents/MacOS/Electron",
  ]);
});

test("an existing explicit executable override wins", async () => {
  const visited: string[] = [];
  const resolved = await resolveVSCodeExecutablePath({
    override: "/custom/Code",
    platform: "darwin",
    pathExists: async (candidate) => {
      visited.push(candidate);
      return candidate === "/custom/Code";
    },
  });

  assert.equal(resolved, "/custom/Code");
  assert.deepEqual(visited, ["/custom/Code"]);
});

test("an unavailable override falls back to installed candidates", async () => {
  const resolved = await resolveVSCodeExecutablePath({
    override: "/missing/Code",
    platform: "darwin",
    pathExists: async (candidate) => (
      candidate === "/Applications/Visual Studio Code.app/Contents/MacOS/Code"
    ),
  });

  assert.equal(resolved, "/Applications/Visual Studio Code.app/Contents/MacOS/Code");
});

test("legacy stable and current Insiders macOS binaries remain discoverable", async () => {
  const legacyStable = await resolveVSCodeExecutablePath({
    platform: "darwin",
    pathExists: async (candidate) => candidate.endsWith("/MacOS/Electron"),
  });
  assert.equal(legacyStable, "/Applications/Visual Studio Code.app/Contents/MacOS/Electron");

  const currentInsiders = await resolveVSCodeExecutablePath({
    platform: "darwin",
    pathExists: async (candidate) => candidate.endsWith("/MacOS/Code - Insiders"),
  });
  assert.equal(
    currentInsiders,
    "/Applications/Visual Studio Code - Insiders.app/Contents/MacOS/Code - Insiders",
  );
});

test("Windows and Linux candidate ordering remains stable", () => {
  assert.deepEqual(vscodeExecutableCandidates("win32"), [
    "C:\\Program Files\\Microsoft VS Code\\Code.exe",
    "C:\\Program Files\\Microsoft VS Code Insiders\\Code - Insiders.exe",
  ]);
  assert.deepEqual(vscodeExecutableCandidates("linux"), [
    "/usr/share/code/code",
    "/snap/bin/code",
    "/usr/share/code-insiders/code-insiders",
  ]);
});

test("missing local installations produce an actionable error", async () => {
  await assert.rejects(
    resolveVSCodeExecutablePath({
      platform: "linux",
      pathExists: async () => false,
    }),
    /SAGE_TEST_VSCODE_EXECUTABLE/,
  );
});
