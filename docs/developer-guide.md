# Developer Guide

For people changing the plugin. Users start with the [User Guide](./install-and-configure.md); the design is in
[ARCHITECTURE.md](../ARCHITECTURE.md).

## Setup

Node.js 22.9+ (26 is covered too) and npm 11+; Rust comes from `rust-toolchain.toml`.

```bash
npm ci
npm run build:rust      # target/debug/sage-ls
npm run build           # extension + syntax assets
```

`npm run dev:vscode` does the same and opens the repository in VS Code.

## Repository Map

| Path | Owns |
| --- | --- |
| `packages/extension-core` | VS Code client: activation, commands, status, REPL/run, docs panel, LSP client startup |
| `crates/sage-ls` | language server: LSP handlers, open documents, background index jobs, runtime docs worker |
| `crates/sage-index` | indexing: discovery, `.sage`/Python/Cython parsing, symbol resolution, SQLite cache |
| `packages/syntax-pack` | grammar, snippets, language configuration (generated into the extension) |
| `examples/manual-smoke-workspace` | fixture workspace for manual and automated editor tests |
| `scripts/` | packaging, smoke tests, perf tools, `configure-workspace`, `doctor`, `export-reference` |
| `docs/design` | design notes that still describe current behavior |

## Debugging

**In VS Code.** Press `F5` and choose **Sage Plugin: Smoke Workspace**. In the `[Extension Development Host]` window,
`.sage` files show `SageMath` and the status bar shows `Sage: ...`. If a `.sage` file shows `Plain Text`, you are in a
normal window; start again with `F5`. `sage.languageServer.rustPath = auto` picks up `target/debug/sage-ls` after
`npm run build:rust`.

**Logs.** The **Sage** and **Sage Language Server** output channels. The server logs every request that takes 1 s or
more (`[sage-ls] references demo.sage:12:5 1500 ms (slow)`); with `sage.logging.level` set to `debug` and the server
restarted it logs every request (`SAGE_LS_LOG=debug` when running `sage-ls` by hand). Timing lives in
`crates/sage-ls/src/request_trace.rs`. **Sage: Run UX Self Check** runs hover, docs, definition, completion, references, rename, signature help, and
diagnostics at the cursor and writes one report.

**Without VS Code.**

- `npm run debug:web` opens a browser workbench showing TextMate scopes, semantic tokens, diagnostics, symbols, and
  index status for the fixture files.
- `cargo run -p sage-index --bin sage-debug-inspect -- --root <dir> --file <file> --symbol <name>` prints what the
  index resolves for one query.
- `npm run snapshot:index -- /path/to/sage/src > before.jsonl`, then again after a parser change, and
  `npm run snapshot:diff -- before.jsonl after.jsonl` shows every symbol, highlight, or diagnostic that changed.

**Reduce, then fix.** Turn a wrong hover, definition, or completion into a unit test under
`crates/sage-index/src/tests/` (or a fixture file) before changing resolver code.

## Tests

| Command | When | What it runs |
| --- | --- | --- |
| `cargo test` | any Rust change | unit and protocol tests for both crates |
| `npm run test` | any TypeScript change | extension, syntax, script, and fixture-only workbench tests |
| `npm run test:extension-host` | client lifecycle or UI changes | the real VS Code client against the fixture workspace (opens VS Code windows) |
| `npm run test:ci` | before pushing | Rust tests + clippy, lint, all `test`s, LSP protocol/shutdown/concurrency smokes, real-Sage workbench matrix, VSIX checks, hygiene, readiness, portable perf |
| `npm run test:release` | before tagging | `test:ci` plus VS Code CLI install, release index performance, persistent LSP latency, real-file smoke |

`test:ci` and `test:release` need a Sage source tree: a sibling `../sage/src` checkout or `SAGE_SOURCE_ROOT`. A Sage
executable is not required. Keep `test:lsp-latency`, `test:real-file-smoke`, and `test:extension-host` out of
`test:ci`; they depend on local machine state.

Focused gates: `test:generated-assets` after syntax or icon changes, `test:vsix-package` / `test:vsix-install` after
packaging changes, `test:cache-maintenance` after cache cleanup changes, `test:repo-hygiene` after editing
`SECURITY.md`, `SUPPORT.md`, `CONTRIBUTING.md`, `.gitattributes`, `.editorconfig`, issue templates, or workflows, and
`test:reference-export` after offline reference changes.

### CI

- `ci.yml` (every push and PR): the macOS `test:ci` gate with a sparse `sagemath/sage` checkout, `cargo test` on Linux,
  Windows, and Intel macOS, the Linux extension-host suite under `xvfb`, Node 22/26 compatibility, and a Node 26/npm 12
  packaging check. Rust caches are saved even when a job fails.
- `package.yml` (tags, PRs touching shipped code, manual runs): builds and tests the six native VSIX packages plus two
  Alpine aliases. A manual run can be limited to some targets:

