# Development Progress

## Status Snapshot

- Date: 2026-04-01
- Repository: upgraded to a usable static-analysis baseline
- Process tracking: baseline in place
- Extension package: richer workflow and docs UX added
- Language server package: static indexing, request-level coverage, and config-aware hover behavior added
- Syntax package: baseline scaffold added and synced into extension resources
- Runtime hardening: interpreter launch, path resolution, execution targets, and URI handling aligned
- LSP host runtime: decoupled from the Sage executable and made compatible with Sage Python 3.9 syntax
- LSP lifecycle: restart sequencing and configuration notifications hardened
- Local debugging: repository-level VS Code launch and task scaffolding added
- Manual testing assets: curated smoke workspace added
- Native source support: `.pyx`, `.pxd`, and `.pxi` now participate in indexing, navigation, and highlighting
- VS Code dev helper: one-command repository prep and launch script added
- Standard LSP baseline: workspace symbols, references, rename, and unresolved-import diagnostics now work through the server
- Interpreter discovery: `Sage: Select Interpreter` now pre-populates Sage and Python candidates from the local machine and routes them to the correct settings
- Runtime source-root discovery: selected Sage runtimes now contribute inferred library roots to indexing when no explicit source-root config is present
- Runtime Sage fallback: docs, definitions, and signature help can now query the selected Sage runtime when static indexing misses
- Advanced smoke workspace: graph, elliptic-curve, ideal, symbolic, combinatorics, and number-field samples now exercise heavier Sage usage
- Runtime invocation hardening: runtime Sage introspection now launches subprocesses with argv-safe invocation and direct unit coverage
- Singleton member resolution: dotted Sage APIs such as `graphs.PetersenGraph` now resolve statically through class-body imports, singleton instances, and member completion
- Managed LSP shutdown: extension-driven stop/restart flows now suppress library-level auto-restart to avoid duplicate launches, cancelled requests, and code-0 exits
- Extension-host automation: a real VS Code smoke harness now opens a copied workspace, exercises hover/definition/completion, references, rename, symbols, native Cython, optional native Sage source trees, and validates managed restart stability
- Completion transport hardening: completion responses are now serialized as concrete LSP items so real clients no longer trip `pygls` JSON conversion errors
- Editor assets: Sage-specific highlighting, snippets, triple-quoted editing, and operator coverage now target broader SageMath workflows
- Conservative syntax diagnostics: Python and `.sage` files now surface syntax errors without flagging valid preparser constructs such as `^` or `R.<x, y>`
- Terminal cleanup: run commands can optionally remove generated `.sage.py` files after standalone terminal execution
- Native library docs generalization: static indexing now inherits factory docstrings, `.pyx` functions contribute hover docs, and runtime metadata enriches static documentation without discarding local source paths
- Native local smoke automation: a non-GUI repository script now validates documentation and source navigation for common Sage library symbols against a real local checkout
- Documentation prewarm: opening a Sage file now preloads a bounded set of likely callable docs into cache before the first hover request
- Highlighting depth: Sage grammar now separates algebraic domains, constructors, symbolic work, plotting, graph theory, combinatorics, crypto, number theory, and linear algebra into richer scopes
- Environment-first interpreter selection: `Sage: Select Interpreter` now prioritizes complete local-dev and system-Sage profiles instead of making users assemble runtime and Python hosts manually
- Definition prewarm: opening a Sage document now warms both documentation and definition caches for likely callable targets, reducing first-jump latency alongside first-hover latency
- Runtime import-root hardening: runtime fallback now receives the resolved language-server Python host plus discovered `sage/src` and `builddir*/src` roots for local checkout scenarios
- Structure-aware highlighting: grammar and snippets now cover cached decorators, runtime helper names, toric and module namespaces, factory-style assignments, and a dedicated manual smoke file for visually checking heavier Sage source patterns
- Python-like `.sage` fast path: `.sage` files without preparser assignment now parse through the Python AST path, preserving richer class, method, and import analysis for mixed Sage/Python scripts
- Saved `.sage` indexing: workspace builds now index `.sage` and `.pxi` files so navigation is not limited to transient open-document parsing
- Resolution caching: indexed-module symbol and member lookups now reuse cached results to reduce repeated definition and documentation resolution cost
- Semantic-token baseline: the server now publishes semantic tokens for Sage namespaces, constructors, decorators, readonly library values, methods, and preparser generator declarations
- Extension-host smoke revalidated: the real VS Code smoke harness now passes again under an approval path that permits launching the local app in the background
- Hybrid preparser parsing: `.sage` files that mix `R.<x>`-style declarations with ordinary Python class/method code now merge loose Sage extraction with AST structure instead of dropping the whole file to the weaker parser
- Persistent index cache: indexed source roots now reuse serialized module records across rebuilds, invalidate entries when source files change, and fall back safely when the preferred cache location is not writable
- Open-document overlay cache: unchanged editor buffers now reuse parsed records across hover/definition/completion requests, update on edits, and clean up on close
- Index readability cleanup: recent warm-cache and hot-overlay logic is now split into smaller helpers so the indexing path is easier to follow and maintain

