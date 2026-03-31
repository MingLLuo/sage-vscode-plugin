# Development Log: 2026-04-01

## Entry 1

- Date: 2026-04-01
- Task ID: LSP-018
- Scope: lsp
- Related milestone: Runtime hardening
- Commit: `fa0f257`

### Goal

Generalize native Sage documentation so common library symbols resolve useful hover content and source locations beyond
the small set already covered by runtime fallback.

### Decisions

- Decision: merge runtime signatures back into static documentation instead of treating runtime and static analysis as
  mutually exclusive.
- Reason: local source paths from static indexing are valuable, but runtime fallback often carries richer callable
  signatures.
- Decision: inherit docstrings from factory classes and `.pyx` function bodies during static indexing.
- Reason: local Sage source checkouts do not always provide reliable runtime introspection, especially for constructor
  factories and native Cython helpers.

### Verification

- Checks run:
  - `python -m pytest packages/sage-lsp/tests/test_parser.py packages/sage-lsp/tests/test_index.py packages/sage-lsp/tests/test_server.py packages/sage-lsp/tests/test_runtime_introspection.py`
  - direct native-source probe for `graphs.PetersenGraph`, `PolynomialRing`, `EllipticCurve`, `matrix`, `NumberField`, and `Partitions`
- Result: parser, index, server, and runtime-introspection tests passed; native-source probe returned documentation and
  source paths for the targeted library symbols

### Follow-ups

- Next task: QA-004
- Risks or blockers: real VS Code extension-host smoke still requires an approval path that allows local app launch

## Entry 2

- Date: 2026-04-01
- Task ID: QA-004
- Scope: repo/test
- Related milestone: Runtime hardening
- Commit: `685345f`

### Goal

Add a repeatable local smoke command that validates native Sage library support without relying on a GUI launch.

### Decisions

- Decision: ship a repository-local smoke script that exercises the same documentation and definition pipeline against a
  real Sage source checkout.
- Reason: this keeps native-library verification available even when local VS Code app launches are blocked or
  undesirable.

### Verification

- Checks run:
  - `npm run test:native-smoke`
  - `npm run test`
- Result: native Sage smoke passed for `graphs.PetersenGraph`, `PolynomialRing`, `EllipticCurve`, `matrix`,
  `NumberField`, and `Partitions`; repository unit and Python tests also passed

### Follow-ups

- Next task: revisit extension-host native-library smoke once GUI launch approval is available
- Risks or blockers: the non-GUI smoke validates the analysis pipeline, not final command-palette or hover-popup UI

## Entry 3

- Date: 2026-04-01
- Task ID: LSP-019
- Scope: lsp
- Related milestone: Runtime hardening
- Commit: `b1c61c1`

### Goal

Reduce the first-hover cost for common Sage call targets by preloading documentation when a file is opened instead of
waiting for the first pointer hover to trigger cold lookup work.

### Decisions

- Decision: prewarm only on document open, not on every text change.
- Reason: this preserves the first-hover latency improvement without turning normal typing into repeated runtime
  introspection work.
- Decision: cap prewarming to a small set of likely callable targets discovered from the current document.
- Reason: warming a bounded set of high-value callables keeps startup cost predictable.

### Verification

- Checks run:
  - `python -m pytest packages/sage-lsp/tests/test_server.py`
  - `npm run test`
- Result: request-level tests now cover prewarm population and hover cache reuse; repository tests remained green

### Follow-ups

- Next task: observe whether the candidate cap should become configurable after more real-world Sage files are tested
- Risks or blockers: very large notebooks or generated files may still contain more useful call targets than the
  current prewarm budget

## Entry 4

- Date: 2026-04-01
- Task ID: SYN-004
- Scope: syntax
- Related milestone: Native source support
- Commit: `a287365`

### Goal

Make Sage highlighting feel less flat by separating the major mathematical domains into richer grammar scopes instead
of coloring almost everything as one generic support token.

### Decisions

- Decision: split Sage highlighting into domain-oriented scopes for rings and fields, constructors, symbolic work,
  plotting, graph theory, combinatorics, crypto, number theory, and linear algebra.
- Reason: richer scopes give themes more room to style Sage code intentionally instead of collapsing most APIs into one
  color.

### Verification

- Checks run:
  - `npm run sync:syntax`
  - `npm run test --workspace @sage-vscode/extension-core`
  - `npm run test`
- Result: syntax assets synced successfully and both extension-only plus repository-wide tests passed after the
  generated grammar was updated

### Follow-ups

- Next task: consider semantic tokens on top of the richer grammar if TextMate scopes alone still look too conservative
- Risks or blockers: final color separation still depends partly on the active VS Code theme

## Entry 5

- Date: 2026-04-01
- Task ID: EXT-011
- Scope: extension
- Related milestone: Developer workflow
- Commit: `pending`

### Goal

Reduce interpreter-selection complexity by promoting complete Sage environment profiles instead of making users choose a
runtime path and a language-server Python host separately.

### Decisions

- Decision: make `Sage: Select Interpreter` environment-first and surface `Local Sage development environment` plus
  `System Sage (stable)` before any advanced custom-path actions.
- Reason: the local workflow actually depends on pairing a nearby `sage` checkout with `conda` `sage-dev`, while the
  installed stable runtime is a separate path; showing those directly is clearer than exposing the underlying split as
  the first user-facing concept.
- Decision: selecting an environment profile updates both `sage.interpreter.path` and
  `sage.languageServer.pythonPath` together.
- Reason: users should not need to manually coordinate two settings for the common paths that the extension can
  already infer reliably.

### Verification

