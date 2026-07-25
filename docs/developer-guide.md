# Developer Guide

## Purpose

This guide is for engineers working on the plugin itself. It explains how the repository is organized and how the
current static-analysis baseline should be extended safely.

## Repository Map

- `packages/extension-core`
  VS Code extension client. Owns activation, command registration, configuration plumbing, status presentation,
  documentation panel rendering, workspace discovery, and LSP client startup.
- `crates/sage-ls`
  Rust V2 language-server process. Owns the primary LSP entrypoint, request dispatch, open-document overlays,
  semantic-token encoding, and Rust status/rebuild/documentation commands.
- `crates/sage-index`
  Rust V2 indexing engine. Owns parallel file scanning, Sage/Python/Cython symbol extraction, source preprocessing,
  SQLite/FTS persistence, and release indexing benchmarks.
- `packages/sage-lsp`
  Legacy Python language server retained as a migration baseline. It still carries the previous `pygls` implementation,
  tests, fixtures, and debug hooks, but new runtime work should target the Rust V2 crates unless a task explicitly
  concerns migration parity.
- `packages/syntax-pack`
  Shared syntax assets. Owns grammar, snippets, and language configuration for `.sage` and Sage-native Cython files.
- `docs/design`
  Architecture notes and accepted design decisions.
- `docs/plugin-completeness.md`
  Manifest-to-documentation completeness reference. The extension metadata tests require every contributed command and
  setting to appear in the English user docs.
- `docs/process`
  Release-gate notes that are still exercised by repository hygiene tests.
- `docs/progress`
  Short current status and active task tracking.
- `.vscode`
  Repository-local launch and task definitions for starting the extension development host with `F5`.
- `scripts/dev-vscode.sh`
  Helper entrypoint that can bootstrap dependencies, sync syntax assets, build the repo, and open VS Code in one step.
- `scripts/debug-workbench.mjs`
  Local Browser Use debugging workbench. It serves smoke fixtures with TextMate scope matches, Rust semantic spans,
  diagnostics, symbols, and index/docs status without depending on VS Code Web screenshots.
- `scripts/export-reference.mjs`
  Static offline reference exporter. It writes `.sage-reference/index.html` plus local JS/CSS/data shards so a project can
  be searched and read without VS Code, Sage, this extension, or a local server.
- `scripts/reference-viewer`
  Source assets for the offline reference viewer. Keep presentation and browser interaction work here; the exporter
  should stay focused on inspection, data shaping, sanitization, and file emission.
- `scripts/cache-maintenance.mjs`
  Safe inventory and prune helper for root-aware Rust SQLite index caches. Inventory is the default; prune mode is a
  dry-run unless `--yes` is explicitly supplied.
- `examples/manual-smoke-workspace`
  Self-contained manual smoke-test workspace used to exercise hover, definition, completion, docs, `.pyx`, `.pxd`,
  `.pxi`, and `.sage` cases plus heavier runtime-backed Sage examples.

## Working Rules

1. Start by identifying the subsystem: `extension`, `lsp`, `syntax`, or `repo/docs`.
2. Keep the client thin unless a UI concern truly belongs in VS Code.
3. Keep new analysis behavior in the Rust V2 server/index crates; extension code should consume stable server payloads
   rather than duplicate logic.
4. Update progress records when milestone status changes.
5. Add or update a design note when architecture or repository rules change.

## Key Extension Modules

- `src/extension.ts`
  Composition root for activation, deactivation, shared state, and module wiring. Keep feature implementations in the
  focused modules below so lifecycle races can be tested without exercising the whole extension host.
- `src/configuration.ts`
  Reads VS Code settings into the stable `SageSettings` model.
- `src/documentSelector.ts`
  Builds the language-client selector and keeps the read-only `sage-source:` scheme outside the normal client route so
  it has exactly one navigation provider.
- `src/workspaceDiscovery.ts`
  Determines which workspace roots should be indexed and augments them with Sage roots derived from the selected
  runtime when `sage.analysis.sourceRoots` is left empty.
- `src/sourceRootPaths.ts`
  Normalizes configured, indexed, and workspace paths, resolves physical aliases, tests source-root containment, and
  maps canonical results back to the workspace URI identity that VS Code opened.
- `src/languageClient.ts`
  Starts the Rust `sage-ls` process, watches Sage/Cython document types, and sends command-backed documentation/status
  requests while suppressing client-library auto-restarts during extension-managed shutdown and restart cycles.
