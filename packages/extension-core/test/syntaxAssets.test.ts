import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { INITIAL, Registry, parseRawGrammar, type IGrammar, type StateStack } from "vscode-textmate";
import { loadWASM, OnigScanner, OnigString } from "vscode-oniguruma";

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

async function loadSageGrammar(): Promise<IGrammar> {
  const wasm = fs.readFileSync(require.resolve("vscode-oniguruma/release/onig.wasm"));
  await loadWASM(wasm.buffer.slice(wasm.byteOffset, wasm.byteOffset + wasm.byteLength));
  const grammarSource = fs.readFileSync(
    path.resolve(__dirname, "../../../syntax-pack/syntaxes/sagemath.tmLanguage.json"),
    "utf-8",
  );
  const registry = new Registry({
    onigLib: Promise.resolve({
      createOnigScanner: (sources) => new OnigScanner(sources),
      createOnigString: (source) => new OnigString(source),
    }),
    loadGrammar: async (scopeName) => scopeName === "source.sagemath"
      ? parseRawGrammar(grammarSource, "sagemath.tmLanguage.json")
      : null,
  });
  const grammar = await registry.loadGrammar("source.sagemath");
  assert.ok(grammar);
  return grammar;
}

function scopesForText(grammar: IGrammar, source: string, lineIndex: number, needle: string): string[] {
  let state: StateStack = INITIAL;
  const lines = source.split("\n");
  for (let index = 0; index <= lineIndex; index += 1) {
    const line = lines[index] ?? "";
    const result = grammar.tokenizeLine(line, state);
    state = result.ruleStack;
    if (index === lineIndex) {
      const offset = line.indexOf(needle);
      assert.ok(offset >= 0, `missing ${needle} on line ${lineIndex}`);
      const token = result.tokens.find((item) => item.startIndex <= offset && offset < item.endIndex);
      assert.ok(token, `missing token for ${needle}`);
      return token.scopes;
    }
  }
  throw new Error(`missing line ${lineIndex}`);
}

test("TextMate tokenization recognizes Unicode Sage and Cython identifiers", async () => {
  const grammar = await loadSageGrammar();
  const source = [
    "def 变量(数):",
    "    return 数",
    "R.<参数> = PolynomialRing(QQ)",
    "结果 = 变量(参数)",
    "cdef int 计数 = 1",
    "# 变量(数)",
    "text = '变量(数)'",
  ].join("\n");
  assert.ok(scopesForText(grammar, source, 0, "变量").includes("entity.name.function.sagemath"));
  assert.ok(scopesForText(grammar, source, 2, "参数").includes("variable.parameter.preparse.generator.sagemath"));
  assert.ok(scopesForText(grammar, source, 3, "结果").includes("variable.other.assignment.sagemath"));
  assert.ok(scopesForText(grammar, source, 3, "变量").includes("support.function.call.sagemath"));
  assert.ok(scopesForText(grammar, source, 4, "计数").includes("variable.other.typed.cython.sagemath"));
  assert.ok(scopesForText(grammar, source, 5, "变量").some((scope) => scope.startsWith("comment.")));
  assert.ok(scopesForText(grammar, source, 6, "变量").some((scope) => scope.startsWith("string.")));
});

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
