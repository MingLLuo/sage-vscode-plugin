# LSP Baseline Features

## Purpose

This note records the maintained production baseline for the Rust language server. The legacy Python server remains a
compatibility oracle, but it is not the production analysis authority.

## Editor Baseline

The maintained baseline includes:

- hover and static/runtime documentation
- context-aware completion and completion resolution
- definition, declaration, type definition, and implementation
- ordered navigation candidates through `LocationLink`
- references, prepare rename, and rename
- signature help
- semantic tokens and range semantic tokens
- document and workspace symbols
- document highlights, links, folding, selection ranges, and inlay hints
- call hierarchy
- conservative diagnostics and deterministic quick fixes
- Sage `load`/`attach`, Cython include/import, `.pyx`, `.pxd`, `.pxi`, and `.spyx` navigation

## Navigation Contract

Navigation is confidence-aware:

- `high`: one identity-checked target may be returned as a direct jump.
- `ambiguous`: no target is forced; the client receives ordered candidates and an explanation.
- `weak` or `none`: evidence may enrich hover or completion, but it cannot authorize references, rename, call
  hierarchy, or a direct jump.

Type ownership must come from visible imports, explicit constructors, verified assignment flow, preparser bindings, or
another identity-preserving source. A member name or variable-name resemblance alone is never sufficient for a
high-confidence jump. Coarse catalog owners that span multiple implementation families remain candidate-only.

Method return inference uses both the receiver type and method name. For example, `change_ring()` preserves a proven
Matrix or Vector owner, `GF(...).gen()` yields a finite-field element, and a NumberField generator yields a distinct
number-field element. Ordinary polynomial-ring parents are split into proven univariate and multivariate owners;
element return types remain conservative where the concrete implementation is not proved. Tuple-valued and other
variant-dependent calls remain unknown.

## Sage Source Support

- `.sage` preprocessing preserves original source positions while recognizing caret exponents, ranges, empty ring
  brackets, and `R.<x, y> = ...` generator assignments.
- Preparser parent and generator bindings participate in strict type flow and local-function return inference.
- Python, Sage, and Cython imports support single-line and parenthesized forms, including multiline `cimport`.
- `sage.all`, lazy imports, star re-exports, and source-derived method catalogs are materialized from the configured
  Sage checkout.
- Known Sage owners include matrices, vectors, free modules, polynomial rings/elements/ideals, finite fields/elements,
  graphs, elliptic curves, number fields/elements, and polyhedra.
- Polyhedron navigation follows the shared `base0` through `base7` hierarchy while excluding faces,
  representations, parents, and backend-specific classes from the generic instance cache.

## Index and Runtime Baseline

- SQLite caches are namespaced by cache format, source roots, excludes, and parser options.
- Cache-format changes invalidate old materialized semantics rather than silently reusing stale method targets.
- Startup hydrates a valid cache first, then reconciles source fingerprints and editable files in the background.
- Open documents override disk state and preserve their client URI across canonical path aliases.
- Static analysis works without launching Sage. Optional runtime introspection enriches documentation and reports a
  clear degraded state when unavailable.
- Blocking scans, parsing, SQLite work, and linked-document prewarming stay off LSP request workers.

## Verification Baseline

Changes to this surface should keep the following gates green:

- Rust unit tests and strict Clippy
- extension, syntax-pack, and legacy Python compatibility tests
- LSP navigation and shutdown protocol smokes
- debug-workbench interaction matrix
- real-file smoke tests against the locally discovered Sage checkout
- release LSP latency and cold index performance budgets
- repository hygiene and product-readiness checks

Real-file coverage must include both positive and negative navigation cases. A passing timing-only query is not evidence
that its owner, confidence, and source path are correct.

## Follow-up Areas

- Complete element and constructor variants where Sage behavior differs: univariate versus multivariate polynomial
  elements, Laurent/power-series/Boolean rings, Graph versus DiGraph, absolute versus relative number fields, and
  field-specific elliptic curves.
- Expand common constructors without collapsing distinct Sage domains into a broad owner.
- Continue splitting large test and completion modules when responsibilities can move without weakening behavioral
  coverage.
- Expand semantic classification and diagnostics only where the result remains conservative and source-mapped.