- `src/sageCommandClient.ts` and `boundedOperation.ts`
  Keep execute-command requests on the typed LSP protocol overload and bound status/start/stop waits. Do not switch
  command requests back to the string-plus-token overload: it serializes the parameters as a positional array.
- `src/sageSourceView.ts` and `backingFileWatchRegistry.ts`
  Serve read-only external Sage sources and share directory watchers across their backing files. Directory watches keep
  virtual documents current across atomic saves and Git checkouts; reads are registered before I/O to avoid a stale
  first snapshot.
- `src/externalSourceNavigation.ts`
  Bridges definition, declaration, implementation, type-definition, and reference requests from a read-only
  `sage-source:` editor to its backing `file:` URI, then rewrites results back to the appropriate visible URI. It verifies
  the visible text against disk before forwarding a position and must not open a second hidden `file:` document.
- `src/executionCommands.ts`
  Registers file, selection, and cell execution commands. Terminal/process construction remains in `executionPlan.ts`
  and `terminalManager.ts` so command handlers only validate editor and workspace state.
- `src/executionPlan.ts`
  Builds shell-safe run commands and REPL load commands from extension settings, including optional cleanup of
  generated `.sage.py` files after standalone terminal runs or managed-REPL file loads.
- `src/navigationCommands.ts`
  Registers user-facing documentation and reference commands and routes them through the active backing document.
  Pure payload and label helpers remain in `sageNavigation.ts` and `referenceQuickPick.ts`.
- `src/indexRebuild.ts`
  Waits for an installed rebuild generation, detects a rebuild superseded by another index operation, and reschedules
  within a bounded timeout instead of reporting an unrelated refresh as success.
- `src/statusRefreshController.ts`
  Owns generation-safe status polling until background index work becomes idle; stale in-flight ticks cannot cancel a
  newer polling schedule.
- `src/statusCommands.ts`
  Registers environment, index, documentation, support-bundle, rebuild, and restart commands. Formatting remains in
  `environmentPresentation.ts` and `statusReports.ts`, and status-bar actions remain in `statusMenu.ts`.
- `src/workspaceSettingsJson.ts`
  Applies narrowly scoped JSONC setting edits while preserving comments, indentation, line endings, and a byte-order
  mark, and refuses ambiguous duplicate keys or malformed input.
- `src/serverRestart.ts`
  Limits language-server restarts to configuration changes that actually affect analysis behavior and keeps close/restart
  policy explicit during managed shutdown.
- `src/serverLaunch.ts`
  Resolves the Rust language-server binary from `sage.languageServer.rustPath`, `SAGE_LS_PATH`, local `target/*`
  builds, or `PATH`. The legacy Python launch resolver remains for migration tests.
- `src/interpreterDiscovery.ts`
  Finds Sage runtime/Python candidates and resolves quick-pick selections into configuration updates. Keep path probing
  and selection handling here so `extension.ts` only applies settings and reports the result.
- `src/documentationRequest.ts`
  Normalizes documentation payloads into a render-friendly shape.
- `src/docsPanel.ts`
  Owns the documentation webview lifecycle.
- `src/documentationFallback.ts`
  Keeps the no-documentation action labels and command mapping in one place. Use it when adding diagnostics or recovery
  actions for documentation misses.

## Key Language Server Modules

Rust V2:

- `crates/sage-ls/src/main.rs`
  Composition root for `tower-lsp` capabilities, request dispatch, shared server state, and internal `sage.__rust.*`
  execute commands. Keep navigation, text conversion, editor features, and background work in their modules below.
- `crates/sage-ls/src/navigation.rs`
  Owns definition, declaration, type-definition, and implementation responses, including navigation caching, verified
  live ranges, `LocationLink` capability negotiation, and ordered candidate links.
- `crates/sage-ls/src/references.rs`
  Owns high-confidence references, prepare-rename, rename edits, alias-binding identity, and workspace reference
  collection. Changes here must preserve the same identity threshold as navigation.
- `crates/sage-ls/src/open_documents.rs`
  Owns open-document identity and source lookup. It canonicalizes physical aliases only for matching, prefers the newest
  live buffer over indexed disk text, and preserves the client-facing URI used to open that buffer.
