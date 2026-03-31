import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";

function readJson(relativePath: string): unknown {
  const assetPath = path.resolve(__dirname, "../../..", "syntax-pack", relativePath);
  return JSON.parse(fs.readFileSync(assetPath, "utf-8"));
}

test("syntax grammar highlights broad SageMath domains and operators", () => {
  const grammar = readJson("syntaxes/sagemath.tmLanguage.json") as {
    repository?: Record<string, { patterns?: Array<{ match?: string }> }>;
  };
  const supportPatterns = grammar.repository?.["sage-support"]?.patterns ?? [];
  const operatorPatterns = grammar.repository?.operators?.patterns ?? [];
  const patternText = [...supportPatterns, ...operatorPatterns]
    .map((entry) => entry.match ?? "")
    .join("\n");

  assert.match(patternText, /PolynomialRing/);
  assert.match(patternText, /MatrixSpace/);
  assert.match(patternText, /EllipticCurve/);
  assert.match(patternText, /BooleanFunction/);
  assert.match(patternText, /Partitions/);
  assert.match(patternText, /graphs/);
  assert.match(patternText, /sigma/);
  assert.match(patternText, /\^/);
});

test("syntax snippets cover common SageMath authoring patterns", () => {
  const snippets = readJson("snippets/sagemath.json") as Record<
    string,
    { prefix?: string; body?: string[] }
  >;

  assert.equal(snippets["Polynomial Ring"]?.prefix, "sagepoly");
  assert.ok(snippets["Polynomial Ring"]?.body?.some((line) => line.includes("PolynomialRing")));
  assert.equal(snippets["Finite Field"]?.prefix, "sagegf");
  assert.equal(snippets["Matrix Builder"]?.prefix, "sagematrix");
  assert.equal(snippets["Graph Constructor"]?.prefix, "sagegraph");
  assert.equal(snippets["Plot Builder"]?.prefix, "sageplot");
  assert.equal(snippets["Elliptic Curve"]?.prefix, "sageec");
});

test("language configuration supports triple quotes and bracket-driven indentation", () => {
  const config = readJson("language-configuration.json") as {
    autoClosingPairs?: Array<{ open: string; close: string }>;
    indentationRules?: { increaseIndentPattern?: string; decreaseIndentPattern?: string };
  };

  const autoClosingPairs = config.autoClosingPairs ?? [];
  assert.ok(autoClosingPairs.some((pair) => pair.open === '"""' && pair.close === '"""'));
  assert.ok(autoClosingPairs.some((pair) => pair.open === "'''" && pair.close === "'''"));
  assert.match(config.indentationRules?.increaseIndentPattern ?? "", /\[\\\(\{]/);
  assert.match(config.indentationRules?.decreaseIndentPattern ?? "", /elif\|else\|except\|finally/);
});