- Checks run:
  - `npm run test --workspace @sage-vscode/extension-core`
  - `npm run test`
- Result: the extension test suite now covers local-checkout plus `sage-dev` profile detection and the full repository
  test path remained green

### Follow-ups

- Next task: keep refining the runtime-side behavior of the detected local development profile as more real Sage
  checkout environments are tested
- Risks or blockers: uncommon Sage layouts may still need the advanced custom-path entries

## Entry 6

- Date: 2026-04-01
- Task ID: LSP-020
- Scope: lsp
- Related milestone: Runtime hardening
- Commit: `pending`

### Goal

Lower first-jump latency and make local-checkout runtime fallback more resilient by warming definition results and
feeding the runtime probe enough import roots to see source plus compiled native modules.

### Decisions

- Decision: extend document-open prewarm from documentation only to both documentation and definition caches.
- Reason: the first `Go to Definition` on common call targets should not still pay the full cold-resolution cost after
  hover prewarm work has already identified the same callables.
- Decision: pass the resolved language-server Python path into the server environment and expand runtime `PYTHONPATH`
  with discovered `sage/src` and sibling `builddir*/src` roots.
- Reason: checkout-based Sage development relies on source plus compiled native artifacts rather than only an installed
  stable `sage` executable.

### Verification

- Checks run:
  - `python -m pytest packages/sage-lsp/tests/test_runtime_introspection.py packages/sage-lsp/tests/test_server.py`
  - `npm run test`
  - `npm run test:native-smoke`
- Result: runtime-introspection and server tests passed, repository tests stayed green, and the native smoke suite kept
  resolving common Sage library symbols against the local checkout

### Follow-ups

- Next task: continue probing whether local `sage-dev` runtime imports can be made fully live for more dynamic symbols
  without relying on the stable installed Sage executable
- Risks or blockers: some editable or partially configured local Sage development environments may still need extra
  shell activation context beyond what the extension can infer automatically

## Entry 7

- Date: 2026-04-01
- Task ID: SYN-005
- Scope: syntax
- Related milestone: Native source support
- Commit: `pending`

### Goal

Make the editor-side Sage highlighting feel less generic for structure-heavy source files by surfacing common runtime
helpers, decorators, namespaces, and factory patterns in addition to broad mathematical domains.

### Decisions

- Decision: treat structure-heavy Sage forms such as `@cached_method`, `lazy_import`, `UniqueFactory`, and
  `toric_varieties` as dedicated highlighting signals.
- Reason: these are frequent visual anchors in real Sage sources and deserve stronger differentiation than a single
  catch-all support scope.
- Decision: add a dedicated manual smoke file for highlighting validation instead of relying only on automated grammar
  assertions.
- Reason: automated tests can confirm the scope patterns exist, but the final usefulness still depends on how a real VS
  Code theme renders them.

### Verification

- Checks run:
  - `npm run sync:syntax`
  - `npm run test --workspace @sage-vscode/extension-core`
  - `npm run test`
- Result: synced syntax assets, extension tests, and full repository tests all passed after the richer grammar and
  snippet set were updated

### Follow-ups

- Next task: add semantic tokens on top of the richer TextMate scopes when theme-independent structure highlighting
  becomes the next priority
- Risks or blockers: some themes still collapse custom TextMate scopes into similar colors, so semantic tokens remain
  the stronger long-term path

## Entry 8

- Date: 2026-04-01
- Task ID: LSP-021
- Scope: lsp
- Related milestone: LSP baseline
- Commit: `pending`

### Goal

Make complex Python-style `.sage` files behave closer to normal Python-editor expectations by preserving AST-grade
analysis where possible, reducing repeated lookup cost, and adding a first semantic-token layer on top of the richer
TextMate grammar.

### Decisions

- Decision: treat `.sage` files without preparser assignment syntax as Python-like and parse them through the Python AST
  path after lightweight Sage preprocessing.
- Reason: many real Sage files are mostly Python with a `.sage` suffix, so dropping them to the loose line-based parser
  throws away class, method, and import structure that should remain available.
- Decision: keep preparser-assignment forms such as `R.<x, y> = ...` on the existing loose parser for now.
- Reason: the current source mapping is still too narrow to remap those lines safely through a full AST path without
  risking incorrect ranges.
- Decision: index saved `.sage` and `.pxi` files during workspace builds and add caches for indexed symbol and member
  resolution.
- Reason: this reduces the gap between open-document analysis and workspace-wide navigation while avoiding repeated
  recursive resolution work for stable indexed modules.
- Decision: add a semantic-token baseline now instead of waiting for a larger highlighting overhaul.
- Reason: semantic tokens immediately improve theme-independent differentiation for methods, namespaces, constructors,
  readonly Sage values, decorators, and preparser generator declarations.

### Verification

- Checks run:
  - `python -m py_compile packages/sage-lsp/src/sage_lsp/parser.py packages/sage-lsp/src/sage_lsp/index.py packages/sage-lsp/src/sage_lsp/server.py`
  - `python -m pytest packages/sage-lsp/tests/test_parser.py packages/sage-lsp/tests/test_index.py packages/sage-lsp/tests/test_server.py -q`
  - `npm run test`
  - `npm run test:native-smoke`
  - `npm run test:extension-host`
- Result: parser, index, server, extension, native-smoke, and real VS Code extension-host smoke all passed after the
  `.sage` fast path, semantic-token baseline, and indexed lookup caches were added

### Follow-ups

- Next task: persist the Sage library/workspace index so restart cost drops on large trees
- Risks or blockers: preparser-heavy `.sage` files still fall back to the loose parser until source mapping grows past
  caret rewrite and can safely support more transformed constructs