- `crates/sage-ls/src/text_positions.rs`
  Defines the LSP coordinate boundary: `sage-index` ranges use UTF-8 byte columns, while every incoming and outgoing LSP
  position uses UTF-16 code units. Incremental text edits and all feature ranges must pass through these helpers.
- `crates/sage-ls/src/call_hierarchy.rs`
  Resolves local/indexed call-hierarchy items and derives incoming and outgoing call ranges from the selected live source.
- `crates/sage-ls/src/source_symbols.rs`
  Builds nested document symbols and ranked workspace symbols from live source plus index records.
- `crates/sage-ls/src/linked_document_prewarm.rs`
  Debounces `load`, `attach`, include, and import prewarming by live-document revision. Parsing runs on detached named
  worker threads behind a shared gate, away from Tokio's LSP request workers.
- `crates/sage-ls/src/index_jobs.rs`
  Coordinates rebuild, cache reconciliation, and refresh installation with work gates and monotonic generations. Blocking
  scans, parsing, and SQLite work run on detached OS threads; stale results are discarded before they can replace a newer
  index.
- `crates/sage-ls/src/editor_features.rs`
  Implements pure selection-range, folding-range, and inlay-hint derivation.
- `crates/sage-ls/src/document_links.rs`
  Extracts and resolves Sage `load`/`attach` and Cython include links without mixing link parsing into request dispatch.
- `crates/sage-ls/src/documentation.rs`
  Owns source-position documentation extraction, runtime-source selection, and compact hover Markdown rendering.
- `crates/sage-ls/src/signature_help.rs`
  Builds signature and parameter metadata, including UTF-16 parameter label offsets required by LSP.
- `crates/sage-ls/src/runtime_docs.rs`
  Owns the opportunistic persistent Sage runtime documentation worker. It starts only when runtime introspection is
  enabled and a usable Sage/Python runtime is configured; otherwise docs status reports a degraded or disabled state
  while static docs keep working.
- `crates/sage-ls/src/tests.rs`
  Holds protocol and cross-feature regression tests; navigation and reference suites live in the adjacent `tests/`
  modules, while focused implementation tests stay next to the code they exercise.

Navigation correctness depends on both boundaries above: identify a file by its canonical physical path, answer from the
latest matching live buffer, return its original client URI, and convert byte columns to UTF-16 only at the LSP edge. Do
not bypass `open_documents.rs` or `text_positions.rs` with direct disk reads or raw `SourceRange` construction.

Rust index:

- `crates/sage-index/src/lib.rs`
  Thin crate entrypoint: declares modules, re-exports the public model/query surface, and selects a writable persistent or
  temporary cache directory. Domain behavior belongs in the modules below.
- `crates/sage-index/src/model.rs`
  Defines index options/status, workspace state, symbols, source ranges, diagnostics, documentation, and query contracts.
- `crates/sage-index/src/workspace_lifecycle.rs`
  Owns rebuild, hydration, incremental reconciliation, refresh, persistence fallback, generations, and operation timings.
- `crates/sage-index/src/workspace_queries.rs`
  Implements source queries, documentation, completion, workspace symbols, and feature-selective query execution.
- `crates/sage-index/src/symbol_resolution.rs` and `crates/sage-index/src/lookup_state.rs`
  Separate Sage-aware import/member/type resolution from root-filtered file, symbol, reference, and in-memory/SQLite
  lookup state.
- `crates/sage-index/src/cache_metadata.rs`, `cache_persistence.rs`, `cache_queries.rs`, and `materialized_cache.rs`
  Split schema metadata and fingerprints, writes, reads, and materialized Sage export/method caches. Keep cache count and
  root validation here so a missing or truncated cache cannot be mistaken for a valid warm index. Hydration reads the
  transactionally persisted metadata on the startup path; the immediately scheduled reconciliation performs exact,
  index-range-backed row validation and rebuilds a logically truncated database. Refresh validates synchronously before
  applying incremental writes.
- `crates/sage-index/src/source_analysis.rs` and `crates/sage-index/src/source_analysis/`
  Route file discovery, `.sage` preprocessing, declarations, imports/exports, diagnostics, semantic tokens, references,
  and source-import analysis through small parsing submodules.
- `crates/sage-index/src/preparser_support.rs`
  Collects logical multiline `R.<x> = ...` statements while respecting delimiters, strings, comments, and explicit
  continuations. Query inference and `.sage` preprocessing share this boundary so neither can act before closure.
