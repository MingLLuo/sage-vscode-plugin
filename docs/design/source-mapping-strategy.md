# Source Mapping

## Problem

`.sage` files are not Python: `x^2`, `a ^^ b`, `[1, 3, .., 11]`, `R.<x, y> = QQ[]`, and `f(x) = x^2` must be
understood by a Python parser, while every hover, jump, highlight, and diagnostic has to point at the user's original
text.

## Decision

Preprocessing is rewrite-based and line-preserving
(`crates/sage-index/src/source_analysis/preprocessing.rs`):

- Each rewrite produces Python on the same physical line and records a `PreprocessEdit` (line, source column,
  generated column, source text, generated text). Lines are never added or removed, so line numbers map one to one.
- `R.<x, y> = ...` becomes ordinary assignments of the parent and its generators; multi-line right-hand sides are
  collected first (`preparser_support.rs`) so a rewrite only happens on a complete statement.
- `f(x) = rhs` becomes `f = lambda x: rhs`, keeping `rhs` at its original columns.
- Nothing inside strings or comments is rewritten.

Analysis runs on the generated text, and every result position is translated back through the recorded edits of its
line. Diagnostics come from tree-sitter ERROR/MISSING nodes and are mapped the same way, so an error inside a rewritten
construct is shown on the user's own text.

## Rules for New Constructs

- Prefer a same-line rewrite with a recorded edit; never change the number of lines.
- When a construct cannot be rewritten safely (unfinished statement, unclear nesting), leave it unchanged. Navigation
  then stays conservative instead of jumping to a guessed target.
- Add a mapping test for positions before, inside, and after the rewritten text, plus a diagnostics test showing that
  valid syntax produces no error.
