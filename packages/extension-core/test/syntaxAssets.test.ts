import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";

type GrammarPattern = {
  begin?: string;
  beginCaptures?: Record<string, { name?: string }>;
  captures?: Record<string, { name?: string }>;
  end?: string;
  endCaptures?: Record<string, { name?: string }>;
  match?: string;
  name?: string;
  patterns?: GrammarPattern[];
};

function readJson(relativePath: string): unknown {
  const assetPath = path.resolve(__dirname, "../../..", "syntax-pack", relativePath);
  return JSON.parse(fs.readFileSync(assetPath, "utf-8"));
}

function flattenPatternText(patterns: GrammarPattern[]): string {
  return patterns
    .flatMap((entry) => [
      entry.name ?? "",
      entry.match ?? "",
      entry.begin ?? "",
      entry.end ?? "",
      ...Object.values(entry.captures ?? {}).map((capture) => capture.name ?? ""),
      ...Object.values(entry.beginCaptures ?? {}).map((capture) => capture.name ?? ""),
      ...Object.values(entry.endCaptures ?? {}).map((capture) => capture.name ?? ""),
      flattenPatternText(entry.patterns ?? []),
    ])
    .join("\n");
}

test("syntax grammar highlights broad SageMath domains and operators", () => {
  const grammar = readJson("syntaxes/sagemath.tmLanguage.json") as {
    repository?: Record<string, { patterns?: GrammarPattern[] }>;
  };
  const supportPatterns = [
    ...(grammar.repository?.["sage-runtime-helpers"]?.patterns ?? []),
    ...(grammar.repository?.["sage-domains"]?.patterns ?? []),
    ...(grammar.repository?.["sage-namespaces"]?.patterns ?? []),
    ...(grammar.repository?.["sage-constructors"]?.patterns ?? []),
    ...(grammar.repository?.["sage-geometry"]?.patterns ?? []),
    ...(grammar.repository?.["sage-modules"]?.patterns ?? []),
    ...(grammar.repository?.["sage-linear-algebra"]?.patterns ?? []),
    ...(grammar.repository?.["sage-symbolic"]?.patterns ?? []),
    ...(grammar.repository?.["sage-plotting"]?.patterns ?? []),
    ...(grammar.repository?.["sage-number-theory"]?.patterns ?? []),
    ...(grammar.repository?.["sage-combinatorics"]?.patterns ?? []),
    ...(grammar.repository?.["sage-graph-theory"]?.patterns ?? []),
    ...(grammar.repository?.["sage-crypto"]?.patterns ?? []),
    ...(grammar.repository?.["sage-call-sites"]?.patterns ?? []),
  ];
  const operatorPatterns = grammar.repository?.operators?.patterns ?? [];
  const patternText = flattenPatternText([...supportPatterns, ...operatorPatterns]);

  assert.match(patternText, /PolynomialRing/);
  assert.match(patternText, /FunctionField/);
  assert.match(patternText, /MatrixSpace/);
  assert.match(patternText, /EllipticCurve/);
  assert.match(patternText, /ProjectiveSpace/);
  assert.match(patternText, /RootSystem/);
  assert.match(patternText, /Zmod/);
  assert.match(patternText, /BooleanFunction/);
  assert.match(patternText, /Partitions/);
  assert.match(patternText, /cartesian_product/);
  assert.match(patternText, /graphs/);
  assert.match(patternText, /codes/);
  assert.match(patternText, /polytopes/);
  assert.match(patternText, /toric_varieties/);
  assert.match(patternText, /support\.function\.method\.sagemath/);
  assert.match(patternText, /support\.function\.call\.sagemath/);
  assert.match(patternText, /cached_method/);
  assert.match(patternText, /UniqueFactory/);
  assert.match(patternText, /input_box/);
  assert.match(patternText, /FilteredSimplicialComplex/);
  assert.match(patternText, /ChowGroup/);
  assert.match(patternText, /sigma/);
  assert.match(patternText, /\^/);
  assert.match(patternText, /keyword\.operator\.range\.sagemath/);
});

test("syntax grammar distinguishes Cython declarations and modifiers", () => {
  const grammar = readJson("syntaxes/sagemath.tmLanguage.json") as {
    repository?: Record<string, { patterns?: GrammarPattern[] }>;
  };
  const cythonPatterns = [
    ...(grammar.repository?.["cython-declarations"]?.patterns ?? []),
    ...(grammar.repository?.["cython-support"]?.patterns ?? []),
  ];
  const patternText = flattenPatternText(cythonPatterns);

  assert.match(patternText, /keyword\.declaration\.cython\.sagemath/);
  assert.match(patternText, /keyword\.control\.import\.cython\.sagemath/);
  assert.match(patternText, /keyword\.control\.include\.cython\.sagemath/);
  assert.match(patternText, /storage\.modifier\.cython\.sagemath/);
  assert.match(patternText, /variable\.other\.typed\.cython\.sagemath/);
  assert.match(patternText, /cpdef/);
  assert.match(patternText, /cimport/);
  assert.match(patternText, /nogil/);
});

test("syntax grammar separates Sage preparser generators and keyword arguments", () => {
  const grammar = readJson("syntaxes/sagemath.tmLanguage.json") as {
    repository?: Record<string, { patterns?: GrammarPattern[] }>;
  };
  const preparserPatterns = grammar.repository?.["preparse-assignment"]?.patterns ?? [];
  const variableAssignmentPatterns = grammar.repository?.["sage-variable-assignment"]?.patterns ?? [];
  const keywordArgumentPatterns = grammar.repository?.["sage-keyword-arguments"]?.patterns ?? [];
  const decoratorPatterns = grammar.repository?.decorators?.patterns ?? [];
  const patternText = flattenPatternText([
    ...preparserPatterns,
    ...variableAssignmentPatterns,
    ...keywordArgumentPatterns,
    ...decoratorPatterns,
  ]);

  assert.match(patternText, /punctuation\.definition\.generator\.begin\.sagemath/);
  assert.match(patternText, /variable\.parameter\.preparse\.generator\.sagemath/);
  assert.match(patternText, /punctuation\.separator\.generator\.sagemath/);
  assert.match(patternText, /variable\.other\.assignment\.sagemath/);
  assert.match(patternText, /storage\.type\.annotation\.sagemath/);
  assert.match(patternText, /variable\.parameter\.keyword\.sagemath/);
  assert.match(patternText, /support\.function\.decorator\.interact\.sagemath/);
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
  assert.equal(snippets["Toric Variety"]?.prefix, "sagetoric");
  assert.equal(snippets["Number Field"]?.prefix, "sagenf");
  assert.equal(snippets["Cached Method"]?.prefix, "sagecache");
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