- `crates/sage-index/src/query_support.rs` and `crates/sage-index/src/query_support/`
  Route pure call, completion, Sage-type, symbol, and syntax query helpers without expanding the workspace API modules.
  Sage domain catalogs remain in `sage_types.rs`; scope-aware type flow, assignment/RHS inference, and conservative
  local-function return inference live in `sage_type_inference.rs`, `sage_assignment_inference.rs`, and
  `local_function_returns.rs`. `logical_continuation.rs` identifies only complete bracket or explicit-backslash
  continuations so physical indentation inside a logical line cannot prematurely end a function or control-flow suite.
  `local_scopes.rs` owns completion/reference-facing local bindings, parameter extraction, and lightweight definition
  visibility; keep it separate from the stricter type-flow scope map in `lexical_scope.rs`.
- `crates/sage-index/src/sage_specs.rs`, `source_paths.rs`, `symbol_support.rs`, and `syntax_support.rs`
  Hold static Sage mappings and shared path, ranking/deduplication, and syntax primitives.
- `crates/sage-index/src/tests.rs` and `crates/sage-index/src/tests/`
  Keep shared fixtures/helpers in the test root and group cache, reconciliation, parsing, diagnostics, import resolution,
  completion, editor-query, and Sage-navigation regressions by domain.
- `crates/sage-index/src/bin/sage-index-bench.rs`
  Measures cold release indexing against a local Sage source checkout. Use release mode for performance validation.

Legacy Python LSP:

- `src/sage_lsp/parser.py`
  Parses Python, loose `.sage`, and lightweight `.pyx`/`.pxd`/`.pxi` files into a common module model while preserving
  conservative syntax diagnostics for valid preparser constructs.
- `src/sage_lsp/index.py`
  Compatibility facade for the historical public index import path. New index implementation work should live behind
  `workspace_index.py` or smaller purpose-built modules.
- `src/sage_lsp/workspace_index.py`
  Builds workspace state, merges native declaration and implementation modules, and resolves symbols through imports,
  star imports, lazy imports, workspace symbol search, references, rename edits, import diagnostics, and syntax
  diagnostics.
- `src/sage_lsp/index_db.py`
  Persists lightweight module summaries in SQLite for warm global lookups, with JSON summary files retained as fallback
  storage.
- `src/sage_lsp/jedi_bridge.py`
  Adds optional Jedi-backed local Python context completions and leaves Sage-specific static resolution in
  `workspace_index.py`.
- `src/sage_lsp/trace.py`
  Emits bounded structured request and cache traces for debugging slow or unexpected language-server behavior.
- `src/sage_lsp/source_map.py`
  Hosts the first `.sage` preprocessing and bidirectional position mapping primitives.
- `src/sage_lsp/server.py`
  Wires the index and parser into `pygls` request handlers for hover, completion, definition, signature help,
  references, rename, workspace symbols, and diagnostics publication.
- `src/sage_lsp/runtime_introspection.py`
  Queries a live Sage runtime through `sage.misc.sageinspect` when static indexing cannot supply docs, signatures, or
  source locations.
- `tests/fixtures/sage_src_lite`
  Reduced Sage-aligned source corpus used for parser and index regression tests.

## Bootstrap Commands

```bash
npm ci
npm run sync:syntax
npm run test:generated-assets
npm run build:rust
npm run debug:web
npm run build
npm run test
```

Install the legacy Python LSP package with `python -m pip install -e ./packages/sage-lsp[dev]` only when you need to run
or modify the Python migration baseline directly.

Shortcut:

```bash
npm run dev:vscode
```

Automated verification:

```bash
cargo test
npm run test:ci
npm run package:vsix
npm run configure:workspace -- --dry-run
npm run doctor:mac
npm run test
npm run test:vsix-contents
npm run test:vsix-package
npm run test:vsix-install
npm run test:cache-maintenance
npm run test:repo-hygiene
npm run test:native-smoke
npm run test:product-readiness
npm run test:reference-export
npm run test:performance
npm run test:lsp-navigation
npm run test:lsp-shutdown
npm run test:lsp-protocol
npm run test:lsp-latency
npm run test:extension-host
npm run test:release
npm run test:full
```

Use `npm run doctor:mac` when a local Mac package, VS Code CLI, Sage runtime, or Sage source root does not behave as
expected. The command is diagnostic by default; add `-- --strict` for a shell-failing package-artifact check or
`-- --json` for automation.

