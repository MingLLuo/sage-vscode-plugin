# Complex Call Query Analysis

Date: 2026-05-03

## Scope

This note records the current behavior of Sage call-site queries under complex `.sage` code. The goal is to validate
signature and navigation behavior at real call sites, not only at symbol definitions or first textual matches.

## Tested Patterns

- Keyword call site:
  `trace_window(w^2 + 3*w + 1, width=7)`
- Multi-line call:
  `trace_window(` followed by positional and keyword arguments on later lines.
- Nested tuple/list/group arguments:
  `R.quotient(I, names=("xb", "yb", "zb"))`
- Advanced Sage fixture:
  `examples/manual-smoke-workspace/src/09_advanced_sage_patterns.sage`
- Browser/debug matrix scenario:
  `advanced-keyword-call-signature`

## Findings

- Previous coverage mostly queried by symbol name. That is useful for hover/definition, but it can accidentally test the
  definition or first occurrence instead of a real call site.
- Call-site signature help needed a shared parser for nested parentheses and active-parameter counting. The old logic
  was line-local and counted commas naively.
- The Rust index and Rust LSP carried duplicate call-context parsing logic, which increased maintenance risk.

## Changes Made

- `crates/sage-index` now exposes a shared `function_call_at_position(text, line, character)` helper.
- The helper scans source up to the cursor, ignores strings/comments through `CodeMap`, handles multi-line calls, and
  ignores commas inside nested tuple/list/group frames.
- `crates/sage-ls` now reuses the shared helper instead of keeping its own duplicate implementation.
- `scripts/debug-workbench.mjs` now supports UX scenarios addressed by `line`/`character`, not only by symbol name.
- The workbench matrix now checks the real keyword argument position inside
  `trace_window(w^2 + 3*w + 1, width=7)` and verifies:
  - no diagnostics,
  - signature label `trace_window(poly, base_ring=QQ, *, width=5, normalize=True)`,
  - active parameter `1`,
  - warm position query under the existing 1000ms budget.

## Measured Result

A direct debug-inspector query at `09_advanced_sage_patterns.sage:69:35` returned:

- elapsed time: about `45ms` on the local warm cache,
- signature: `trace_window(poly, base_ring=QQ, *, width=5, normalize=True)`,
- active parameter: `1`,
- diagnostics: `0`.

## Remaining Optimization Space

- Query surfaces still report `fallback_reason = symbol-not-in-index-or-known-sage-set` when the cursor is on a keyword
  argument such as `width`. This is technically correct for hover/definition on the argument symbol, but noisy for
  signature-help-only diagnostics. A future improvement could separate `signature_context` from `symbol_context` in the
  debug payload.
- Method-chain signatures such as `.right_kernel().dimension()` are still limited by static type inference. The current
  coverage confirms highlighting and method query latency, but richer method signatures would need a stronger Sage type
  model or runtime-assisted member introspection.
- The call-context scanner is linear in the source length before the cursor. That is acceptable for current smoke files
  and measured warm queries, but very large generated `.sage` files could benefit from cached line/paren state if
  signature-help latency becomes visible.
