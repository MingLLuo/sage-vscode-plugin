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
- Commit: `edb8aaf`

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
- Commit: `d054bd5`

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
- Commit: `f8a6f8d`

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
- Commit: `b010904`

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

## Entry 9

- Date: 2026-04-01
- Task ID: LSP-022
- Scope: lsp
- Related milestone: LSP baseline
- Commit: `8911b0f`

### Goal

Recover AST-level structure for `.sage` files that mix Sage preparser assignment syntax with otherwise ordinary Python
class, method, and import code.

### Decisions

- Decision: keep preparser declarations from the loose parser, but parse a sanitized version of the same file through
  the Python AST path and merge the two records.
- Reason: preparser lines such as `pring.<x> = QQ[]` are still not safe to map fully through the AST path, but they
  should not force the rest of the file to lose methods, instance tracking, and member completion.
- Decision: sanitize preparser assignment lines down to a valid placeholder assignment during AST parsing.
- Reason: this makes heavily Sage-flavored top-level declarations legal Python for structural parsing without needing a
  full preparser implementation first.

### Verification

- Checks run:
  - `python -m pytest packages/sage-lsp/tests/test_parser.py packages/sage-lsp/tests/test_index.py -q`
  - `npm run test`
  - `npm run test:native-smoke`
  - `npm run test:extension-host`
- Result: targeted parser/index tests, repository tests, native Sage smoke, and real VS Code extension-host smoke all
  passed after the hybrid `.sage` parse path was introduced

### Follow-ups

- Next task: broaden sanitized/preprocessed parsing to more Sage preparser forms without sacrificing range accuracy
- Risks or blockers: preparser lines themselves still rely on the loose parser for symbol ranges and do not yet expose
  a full transformed-AST mapping model

## Entry 10

- Date: 2026-04-01
- Task ID: LSP-023
- Scope: lsp
- Related milestone: LSP baseline
- Commit: `a1ae742`

### Goal

Reduce rebuild cost for large Sage source trees by reusing parsed module records across server restarts instead of
re-parsing every indexed file from scratch.

### Decisions

- Decision: persist serialized `ModuleRecord` data per indexed file and reuse it when the file's size and nanosecond
  mtime are unchanged.
- Reason: this keeps the first cache layer simple, deterministic, and cheap to validate while still cutting out the
  parse step on warm rebuilds.
- Decision: keep source text out of the persistent payload and continue reading files on rebuild.
- Reason: the current symbol/reference paths still rely on live source text, and avoiding stored source keeps the
  cache smaller and simpler.
- Decision: treat persistence as best-effort and fall back to a temporary directory or no persistence if the preferred
  cache location cannot be written.
- Reason: the cache should never become a new startup failure mode.

### Verification

- Checks run:
  - `python -m pytest packages/sage-lsp/tests/test_index.py -q`
  - `python -m pytest packages/sage-lsp/tests/test_index.py packages/sage-lsp/tests/test_server.py -q`
  - `npm run test`
  - `npm run test:native-smoke`
  - `npm run test:extension-host`
- Result: persistent-cache reuse and invalidation tests passed, repository tests stayed green, native Sage smoke passed,
  and the real VS Code extension-host smoke still passed after the cache layer was added

### Follow-ups

- Next task: split hot document state from cached library/workspace state and start moving toward incremental rebuilds
- Risks or blockers: this cache still re-reads source files and still rebuilds the module list eagerly; it removes
  parse cost first, not the entire indexing cost

## Entry 11

- Date: 2026-04-01
- Task ID: LSP-024
- Scope: lsp
- Related milestone: LSP baseline
- Commit: `48decf3`

### Goal

Stop re-parsing the same open document on every hover, definition, completion, and symbol request when the editor
buffer has not changed.

### Decisions

- Decision: add an open-document overlay cache keyed by document URI, language id, and exact source text.
- Reason: modern language tools typically treat open buffers as hot state above the colder workspace index, and the
  same unchanged editor text should not be reparsed for every request.
- Decision: populate the overlay on open/change and drop it on close.
- Reason: this keeps live editor state synchronized with the buffer lifecycle and avoids retaining stale document
  records longer than needed.