Use `npm run configure:workspace -- --workspace /path/to/project --profile auto` to reproduce the VS Code
`Sage: Configure Workspace` setup from a terminal on macOS, Linux, or Windows. Use `-- --dry-run --json` in tests or
bug reports when you need to inspect the generated settings without writing files.

Use `npm run test:ci` for the public GitHub-compatible macOS gate. It intentionally avoids private real-file paths and
desktop VS Code while still checking Rust, clippy, lint, tests, package contents, generated VSIX structure, cache
maintenance, repository hygiene, product readiness, and portable performance smoke. Use `npm run test:repo-hygiene`
after changing issue templates, `SECURITY.md`, `SUPPORT.md`, PR templates, or gate definitions. Use
`npm run test:product-readiness` after changing interaction surfaces, visual polish, performance gates, smoke fixtures,
packaging, or future Sage-update support. Use `npm run test:product-readiness -- --json` when a machine-readable report
is needed. Use `npm run test:release` for local release candidates
that have access to a Sage checkout and the real Sage-heavy smoke inputs. See `docs/process/ci-and-release-gates.md` for
the exact split.

Rust release benchmark:

```bash
cargo run --release -p sage-index --bin sage-index-bench -- /path/to/sage/src
```

Release performance gate:

```bash
SAGE_SOURCE_ROOT=/path/to/sage/src npm run test:performance
```

Use `-- --skip-workbench` when you only need the release index budgets and do not want to run the Browser workbench
latency smoke in the same pass. The performance smoke uses an isolated temporary `XDG_CACHE_HOME` by default so cold
rebuild numbers are not polluted by an old or fragmented SQLite cache. Set `SAGE_PERF_CACHE_DIR=/path/to/cache` to inspect
the generated cache, and set `SAGE_PERF_KEEP_CACHE=1` to reuse that cache intentionally.

Rust index cache maintenance:

```bash
npm run cache:status
npm run cache:prune:dry-run
node scripts/cache-maintenance.mjs --prune --max-age-days 30 --max-total-bytes 2147483648 --keep-latest 2 --yes
```

`cache:status` reads the same default cache root as Rust (`$XDG_CACHE_HOME/sage-vscode-plugin/rust-index-v2`, or
`$HOME/.cache/sage-vscode-plugin/rust-index-v2`). Prune mode only matches `sage-index-<digest>.sqlite` plus its SQLite
`-wal`/`-shm` sidecars, keeps the newest namespaces, applies an optional total-size budget, and removes old orphan sidecars.
The VS Code extension applies the same conservative policy to its own global-storage `rust-index-v2` directory before
starting `sage-ls`; `npm run test:cache-maintenance` validates the deletion path in a temporary directory.

`npm run test:native-smoke` validates common library symbols such as `PolynomialRing`, `EllipticCurve`, `matrix`,
`NumberField`, `Partitions`, and `graphs.PetersenGraph` against a real local Sage source checkout without launching VS
Code.

`Sage: Select Interpreter` is now environment-first. The primary choices should be `Local Sage development environment`
for a nearby checkout paired with `conda` `sage-dev`, and `System Sage (stable)` for the installed standalone Sage
runtime.

`Sage: Run UX Self Check` runs the active editor position through the Rust query payload and writes a compact report to
the Sage output channel. Use it before collecting logs for hover/docs/definition/completion/references/rename/signature
or diagnostics regressions.

Rust V2 documentation uses static indexed docstrings first. For weak built-in Sage placeholders such as
`PolynomialRing` or `graphs.PetersenGraph`, `sage-ls` can ask a persistent Sage runtime worker for fuller
`sageinspect` documentation. If the configured runtime cannot import Sage, `Sage: Show Docs Status` reports the
degraded reason and hover/docs continue to use static fallback text.

`npm run test:extension-host` launches the locally installed VS Code desktop in an unattended background test session.
The harness copies the smoke workspace into a temp directory, captures extension-host and language-server logs, and
fails the run on known runtime regressions.

`npm run debug:web` builds the Rust debug inspector and starts the local Browser Use workbench. Open the printed
`http://127.0.0.1:<port>/` URL to inspect `08_highlighting_structures.sage`, native Cython fixtures, TextMate scopes,
semantic tokens, diagnostics, symbols, and Rust index/docs status in a stable browser surface.

