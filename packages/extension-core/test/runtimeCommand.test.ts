import test from "node:test";
import assert from "node:assert/strict";

import { buildShellCommand } from "../src/runtimeCommand";

test("buildShellCommand quotes interpreter paths and file paths with spaces", () => {
  assert.equal(
    buildShellCommand(["/Applications/Sage Math/sage", "--nodotsage", "/tmp/demo file.sage"]),
    "\"/Applications/Sage Math/sage\" --nodotsage \"/tmp/demo file.sage\"",
  );
});
