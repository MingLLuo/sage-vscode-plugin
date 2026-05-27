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
        platform: "darwin",
      },
      "/tmp/example file.sage",
    ),
    "PYTHONPATH=/tmp${PYTHONPATH:+:$PYTHONPATH} \"/Applications/Sage Math.app/Contents/MacOS/sage\" --nodotsage \"/tmp/example file.sage\"",
  );
});

test("buildRunFileCommand aligns terminal imports with configured runtime paths", () => {
  assert.equal(
    buildRunFileCommand(
      {
        interpreterPath: "sage",
        interpreterArgs: [],
        runtimePythonPaths: ["/workspace/vendor"],
        platform: "darwin",
      },
      "/workspace/src/example.sage",
    ),
    "PYTHONPATH=/workspace/src:/workspace/vendor${PYTHONPATH:+:$PYTHONPATH} sage /workspace/src/example.sage",
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
    "__sage_status=0; PYTHONPATH=/tmp${PYTHONPATH:+:$PYTHONPATH} \"/Applications/Sage Math.app/Contents/MacOS/sage\" --nodotsage \"/tmp/example file.sage\" || __sage_status=$?; rm -f \"/tmp/example file.sage.py\"; exit $__sage_status",
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
    "import sys as __sage_vscode_sys; __sage_vscode_paths = [\"/tmp\"]; [__sage_vscode_sys.path.insert(0, __sage_vscode_path) for __sage_vscode_path in reversed(__sage_vscode_paths) if __sage_vscode_path not in __sage_vscode_sys.path]; load(\"/tmp/example \\\"quoted\\\" file.sage\")",
  );
});

test("buildReplLoadCommand aligns REPL imports with configured runtime paths", () => {
  assert.equal(
    buildReplLoadCommand("/workspace/src/example.sage", ["/workspace/vendor"]),
    "import sys as __sage_vscode_sys; __sage_vscode_paths = [\"/workspace/src\",\"/workspace/vendor\"]; [__sage_vscode_sys.path.insert(0, __sage_vscode_path) for __sage_vscode_path in reversed(__sage_vscode_paths) if __sage_vscode_path not in __sage_vscode_sys.path]; load(\"/workspace/src/example.sage\")",
  );
});

test("buildReplLoadCommand optionally removes generated .sage.py files", () => {
  assert.equal(
    buildReplLoadCommand("/tmp/example file.sage", [], true, "darwin"),
    "import sys as __sage_vscode_sys; __sage_vscode_paths = [\"/tmp\"]; [__sage_vscode_sys.path.insert(0, __sage_vscode_path) for __sage_vscode_path in reversed(__sage_vscode_paths) if __sage_vscode_path not in __sage_vscode_sys.path]; exec(\"import os as __sage_vscode_os\\n__sage_vscode_generated = \\\"/tmp/example file.sage.py\\\"\\ntry:\\n    load(\\\"/tmp/example file.sage\\\")\\nfinally:\\n    if __sage_vscode_os.path.exists(__sage_vscode_generated):\\n        __sage_vscode_os.remove(__sage_vscode_generated)\")",
  );
});

test("buildReplLoadCommand leaves Windows loads untouched when cleanup is enabled", () => {
  assert.equal(
    buildReplLoadCommand("/tmp/example.sage", [], true, "win32"),
    "import sys as __sage_vscode_sys; __sage_vscode_paths = [\"/tmp\"]; [__sage_vscode_sys.path.insert(0, __sage_vscode_path) for __sage_vscode_path in reversed(__sage_vscode_paths) if __sage_vscode_path not in __sage_vscode_sys.path]; load(\"/tmp/example.sage\")",
  );
});

test("shouldRunInRepl reflects the configured run target", () => {
  assert.equal(shouldRunInRepl("repl"), true);
  assert.equal(shouldRunInRepl("terminal"), false);
});
