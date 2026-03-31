import test from "node:test";
import assert from "node:assert/strict";

import {
  buildInterpreterCommand,
  buildReplLoadCommand,
  buildRunFileCommand,
  shouldRunInRepl,
} from "../src/executionPlan";

test("buildInterpreterCommand quotes interpreter arguments for REPL startup", () => {
  assert.equal(
    buildInterpreterCommand({
      interpreterPath: "/Applications/Sage Math.app/Contents/MacOS/sage",
      interpreterArgs: ["--nodotsage"],
    }),
    "\"/Applications/Sage Math.app/Contents/MacOS/sage\" --nodotsage",
  );
});

test("buildRunFileCommand quotes interpreter and file paths", () => {
  assert.equal(
    buildRunFileCommand(
      {
        interpreterPath: "/Applications/Sage Math.app/Contents/MacOS/sage",
        interpreterArgs: ["--nodotsage"],
      },
      "/tmp/example file.sage",
    ),
    "\"/Applications/Sage Math.app/Contents/MacOS/sage\" --nodotsage \"/tmp/example file.sage\"",
  );
});

test("buildRunFileCommand optionally removes generated .sage.py files on POSIX shells", () => {
  assert.equal(
    buildRunFileCommand(
      {
        interpreterPath: "/Applications/Sage Math.app/Contents/MacOS/sage",
        interpreterArgs: ["--nodotsage"],
        cleanupGeneratedPython: true,
        platform: "darwin",
      },
      "/tmp/example file.sage",
    ),
    "__sage_status=0; \"/Applications/Sage Math.app/Contents/MacOS/sage\" --nodotsage \"/tmp/example file.sage\" || __sage_status=$?; rm -f \"/tmp/example file.sage.py\"; exit $__sage_status",
  );
});

test("buildRunFileCommand leaves Windows runs untouched even when cleanup is enabled", () => {
  assert.equal(
    buildRunFileCommand(
      {
        interpreterPath: "C:/SageMath/sage.exe",
        interpreterArgs: [],
        cleanupGeneratedPython: true,
        platform: "win32",
      },
      "C:/tmp/example.sage",
    ),
    "C:/SageMath/sage.exe C:/tmp/example.sage",
  );
});

test("buildReplLoadCommand escapes paths for Sage load()", () => {
  assert.equal(
    buildReplLoadCommand("/tmp/example \"quoted\" file.sage"),
    "load(\"/tmp/example \\\"quoted\\\" file.sage\")",
  );
});

test("shouldRunInRepl reflects the configured run target", () => {
  assert.equal(shouldRunInRepl("repl"), true);
  assert.equal(shouldRunInRepl("terminal"), false);
});
