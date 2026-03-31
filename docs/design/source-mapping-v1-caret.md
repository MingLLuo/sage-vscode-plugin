# Source Mapping v1: Caret Rewrite

## Status

Accepted

## Context

The repository needs a first concrete `.sage` preprocessing slice that is useful, testable, and small enough to land
without pretending to solve the full Sage preparser problem.

The most defensible first construct is the Sage exponent caret in ordinary code:

- it is common in `.sage`
- it diverges from Python semantics
- it can be rewritten line-preservingly
- it requires real position mapping once the generated text width changes

## Decision

Version 1 source mapping will:

- operate only on `.sage` documents
- rewrite standalone `^` tokens in code regions to `**`
- preserve line count exactly
- skip rewrites inside comments
- skip rewrites inside quoted strings, including triple-quoted strings
- skip ambiguous doubled carets such as `^^`

The first implementation will expose a document-level preprocessing result with:

- generated text
- a per-line original-to-generated column map
- a per-line generated-to-original column map
- helper methods for projecting positions in both directions

## Consequences

- Positive: the first mapping feature is real and testable.
- Positive: later diagnostics and hover behavior can build on the same mapping object.
- Negative: many Sage-only constructs remain untreated.
- Negative: this is still not a full preparser substitute.
- Deferred follow-up: add richer `.sage` constructs once the mapping container and test strategy prove stable.