## Current Focus

1. Extend source mapping beyond caret rewrite into more `.sage` constructs.
2. Feed source mapping into diagnostics, references, and rename paths.
3. Persist library/workspace index data so restart cost drops for large Sage trees.
4. Broaden semantic-token coverage and keep refining editor-side highlighting for real Sage source style.
5. Deepen runtime-aware Sage introspection beyond docs/definitions/signatures where useful.

## Milestone Tracker

| Milestone | Status | Notes |
| --- | --- | --- |
| Repository bootstrap | Done | Root docs, package scaffolds, onboarding docs, CI placeholder, and local bootstrap validation are complete. |
| Process baseline | Done | Commit policy, task flow, and progress templates are now committed. |
| Design baseline | Done | Initial overview, workspace, server boundary, and source mapping notes are committed. |
| Extension scaffold | Done | Minimal VS Code client package, commands, configuration model, and language client wiring are committed. |
| LSP scaffold | Done | Minimal `pygls` package, entrypoint, and server settings model are committed. |
| Syntax scaffold | Done | Syntax package, sync script, and generated extension assets are committed. |
| Source mapping v1 | Done | `.sage` caret rewrite, string/comment skipping, bidirectional column maps, and hover preview wiring are committed. |
| Static source intelligence baseline | Done | Parser, workspace index, lazy import resolution, docs extraction, and LSP-backed symbol features are committed. |
| Extension workflow baseline | Done | Status bar, environment presentation, run commands, docs panel, and richer settings model are committed. |
| Local debugging baseline | Done | Repository-local `F5` launch, prelaunch build task, and source-map-enabled extension builds are committed. |
| Manual smoke workspace | Done | Curated `.sage`, `.py`, and `.pyx` examples plus a dedicated extension-host launch target are committed. |
| Runtime hardening | Done | Interpreter-driven LSP launch, workspace-relative path handling, request-level LSP tests, run-target-aware terminals, and hover-doc preference handling are now committed. |
| LSP host runtime split | Done | The server now runs in a dedicated Python environment, configures its own runtime path, and remains source-compatible with Sage Python 3.9. |
| Native source support | Done | The plugin now treats `.pyx`, `.pxd`, and `.pxi` as first-class documents for highlighting, indexing, and lightweight navigation. |
| Developer workflow | Done | A helper script now prepares the repository and opens VS Code with the local launch configs ready to use. |
| LSP baseline | Done | Workspace symbols, references, rename, and conservative diagnostics now sit alongside hover, completion, definition, and document symbols. |
| Editor authoring baseline | Done | Syntax assets, snippets, triple-quoted editing, terminal cleanup, and background extension-host smoke automation now support richer Sage authoring workflows. |
| Native library documentation baseline | Done | Common Sage library constructors and `.pyx` functions now expose documentation and source paths through merged static/runtime analysis plus local smoke coverage. |
| Python-like `.sage` performance baseline | Done | `.sage` files that look like Python now keep AST-grade analysis, saved `.sage` files enter workspace indexing, semantic tokens are available, and repeated indexed lookups reuse caches. |
| `.sage` projection and quick-fix baseline | Done | `.sage` syntax diagnostics now project back to source-facing ranges, deterministic unresolved-import-name diagnostics expose import quick fixes, and both paths are covered through real extension-host smoke. |

## Change Log Notes