- Decision: publish an empty diagnostics set on close.
- Reason: once the editor buffer is gone, the server should stop surfacing stale live-buffer diagnostics for it.

### Verification

- Checks run:
  - `python -m pytest packages/sage-lsp/tests/test_index.py packages/sage-lsp/tests/test_server.py -q`
  - `npm run test`
  - `npm run test:native-smoke`
  - `npm run test:extension-host`
- Result: overlay-cache reuse and invalidation tests passed, full repository tests remained green, native Sage smoke
  passed, and the real VS Code extension-host smoke also passed after the hot-document layer was added

### Follow-ups

- Next task: make workspace rebuilds more incremental so background file changes do not require a full cold rebuild
- Risks or blockers: the current overlay cache keys on full source equality, so memory use still scales with open
  document size until a more compact versioned buffer model is introduced

## Entry 12

- Date: 2026-04-01
- Task ID: LSP-025
- Scope: lsp
- Related milestone: LSP baseline
- Commit: `b91355a`

### Goal

Improve readability and maintainability of the recent indexing and document-cache code without changing behavior.

### Decisions

- Decision: extract smaller helper methods for resetting runtime state, iterating indexable modules, loading or parsing
  module records, storing parsed records, and handling open-document overlays.
- Reason: the recent warm-cache and hot-overlay features had made `WorkspaceIndex.build()` and `parse_document()`
  harder to read than they needed to be.
- Decision: keep the refactor behavior-preserving and verify it with the existing test suite instead of changing the
  indexing model again in the same step.
- Reason: readability work is only useful if it reduces future change risk without creating fresh regressions.

### Verification

- Checks run:
  - `python -m pytest packages/sage-lsp/tests/test_index.py packages/sage-lsp/tests/test_server.py -q`
  - `npm run test`
- Result: targeted index/server tests and the full repository test suite passed after the helper extraction refactor

### Follow-ups

- Next task: continue replacing repeated ad hoc indexing logic with clearer hot-document versus cold-index boundaries
- Risks or blockers: `server.py` still carries a lot of feature wiring and could use a similar readability pass later

## Entry 13

- Date: 2026-04-01
- Task ID: LSP-026
- Scope: lsp
- Related milestone: LSP baseline
- Commit: `5b8e14e`

### Goal

Improve readability in `server.py` by extracting the repeated request/document flow into smaller helpers without
changing request behavior.

### Decisions

- Decision: add dedicated helpers for resolved-request documentation/definition lookup, hover markup construction,
  document-overlay refresh, and empty diagnostics publication.
- Reason: request handlers had started repeating the same glue logic across hover, definition, open/change/close, and
  diagnostics code paths.
- Decision: keep the refactor localized to handler wiring instead of changing the indexing model in the same step.
- Reason: the recent cache work already shifted the performance model, so this pass should stay focused on readability
  and maintenance risk reduction.

### Verification

- Checks run:
  - `python -m pytest packages/sage-lsp/tests/test_server.py packages/sage-lsp/tests/test_index.py -q`
  - `npm run test`
- Result: targeted server/index tests and the full repository suite both passed after the `server.py` helper
  extraction refactor

### Follow-ups

- Next task: continue shrinking handler-level branching as the server moves toward clearer hot-document versus
  cold-index boundaries
- Risks or blockers: request wiring is cleaner now, but future performance work will still need careful discipline to
  avoid pushing cache and runtime policy back into individual handlers

## Entry 14

- Date: 2026-04-01
- Task ID: RUNTIME-004
- Scope: runtime
- Related milestone: Runtime hardening
- Commit: `3beb130`

### Goal

Stop treating saved and watched workspace files as rebuild-only events by refreshing indexed modules incrementally and
keeping dependent request results coherent afterward.

### Decisions

- Decision: keep per-path component records inside `WorkspaceIndex`, then rebuild the merged module view for only the
  affected module when one saved or watched file changes.
- Reason: mixed native modules such as `.pyx` plus `.pxd` cannot be updated safely if the index only remembers the
  final merged record.
