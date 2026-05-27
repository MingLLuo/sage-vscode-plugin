import test from "node:test";
import assert from "node:assert/strict";

import { currentSageCell, sageCellMarkers } from "../src/sageCells";

test("currentSageCell selects code between Python-style cell markers", () => {
  const source = [
    "# %% setup",
    "R = PolynomialRing(QQ, 'x')",
    "x = R.gen()",
    "",
    "# %% solve",
    "I = R.ideal(x^2 + 1)",
    "I.variety()",
  ].join("\n");

  assert.deepEqual(currentSageCell(source, 2), {
    startLine: 1,
    endLine: 2,
    text: "R = PolynomialRing(QQ, 'x')\nx = R.gen()",
  });
  assert.deepEqual(currentSageCell(source, 4), {
    startLine: 5,
    endLine: 6,
    text: "I = R.ideal(x^2 + 1)\nI.variety()",
  });
});

test("currentSageCell supports region markers and trims empty edges", () => {
  const source = [
    "alpha = 1",
    "",
    "# region factor base",
    "",
    "M = matrix(QQ, 2, 2)",
    "",
    "# endregion",
    "omega = 2",
  ].join("\n");

  assert.deepEqual(currentSageCell(source, 4), {
    startLine: 4,
    endLine: 4,
    text: "M = matrix(QQ, 2, 2)",
  });
  assert.deepEqual(currentSageCell(source, 0), {
    startLine: 0,
    endLine: 0,
    text: "alpha = 1",
  });
});

test("currentSageCell falls back to the whole document and ignores empty cells", () => {
  assert.deepEqual(currentSageCell("a = 1\nb = 2", 20), {
    startLine: 0,
    endLine: 1,
    text: "a = 1\nb = 2",
  });
  assert.equal(currentSageCell("# %%\n\n# %% next\n", 0), undefined);
});

test("sageCellMarkers returns runnable cell starts without endregion boundaries", () => {
  const source = [
    "# %% setup",
    "R = PolynomialRing(QQ, 'x')",
    "# region solve block",
    "I = R.ideal(x^2 + 1)",
    "# endregion",
    "# %% ",
    "I.variety()",
  ].join("\n");

  assert.deepEqual(sageCellMarkers(source), [
    { line: 0, kind: "cell", label: "setup" },
    { line: 2, kind: "region", label: "solve block" },
    { line: 5, kind: "cell", label: "Sage cell" },
  ]);
});