`npm run export:reference -- --workspace /path/to/project --source-root /path/to/sage/src` generates a shareable
`.sage-reference/` static viewer for a project. `npm run test:reference-export` validates the generated manifest, symbol
index, source shards, keyboard/hash viewer behavior, documentation rendering, and private-path stripping in a temporary
fixture.

`npm run cache:status` inventories root-aware Rust index caches without deleting files. Use `npm run cache:prune:dry-run`
to preview old-cache cleanup; only the explicit `node scripts/cache-maintenance.mjs --prune --max-age-days 30 --yes`
form deletes matching old SQLite cache files.

## Local Debugging

- Press `F5` in this repository. `Sage Plugin: Smoke Workspace` is the first/default target for GUI smoke testing.
- The repository-level `build` task runs first, then VS Code launches the extension from `packages/extension-core`.
- Choose `Sage Plugin: Extension Host` only when you specifically want the current repository as the first workspace in
  the extension host.
- In a correct smoke session, the new window title starts with `[Extension Development Host]`, `.sage` files show
  `SageMath`, the command palette contains Sage commands, and the left status bar starts with `Sage:`.
- If the host opens empty, use `Open Folder` inside that host and select `examples/manual-smoke-workspace`. If `.sage`
  shows `Plain Text`, close that normal window and relaunch from the repository with `F5`.

## Language Intelligence Troubleshooting

1. Open the `Sage` and `Sage Language Server` output channels.
2. Set `sage.logging.level` to `debug`, then run `Sage: Restart Language Server`.
3. Use `Sage: Show Environment Details` to confirm source roots, extra paths, index mode, and runtime-introspection
   status.
4. For Rust V2, run `Sage: Show Index Status`, `Sage: Show Docs Status`, or `Sage: Rebuild Index`. For legacy Python
   tests, call `sage/__debug/indexStatus` to inspect generation, loaded roots, deferred roots, module counts, overlay
   counts, and summary-cache state.
5. Reduce hover, definition, or completion failures into a fixture under `packages/sage-lsp/tests/fixtures` or
   `examples/manual-smoke-workspace` before changing resolver behavior.
6. If cache size or stale checkout state is suspected, run `npm run cache:status` first and use dry-run prune output
   before deleting any root-aware SQLite cache.

## Verification Strategy

- TypeScript-facing work should keep `npm run build` and `npm run lint` green.
- Manifest, command, setting, or user-facing documentation work should keep
  `npm run test --workspace sage-vscode-extension` green; this includes command activation coverage, generated asset
  existence checks, and user documentation coverage.
- Rust-facing work should keep `cargo test` green and should record `cargo run --release -p sage-index --bin
  sage-index-bench -- /path/to/sage/src` when performance is relevant.
- Python-facing work should keep `npm run test:python` green.
- Syntax or generated branding work should keep `npm run sync:syntax`, `npm run test:generated-assets`, and syntax
  package checks green.
- Cache maintenance or release tooling changes should keep `npm run test:cache-maintenance` green.
- Cross-cutting changes should keep the full `npm run test` path green before commit.
- Browser-debug-facing changes should keep `npm run test:debug-web` green.
- Packaging-facing changes should keep `npm run package:vsix`, `npm run test:vsix-package`, and
  `npm run test:vsix-install` green; use the exact `.node-version` runtime. `package:vsix` stages a locked, path-remapped
  current-platform release `sage-ls` before archive checks, and the package smoke verifies normalized modes under
  different umasks.
- Release-candidate changes that do not need the desktop extension host should keep `npm run test:release` green. This
  runs locked Rust tests, clippy with `-D warnings`, TypeScript lint, the full non-desktop test suite, VSIX content/package
  install smoke, release index performance, persistent LSP latency, the real-file Sage-heavy smoke, and
  `git diff --check`.
- Native Sage library work should also keep `npm run test:native-smoke` green when a local Sage checkout is present.
- Extension-host behavior that depends on the real VS Code client lifecycle should also keep `npm run test:extension-host`
  green.
- The extension-host smoke suite now covers hover, definition, completion, references, rename, document/workspace
  symbols, native Cython navigation, managed restart stability, imported-helper save refresh, and optional native Sage
  source-tree lookups.
- Use `npm run test:full` when you need the whole repository suite plus the extension-host smoke test in one command.