- Decision: handle both `textDocument/didSave` and `workspace/didChangeWatchedFiles`, and include `.py` in extension
  file watchers.
- Reason: users edit Python helper modules alongside `.sage` files, and closed-file changes should refresh the index
  too.
- Decision: clear request caches globally after saved or watched file changes.
- Reason: imported-symbol hover and definition results can stay stale in unrelated open `.sage` files even when the
  underlying module record has already refreshed.

### Verification

- Checks run:
  - `python -m pytest packages/sage-lsp/tests/test_server.py packages/sage-lsp/tests/test_index.py -q`
  - `npm run test`
  - `npm run test:native-smoke`
- Result: incremental refresh/remove tests passed, the full repository suite stayed green, and native Sage smoke still
  resolved common library symbols correctly

### Follow-ups

- Next task: keep pushing toward background incremental indexing so source-root scans are no longer the default answer
  to larger workspace changes
- Risks or blockers: request-cache invalidation is intentionally broad for correctness right now, so future tuning can
  still recover some cross-file cache hit rate

## Entry 15

- Date: 2026-04-01
- Task ID: QA-005
- Scope: extension/test
- Related milestone: Runtime hardening
- Commit: `5fb021a`

### Goal

Prove through the real VS Code host that saving an imported Python helper updates hover results in a `.sage`
consumer without requiring a manual restart.

### Decisions

- Decision: extend the extension-host smoke suite to edit and save `local_docs.py`, then re-query hover inside
  `01_hover_and_definition.sage`.
- Reason: this is a realistic user path for complex Sage projects that mix `.sage` notebooks with ordinary Python
  support modules.
- Decision: make the smoke mutation idempotent so `assertEventually` retries do not fail after the first successful
  file rewrite.
- Reason: smoke assertions should surface product regressions, not self-inflicted retry flakiness.

### Verification

- Checks run:
  - `npm run test:extension-host`
- Result: the real extension-host smoke suite passed after the imported-helper save path refreshed hover results inside
  the `.sage` consumer

### Follow-ups

- Next task: add another end-to-end workspace-change assertion for closed-file watcher updates once the incremental
  index grows beyond save-driven edits
- Risks or blockers: this smoke currently covers the save path, while closed-file watcher propagation remains covered
  by server-side tests rather than the real extension host

## Entry 16

- Date: 2026-04-01
- Task ID: RUNTIME-005
- Scope: runtime
- Related milestone: Runtime hardening
- Commit: `b31b01d`

### Goal

Reduce the overhead of multi-file watcher and save bursts by applying incremental index changes in batches instead of
persisting cache state and clearing resolution caches once per file.

### Decisions

- Decision: add batched `refresh_paths`, `remove_paths`, and `refresh_or_remove_paths` flows inside `WorkspaceIndex`.
- Reason: the single-file incremental API was correct, but a burst of watcher events still paid repeated cache-write
  and resolution-cache-reset costs.
- Decision: route `workspace/didChangeWatchedFiles` through one batched server helper and pin the behavior with tests.
- Reason: the language server already receives batched file events from the client, so the index layer should preserve
  that granularity instead of flattening it back into per-file work.

### Verification

- Checks run:
  - `python -m pytest packages/sage-lsp/tests/test_index.py packages/sage-lsp/tests/test_server.py -q`
  - `npm run test`
  - `npm run test:native-smoke`
  - `npm run test:extension-host`
- Result: targeted index/server tests, full repository tests, native Sage smoke, and real extension-host smoke all
  passed after the batched incremental-update refactor

### Follow-ups

- Next task: start separating hot-document edits from colder background source-root scans so larger workspaces do even
  less synchronous filesystem work on initialization
- Risks or blockers: batching reduces repeated work inside one event burst, but source-root discovery and cold startup
  still walk the full tree when no cache exists yet

## Entry 17

- Date: 2026-04-01
- Task ID: RUNTIME-006
- Scope: runtime
- Related milestone: Runtime hardening
- Commit: `b1c02ab`

### Goal

Reduce warm-start cold-path I/O by reusing cached module source for unchanged files instead of rereading every module
source file from disk before rebuilding the in-memory index.

### Decisions