```bash
gh workflow run package.yml -f targets=linux-x64,win32-x64
```

On GitHub's shared runners, index parse and internal rebuild budgets are 3000 ms instead of the local 1500 ms, the
end-to-end rebuild 8000 ms instead of 5000 ms, and the warm inspector queries get twice the local 1 s budget
(`SAGE_LATENCY_BUDGET_SCALE`). Local numbers stay the optimization target.

## Performance

`npm run perf:baseline -- --out docs/perf/baseline-<date>.json` measures start, latency while indexing and typing,
references, and memory against a full Sage tree. Compare with the files in [docs/perf](./perf/README.md).
`cargo run --release -p sage-index --bin sage-index-bench -- /path/to/sage/src` times a cold index.

## Index Cache Tools

`npm run cache:status` lists index databases; `npm run cache:prune:dry-run` previews cleanup and
`node scripts/cache-maintenance.mjs --prune --max-age-days 30 --yes` deletes. Close Sage windows before pruning.
`npm run clean:cache -- --yes` removes Rust build caches; `npm run clean -- --yes` also removes packaged artifacts.

## Releasing

1. Bump the version in the root, `packages/extension-core`, and `packages/syntax-pack` `package.json` files and in
   `package-lock.json` (`npm run test:repo-hygiene` checks they agree with the Cargo crates).
2. Move the `packages/extension-core/CHANGELOG.md` entries under `## <version> - <date>`.
3. Run `SAGE_SOURCE_ROOT=../sage/src npm run test:release` and `npm run test:extension-host`.
4. Push, wait for green CI, then tag: `git tag -a v<version> -m "..." && git push origin v<version>`.

The tag runs the CI gate and all packaging jobs. Only if everything passes, the workflow checks the tag matches the
extension version and that exactly eight VSIX files were built, then creates the GitHub release with this version's
changelog section as notes. Re-running never replaces already uploaded files.

## Module Map

### Extension (`packages/extension-core/src`)

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
- `src/languageClientLifecycleController.ts`
  Owns the active and pending client handles, queued restart loop, bounded shutdown, lifecycle counters, and slow-start
  notice state. Keep it free of VS Code imports so startup and shutdown races remain covered by fast unit tests.
- `src/runtimeSourceRootDiscoveryController.ts`
  Serializes asynchronous runtime source-root probes, caches completed folder/configuration keys, drains distinct
  multi-root inputs without parallel subprocesses, rejects results superseded by explicit invalidation, and keeps
  runtime-derived roots in memory without writing user settings.
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
  builds, or `PATH`.
- `src/interpreterDiscovery.ts`
  Finds Sage runtime candidates and resolves quick-pick selections into configuration updates. Keep path probing
  and selection handling here so `extension.ts` only applies settings and reports the result.
- `src/documentationRequest.ts`
  Normalizes documentation payloads into a render-friendly shape.
- `src/docsPanel.ts`
  Owns the documentation webview lifecycle.
- `src/documentationFallback.ts`
  Keeps the no-documentation action labels and command mapping in one place. Use it when adding diagnostics or recovery
  actions for documentation misses.

### Language server and index


- `crates/sage-ls/src/main.rs`
  Composition root for `tower-lsp` capabilities, request dispatch, shared server state, and internal `sage.__rust.*`
  execute commands. Keep navigation, text conversion, editor features, and background work in their modules below.
- `crates/sage-ls/src/navigation.rs`
  Owns definition, declaration, type-definition, and implementation responses, including navigation caching, verified
  live ranges, `LocationLink` capability negotiation, and ordered candidate links. Cache identity includes hover plus
  each navigation role; omitting that flavor can make sequential definition/declaration/implementation requests reuse
  the wrong target.
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

Index (`crates/sage-index/src`):

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
  `navigation_targets.rs` proves exact `.pxd`/`.pyx` class and owned-method siblings before selecting a
  declaration/implementation role; it loads only the two sibling files and requires owned-method signatures to match.
  Do not widen it to name-only or cross-stem matches.
- `crates/sage-index/src/sage_specs/`, `source_paths.rs`, `symbol_support.rs`, and `syntax_support.rs`
  Hold static Sage mappings and shared path, ranking/deduplication, and syntax primitives. The Sage catalog is split into
  export, owner-module, method, and alias tables so updates can be reviewed and debugged independently; `mod.rs` owns
  cross-table invariants.
- `crates/sage-index/src/tests.rs` and `crates/sage-index/src/tests/`
  Keep shared fixtures/helpers in the test root and group cache, reconciliation, parsing, diagnostics, import resolution,
  completion, editor-query, and Sage-navigation regressions by domain.
- `crates/sage-index/src/bin/sage-index-bench.rs`
  Measures cold release indexing against a local Sage source checkout. Use release mode for performance validation.
