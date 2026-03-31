import test from "node:test";
import assert from "node:assert/strict";

import {
  formatEnvironmentDetails,
  formatStatusBarText,
  formatStatusBarTooltip,
} from "../src/environmentPresentation";

const sample = {
  interpreterPath: "/opt/sage/bin/sage",
  analysisMode: "default",
  docsSource: "workspace",
  sourceRoots: ["/workspace/src", "/workspace/vendor/src"],
  enablePyxParsing: true,
} as const;

test("formatStatusBarText emphasizes the selected interpreter like Python tools do", () => {
  assert.equal(formatStatusBarText(sample), "$(beaker) Sage: sage");
});

test("formatStatusBarTooltip includes indexing and docs context", () => {
  const tooltip = formatStatusBarTooltip(sample);
  assert.match(tooltip, /Interpreter: \/opt\/sage\/bin\/sage/);
  assert.match(tooltip, /Indexed source roots: 2/);
  assert.match(tooltip, /Preferred docs: workspace/);
});

test("formatEnvironmentDetails expands the configured source roots", () => {
  const detail = formatEnvironmentDetails(sample);
  assert.match(detail, /Source roots: \/workspace\/src, \/workspace\/vendor\/src/);
  assert.match(detail, /\.pyx parsing: on/);
});