- Decision: store module source alongside each persistent cache entry and reuse it when the cached fingerprint still
  matches the on-disk file.
- Reason: the previous warm cache avoided reparsing, but it still reread every unchanged source file just to
  deserialize the same record again.
- Decision: bump the cache schema and add direct coverage that unchanged warm-cache startup no longer reads module
  source files.
- Reason: this change affects cache compatibility and should be pinned with a targeted regression test instead of
  relying only on parse-level assertions.

### Verification

- Checks run:
  - `python -m pytest packages/sage-lsp/tests/test_index.py -q`
  - `python -m pytest packages/sage-lsp/tests/test_server.py -q`
  - `npm run test`
  - `npm run test:native-smoke`
  - `npm run test:extension-host`
- Result: index tests, server tests, full repository tests, native Sage smoke, and real extension-host smoke all
  passed after warm-cache startup switched to cached-source reuse

### Follow-ups

- Next task: keep pushing cold startup toward a true “cached snapshot first, background reconcile later” model
- Risks or blockers: unchanged-module source reads are gone on warm starts, but the server still walks source roots to
  compute fingerprints and detect new files before initialization completes

## Entry 18

- Date: 2026-04-01
- Task ID: LSP-027
- Scope: lsp
- Related milestone: Source mapping v1
- Commit: `0b2d48b`

### Goal

Project `.sage` syntax diagnostics back onto original source ranges so generated validation text never leaks into the
editor as incorrect caret columns or highlight spans.

### Decisions

- Decision: extend the source-mapping layer with generated-range projection helpers instead of patching syntax-error
  spans ad hoc inside one parser branch.
- Reason: future `.sage` diagnostics and navigation work will need the same projection primitives, so the mapping
  model should own range normalization.
- Decision: thread the preprocessed `.sage` document through syntax validation helpers and mark parser-produced syntax
  diagnostics with an explicit `syntax-error` code.
- Reason: the server now needs to preserve projected ranges during publish and later code-action filtering without
  guessing which diagnostics came from syntax validation.

### Verification

- Checks run:
  - `python -m pytest packages/sage-lsp/tests/test_source_map.py packages/sage-lsp/tests/test_parser.py packages/sage-lsp/tests/test_server.py -q`
- Result: source-map projection tests, parser diagnostics tests, and publish-level server tests passed with `.sage`
  syntax errors highlighting the original source caret position

### Follow-ups

- Next task: keep feeding the same projection model into more `.sage` navigation and edit paths beyond syntax
  diagnostics
- Risks or blockers: current projection work covers syntax diagnostics first; richer `.sage` rename/reference paths
  still need deeper source-map adoption

## Entry 19

- Date: 2026-04-01
- Task ID: LSP-028
- Scope: lsp
- Related milestone: LSP baseline
- Commit: `dedcf16`

### Goal

Expose standard quick-fix code actions for deterministic unresolved-import-name diagnostics and prove the behavior
through both request-level and real VS Code host automation.

### Decisions

- Decision: only generate quick fixes for `unresolved-import-name`, not `unresolved-import-module`.
- Reason: the index can safely rewrite an existing-but-wrong import source module, while missing modules are too
  ambiguous for a low-noise automatic fix.
- Decision: reuse indexed symbol-export knowledge to rank import candidates, preferring the defining module ahead of
  broad re-export surfaces such as `sage.all`.
- Reason: quick fixes should bias toward the most direct source of truth rather than the widest aggregator.
- Decision: extend the extension-host smoke suite with a temporary `.sage` file that exercises both projected syntax
  diagnostics and a real import quick-fix application.
- Reason: this behavior matters at the editor integration layer, not just in handler-level unit tests.

### Verification

- Checks run:
  - `npm run lint`
  - `npm run test`
  - `npm run test:native-smoke`
  - `npm run test:extension-host`
- Result: repository lint, unit/integration tests, native Sage smoke, and real extension-host smoke all passed after
  the server started publishing quick-fix metadata and the host smoke applied the generated import rewrite

### Follow-ups

- Next task: consider extending quick fixes to other deterministic Sage import and binding diagnostics once false
  positives stay under control
