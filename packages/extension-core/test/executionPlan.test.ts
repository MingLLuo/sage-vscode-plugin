import test from "node:test";
import assert from "node:assert/strict";

import {
  buildReplLoadCommand,
  buildRunFileProcessPlan,
  shouldRunInRepl,
} from "../src/executionPlan";

test("buildRunFileProcessPlan keeps executable and arguments structured", () => {
  assert.deepEqual(
    buildRunFileProcessPlan(
      {
        interpreterPath: "/Applications/Sage Math.app/Contents/MacOS/sage",
        interpreterArgs: ["--nodotsage"],
        platform: "darwin",
        environment: {},
      },
      "/tmp/example file.sage",
    ),
    {
      command: "/Applications/Sage Math.app/Contents/MacOS/sage",
      args: ["--nodotsage", "/tmp/example file.sage"],
      cwd: "/tmp",
      environment: { PYTHONPATH: "/tmp" },
      cleanupPath: undefined,
    },
  );
});

test("buildRunFileProcessPlan aligns imports with configured and inherited paths", () => {
  assert.deepEqual(
    buildRunFileProcessPlan(
      {
        interpreterPath: "sage",
        interpreterArgs: [],
        runtimePythonPaths: ["/workspace/vendor"],
        platform: "darwin",
        environment: { PYTHONPATH: "/existing" },
      },
      "/workspace/src/example.sage",
    ),
    {
      command: "sage",
      args: ["/workspace/src/example.sage"],
      cwd: "/workspace/src",
      environment: { PYTHONPATH: "/workspace/src:/workspace/vendor:/existing" },
      cleanupPath: undefined,
    },
  );
});

test("buildRunFileProcessPlan tracks generated files for cleanup", () => {
  const plan = buildRunFileProcessPlan(
    {
      interpreterPath: "sage",
      interpreterArgs: [],
      cleanupGeneratedPython: true,
      platform: "darwin",
      environment: {},
    },
    "/tmp/example file.sage",
  );

  assert.equal(plan.cleanupPath, "/tmp/example file.sage.py");
});

test("buildRunFileProcessPlan does not expose shell metacharacters for evaluation", () => {
  const filePath = "/tmp/$(touch owned); example.sage";
  const plan = buildRunFileProcessPlan(
    {
      interpreterPath: "sage",
      interpreterArgs: ["--nodotsage; echo unsafe"],
      platform: "darwin",
      environment: {},
    },
    filePath,
  );

  assert.equal(plan.command, "sage");
  assert.deepEqual(plan.args, ["--nodotsage; echo unsafe", filePath]);
});

test("buildRunFileProcessPlan uses Windows paths and separators", () => {
  assert.deepEqual(
    buildRunFileProcessPlan(
      {
        interpreterPath: "C:\\SageMath\\sage.exe",
        interpreterArgs: [],
        cleanupGeneratedPython: true,
        runtimePythonPaths: ["C:\\workspace\\vendor"],
        platform: "win32",
        environment: { PYTHONPATH: "C:\\existing" },
      },
      "C:\\workspace\\src\\example.sage",
    ),
    {
      command: "C:\\SageMath\\sage.exe",
      args: ["C:\\workspace\\src\\example.sage"],
      cwd: "C:\\workspace\\src",
      environment: {
        PYTHONPATH: "C:\\workspace\\src;C:\\workspace\\vendor;C:\\existing",
      },
      cleanupPath: "C:\\workspace\\src\\example.sage.py",
    },
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
    buildReplLoadCommand("C:\\tmp\\example.sage", [], true, "win32"),
    "import sys as __sage_vscode_sys; __sage_vscode_paths = [\"C:\\\\tmp\"]; [__sage_vscode_sys.path.insert(0, __sage_vscode_path) for __sage_vscode_path in reversed(__sage_vscode_paths) if __sage_vscode_path not in __sage_vscode_sys.path]; load(\"C:\\\\tmp\\\\example.sage\")",
  );
});

test("shouldRunInRepl reflects the configured run target", () => {
  assert.equal(shouldRunInRepl("repl"), true);
  assert.equal(shouldRunInRepl("terminal"), false);
});
