import test from "node:test";
import assert from "node:assert/strict";

import {
  definitionTargetFromQuery,
  isLspLocationPayload,
  referenceQuickPickLabel,
  sourceRangeFromLspLocation,
  sourceRangeFromUnknown,
} from "../src/sageNavigation";
import type { QueryResponse } from "../src/uxSelfCheck";

test("definitionTargetFromQuery returns existing high confidence Sage source targets", () => {
  const query: QueryResponse = {
    definition: {
      name: "PetersenGraph",
      module: "sage.graphs.generators.smallgraphs",
      path: "/repo/sage/src/sage/graphs/generators/smallgraphs.py",
      range: {
        start_line: 4602,
        start_character: 4,
        end_line: 4602,
        end_character: 17,
      },
    },
    resolutionConfidence: "high",
    resolutionReason: "resolved through materialized sage.all export cache",
  };

  assert.deepEqual(
    definitionTargetFromQuery(query, (candidate) => candidate.startsWith("/repo/sage/src")),
    {
      path: "/repo/sage/src/sage/graphs/generators/smallgraphs.py",
      range: {
        start_line: 4602,
        start_character: 4,
        end_line: 4602,
        end_character: 17,
      },
      confidence: "high",
      reason: "resolved through materialized sage.all export cache",
    },
  );
});

test("definitionTargetFromQuery suppresses missing installed Sage package targets", () => {
  const query: QueryResponse = {
    definition: {
      name: "PetersenGraph",
      module: "sage.graphs.generators.smallgraphs",
      path: "/Applications/SageMath-10-8.app/Contents/Frameworks/Sage.framework/Versions/10.8/local/lib/python3.13/site-packages/sage/graphs/generators/smallgraphs.py",
    },
    resolutionConfidence: "high",
  };

  assert.equal(definitionTargetFromQuery(query, () => false), null);
});

test("definitionTargetFromQuery suppresses low confidence ambiguous method targets", () => {
  const query: QueryResponse = {
    definition: {
      name: "rank",
      module: "sage.misc",
      path: "/repo/sage/src/sage/misc/functional.py",
    },
    resolutionConfidence: "low",
  };

  assert.equal(definitionTargetFromQuery(query, () => true), null);
});

test("sourceRangeFromUnknown validates Rust query range payloads", () => {
  assert.deepEqual(
    sourceRangeFromUnknown({
      start_line: 1,
      start_character: 2,
      end_line: 3,
      end_character: 4,
    }),
    {
      start_line: 1,
      start_character: 2,
      end_line: 3,
      end_character: 4,
    },
  );
  assert.equal(sourceRangeFromUnknown({ start_line: 1 }), undefined);
});

test("isLspLocationPayload validates reference payload shape", () => {
  const payload = {
    uri: "file:///workspace/example.sage",
    range: {
      start: { line: 2, character: 4 },
      end: { line: 2, character: 11 },
    },
  };

  assert.equal(isLspLocationPayload(payload), true);
  assert.equal(isLspLocationPayload({ ...payload, uri: 12 }), false);
  assert.equal(isLspLocationPayload({ ...payload, range: { start: { line: 2 } } }), false);
});

test("sourceRangeFromLspLocation normalizes LSP ranges for reference display", () => {
  assert.deepEqual(
    sourceRangeFromLspLocation({
      uri: "file:///workspace/example.sage",
      range: {
        start: { line: 2, character: 4 },
        end: { line: 2, character: 11 },
      },
    }),
    {
      start_line: 2,
      start_character: 4,
      end_line: 2,
      end_character: 11,
    },
  );
});

test("referenceQuickPickLabel includes path, position, and uri detail", () => {
  assert.deepEqual(
    referenceQuickPickLabel(
      "file:///workspace/src/example.sage",
      {
        start_line: 9,
        start_character: 2,
        end_line: 9,
        end_character: 8,
      },
      () => "src/example.sage",
    ),
    {
      label: "src/example.sage:10:3",
      description: "10:3",
      detail: "file:///workspace/src/example.sage",
    },
  );
});