- Risks or blockers: quick-fix ranking is intentionally conservative today and still depends on static export
  knowledge rather than deeper runtime category inference

## Entry 20

- Date: 2026-04-01
- Task ID: RUNTIME-007
- Scope: runtime
- Related milestone: Runtime hardening
- Commit: `de81ce7`

### Goal

Reduce the perceived latency before definition and hover become useful on warm starts by hydrating cached index
snapshots first and pushing the full source-root rebuild into a background reconcile pass.

### Decisions

- Decision: add an explicit `hydrate_from_cache()` path on `WorkspaceIndex` that restores cached module records
  without rereading or reparsing module source files.
- Reason: the previous warm build still walked the source tree synchronously and rebuilt the in-memory graph before
  the server could answer requests, which dominated the “go to definition” wait on large local Sage checkouts.
- Decision: have server initialization prefer the hydrated snapshot when present, then launch a background rebuild that
  swaps in a fresh index only if the server generation still matches.
- Reason: this keeps request-serving state usable earlier without letting stale background work overwrite a newer
  rebuild request.
- Decision: stop clearing resolution caches on every module inserted during full builds and snapshot restores.
- Reason: those cache resets were redundant during bulk population and added avoidable startup overhead.

### Verification

- Checks run:
  - `python -m pytest packages/sage-lsp/tests/test_index.py packages/sage-lsp/tests/test_server.py -q`
  - `npm run test`
  - `npm run test:native-smoke`
  - `npm run test:extension-host`
  - local timing probe against `/workspace/sage/src`
- Result: targeted tests, full repository tests, native Sage smoke, and real extension-host smoke all passed; the
  local timing probe measured warm full build at about `2.67s` and direct snapshot hydration at about `1.35s` for the
  local Sage source tree

### Follow-ups

- Next task: keep pushing startup toward true lazy or background source-root discovery for first-run cases where no
  snapshot exists yet
- Risks or blockers: warm starts improve, but cold starts without a usable cache still pay the full initial source
  walk before requests become available

## Entry 21

- Date: 2026-04-01
- Task ID: RUNTIME-008
- Scope: runtime
- Related milestone: Runtime hardening
- Commit: `768b772`

### Goal

Keep warm-start snapshots authoritative while still deferring expensive Sage-root traversal, so the startup path does
not poison its own persistent cache with partial module sets.

### Decisions

- Decision: treat cache persistence as valid only for complete snapshots, not for opportunistic bootstrap or lazy
  module loads.
- Reason: deferred eager/lazy loading was writing incomplete `_cache_entries` back to disk, which caused later warm
  starts to restore only a tiny fraction of the real Sage module graph.
- Decision: keep partial eager-root refresh and on-demand module loading strictly in-memory unless the server has a
  full snapshot to persist.
- Reason: this preserves the fast startup path without turning a local optimization into long-lived cache corruption.
- Decision: update startup to refresh smaller roots eagerly, keep large Sage roots deferred, and only persist on
  startup when the loaded roots already represent a full snapshot.
- Reason: small workspaces should still get a fully persisted warm snapshot quickly, while large local Sage checkouts
  avoid paying full traversal cost before the first definition request.

### Verification

- Checks run:
  - `python -m pytest packages/sage-lsp/tests/test_index.py -q`
  - `python -m pytest packages/sage-lsp/tests/test_server.py -q`
  - `npm run test`
  - `npm run test:native-smoke`
  - `npm run test:extension-host`
  - local timing probe against `/workspace/sage/src`
- Result: index and server regressions passed, repository tests and native Sage smoke stayed green, the real
  extension-host smoke passed under the approved desktop path, and a native timing probe measured about `9.10s` for a
  full rebuild versus about `1.53s` for warm snapshot hydration plus bootstrap loading, with `PolynomialRing`
  resolving in about `0.000029s` afterward

### Follow-ups

- Next task: keep pushing first-run startup down now that warm snapshots are authoritative again, especially for large
  local Sage trees without a preexisting cache
- Risks or blockers: full workspace-symbol and import-candidate requests still require a complete index when no warm
  snapshot is available