- Initialized an independent repository and set `main` as the default branch.
- Added root governance and architecture documents.
- Reserved `docs/design`, `docs/process`, and `docs/progress` for ongoing repository records.
- Added process templates for commit policy, task state flow, development logs, and milestone reviews.
- Added the first `extension-core` scaffold with commands, settings mapping, and stdio language-client wiring.
- Added the first `sage-lsp` scaffold with a `pygls` entrypoint, server settings model, and basic tests.
- Added the first `syntax-pack` scaffold plus a sync script that materializes extension-owned runtime assets.
- Added developer onboarding docs and a bootstrap GitHub Actions workflow aligned with root scripts.
- First dependency-install pass exposed and isolated a Node workspace wiring issue in the extension package manifest.
- Installed Node and Python dependencies locally, corrected bootstrap lifecycle typing, and verified `npm run build`, `npm run lint`, and `npm run test`.
- Accepted the first concrete `.sage` mapping slice around caret-to-power rewrite with bidirectional line-local column maps.
- Implemented the first real `.sage` preprocessing module, covered it with Python tests, and surfaced mapping information in the server hover path.
- Ported a reduced Sage fixture corpus plus parser/index stack into the `pygls` server and verified static hover, completion, definition, symbol, and documentation paths through tests.
- Upgraded the VS Code client with richer configuration, source-root discovery, run commands, status presentation, documentation rendering, and unit-test coverage.
- Aligned language-server startup with the configured interpreter, corrected workspace-relative source-root and extra-path resolution, normalized Windows-style file URIs, and fixed the `pygls` server import path used at runtime.
- Added request-level `pygls` coverage for initialize, hover, definition, completion, document symbols, and custom documentation requests.
- Taught the extension to manage dedicated run and REPL terminals, honor `sage.run.target`, reset stale REPL state when interpreter settings change, and avoid restarting the language server for run-target-only setting edits.
- Extended server-side environment parsing to cover documentation, logging, and experimental settings, then made hover output respect the client's documentation-preview preference.
- Wired resolved `analysis.extraPaths` into both language-server initialization payloads and server-side indexing so external source roots affect analysis instead of only process import state.
- Added repository-local `.vscode` launch and task definitions so `F5` starts the extension development host after a root build, and enabled source maps for direct extension-side TypeScript debugging.
- Added a curated manual smoke-test workspace with source-root modules, extra-path modules, `.pyx` coverage, lazy-import cases, source-mapping examples, and a dedicated extension-host launch target.
- Split the LSP host runtime from the Sage executable, added explicit `sage.languageServer.*` settings, updated startup error reporting, and removed Python 3.10+/3.11+ syntax that broke Sage's bundled Python 3.9.
- Serialized extension-side restart requests so overlapping configuration and workspace events no longer tear down an in-flight language-server connection, and added an explicit `workspace/didChangeConfiguration` handler on the server.
- Added lightweight native-source parsing for `.pyx`, `.pxd`, and `.pxi`, including `cimport` resolution and module merging between declarations and implementations.
- Registered native Sage Cython files in the extension so language-server activation, document selection, and file watching now cover the same source set as the parser and grammar.
- Replaced the placeholder syntax grammar with a broader Sage/Cython grammar and expanded the smoke workspace with `.pxd` and `.pxi` fixtures for manual validation.
- Added a repository-local `dev-vscode.sh` helper plus root npm shortcuts so contributors can bootstrap, build, and open the project in VS Code with a single command.
- Added a standard LSP baseline slice with workspace symbol search, cross-file references, rename edits, and unresolved-import diagnostics, then covered those request paths with `pygls` tests.
- Fixed loose `.sage` lazy-import alias parsing so smoke-workspace aliases no longer degrade into false unresolved-import diagnostics.
- Switched diagnostics publication over to the `pygls` `textDocument/publishDiagnostics` API so document open/change events no longer crash the server on missing `publish_diagnostics`.
- Removed the `import.meta.dirname` dependency from the syntax-sync script so repository bootstrap and `dev:vscode` flows run on Node 20 environments.
- Reworked `Sage: Select Interpreter` again into an environment-first picker that promotes `Local Sage development environment` and `System Sage (stable)` before the advanced custom-path actions.
- Added automatic Sage source-root discovery so the extension can infer `.../src` or nearby `site-packages` roots from the selected runtime and feed them into the language server without manual `sourceRoots` setup.
- Added runtime-backed documentation and definition fallback through Sage's own introspection helpers so real Sage objects remain navigable when static indexing misses them.
- Added runtime-backed signature help plus dotted-name fallback, allowing calls such as `graphs.PetersenGraph(...)` to retain docs and signatures instead of collapsing to a bare final identifier.
- Expanded the smoke workspace with heavier SageMath scenarios across graph theory, elliptic curves, ideals, symbolic calculus, combinatorics, and number fields.
- Fixed runtime introspection subprocess launching so Python 3.12 no longer misreads the argument vector as `bufsize` and drops hover or definition requests.
- Extended static analysis with class-body import handling, singleton instance tracking, dotted member resolution, member completion, and workspace-symbol coverage for common Sage generator objects.
- Suppressed client-library auto-restart during extension-managed language-server shutdown so configuration-triggered restarts no longer produce duplicate launches and spurious code-0 exits.
- Added a real extension-host smoke harness that launches the local VS Code app against a copied smoke workspace and validates hover, definition, completion, and repeated managed restarts end to end.
- Fixed completion serialization for real VS Code clients after the new extension-host smoke test exposed dictionary-based completion payloads that `pygls` could not encode.
- Expanded Sage grammar, snippets, and language configuration so common rings, fields, graphs, plots, symbolic functions, operators, and triple-quoted authoring flows are covered by automated syntax-asset tests.
- Added conservative syntax diagnostics for Python and `.sage` documents, including valid-preparser exceptions for caret exponentiation and multi-generator `R.<x, y>` declarations.
- Added an optional run-command cleanup toggle for generated `.sage.py` files and deepened the extension-host smoke harness to cover references, rename, symbols, native Cython navigation, and optional native Sage source-tree lookups.
- Hardened runtime introspection with an isolated writable Sage home plus a longer timeout so common library lookups stop failing during cache initialization or slower import paths.
- Merged runtime documentation back into static documentation when runtime signatures are richer, keeping local source paths while improving hover details for constructors such as `PolynomialRing`.
- Extended static documentation extraction so factory-style assignments such as `EllipticCurve = EllipticCurveFactory(...)` inherit class docstrings even when runtime introspection is unavailable.
- Taught `.pyx` parsing to extract function docstrings, allowing common native APIs such as `matrix` to surface hover documentation directly from source.
- Added a non-GUI `test:native-smoke` repository script that validates summaries and definition paths for `graphs.PetersenGraph`, `PolynomialRing`, `EllipticCurve`, `matrix`, `NumberField`, and `Partitions` against the local Sage checkout.
- Added bounded documentation prewarming on document open so common call targets are cached before the first hover request.
- Reworked Sage grammar scopes so rings and fields, constructors, symbolic functions, plotting, graph theory, combinatorics, crypto, number theory, and linear algebra no longer share one flat generic support scope.
- Extended document-open prewarming to definitions as well as docs, and added shared request-cache invalidation so repeated jump requests can reuse the same resolved targets.
- Taught the extension to pass the resolved language-server Python path down to the server environment so runtime-fallback decisions can distinguish local checkout development from stable installed Sage.
- Taught runtime fallback to expand local checkout import roots with matching `builddir*/src` directories, so compiled native modules remain available when runtime probes are driven from the development Python host.
- Expanded syntax assets again so `@cached_method`, `lazy_import`, `UniqueFactory`, `toric_varieties`, `ChowGroup`, `FilteredSimplicialComplex`, and related structure-heavy Sage patterns receive stronger, more intentional scopes instead of blending into generic support tokens.
- Added manual smoke coverage for the new highlighting path through `08_highlighting_structures.sage` and corresponding workspace instructions.
- Added a Python-like `.sage` parsing fast path that preserves AST-grade symbol extraction, member tracking, and import handling whenever the file does not use preparser assignment syntax.
- Started indexing saved `.sage` and `.pxi` files during workspace builds so navigation and symbol queries can see those modules even before they are opened in the editor.
- Added indexed-module symbol and member resolution caches to reduce repeated lookup cost during documentation, definition, completion, and workspace-symbol requests.
- Added a first semantic-token baseline for Sage namespaces, constructors, readonly library values, decorators, methods, and preparser generator declarations.
- Revalidated the real VS Code extension-host smoke flow after the `.sage` parser and semantic-token changes by rerunning the background host against the copied smoke workspace.
- Added a hybrid `.sage` parse path so preparser-heavy files can keep top-level Sage declarations while regaining AST-derived methods, instance tracking, and member completion for the rest of the module.
- Added a persistent module-cache baseline for indexed source roots, plus cache invalidation on file changes and automatic fallback when the preferred cache directory cannot be written.
- Added a hot open-document overlay layer so repeated requests against unchanged editor buffers no longer re-parse the same text, while document-close events now drop that overlay state cleanly.
- Refactored the recent index/cache code into clearer helper functions so warm-cache loading, module storage, document overlays, and cache-eligibility checks are easier to reason about without changing behavior.
- Refactored the language-server request flow so hover/definition wrappers, open-document overlay refresh, and empty-diagnostics publication now live behind smaller helpers instead of being repeated across `server.py`.
- Added incremental refresh and removal for indexed workspace files, so saved or watched Python/Sage/Cython modules now update the persistent index and merged native-module state without requiring a full rebuild.
- Extended the real extension-host smoke suite so it edits and saves an imported Python helper module, then verifies that hover inside a `.sage` consumer updates without a manual language-server restart.
- Batched multi-file watcher/save indexing work so change bursts now persist cache state and clear resolution caches once per batch instead of once per file.
- Reused cached module source during warm index startup so unchanged files can be reconstructed from persistent cache data without rereading every module source file.
- Projected `.sage` syntax diagnostics back to original source ranges so caret-based preparser syntax no longer reports generated-text columns to the editor.
- Added standard LSP import quick fixes for deterministic unresolved-import-name diagnostics and validated them through both request-level tests and real extension-host smoke.
