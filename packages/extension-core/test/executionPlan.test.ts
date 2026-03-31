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
