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

- `src/configuration.ts`
  Reads VS Code settings into the stable `SageSettings` model.
- `src/workspaceDiscovery.ts`
  Determines which workspace roots should be indexed and augments them with Sage roots derived from the selected
  runtime when `sage.analysis.sourceRoots` is left empty.
- `src/languageClient.ts`
  Starts the Rust `sage-ls` process, watches Sage/Cython document types, and sends command-backed documentation/status
  requests while suppressing client-library auto-restarts during extension-managed shutdown and restart cycles.
- `src/executionPlan.ts`
  Builds shell-safe run commands and REPL load commands from extension settings, including optional cleanup of
  generated `.sage.py` files after standalone terminal runs or managed-REPL file loads.
- `src/serverRestart.ts`
  Limits language-server restarts to configuration changes that actually affect analysis behavior and keeps close/restart
  policy explicit during managed shutdown.
- `src/serverLaunch.ts`
  Resolves the Rust language-server binary from `sage.languageServer.rustPath`, `SAGE_LS_PATH`, local `target/*`
  builds, or `PATH`. The legacy Python launch resolver remains for migration tests.
- `src/documentationRequest.ts`
  Normalizes documentation payloads into a render-friendly shape.
- `src/docsPanel.ts`
  Owns the documentation webview lifecycle.

## Key Language Server Modules

Rust V2:

- `crates/sage-ls/src/main.rs`
  Wires `tower-lsp` capabilities, open-document overlays, semantic tokens, hover, definition, completion, workspace
  symbols, document symbols, and internal `sage.__rust.*` execute commands consumed by the extension's user-facing
  status, rebuild, documentation, and UX self-check commands.
- `crates/sage-ls/src/runtime_docs.rs`
  Owns the opportunistic persistent Sage runtime documentation worker. It starts only when runtime introspection is
  enabled and a usable Sage/Python runtime is configured; otherwise docs status reports a degraded or disabled state
  while static docs keep working.
- `crates/sage-index/src/lib.rs`
  Scans configured roots, extracts static symbols and docstrings, preprocesses `.sage` source, persists SQLite/FTS
  cache data, falls back to a temp cache if the configured cache path is unusable, and exposes status/query APIs used by
  `sage-ls`.
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
npm install
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
npm run test
npm run test:vsix-contents
npm run test:vsix-package
npm run test:vsix-install
npm run test:cache-maintenance
npm run test:repo-hygiene
npm run test:native-smoke
npm run test:product-readiness
npm run test:performance
npm run test:lsp-latency
npm run test:extension-host
npm run test:release
npm run test:full
```

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
  `npm run test:vsix-install` green; `package:vsix` stages the current-platform release `sage-ls` before archive checks.
- Release-candidate changes that do not need the desktop extension host should keep `npm run test:release` green. This
  runs Rust tests, clippy with `-D warnings`, TypeScript lint, the full non-desktop test suite, VSIX content/package
  install smoke, release index performance, persistent LSP latency, the real-file Sage-heavy smoke, and
  `git diff --check`.
- Native Sage library work should also keep `npm run test:native-smoke` green when a local Sage checkout is present.
- Extension-host behavior that depends on the real VS Code client lifecycle should also keep `npm run test:extension-host`
  green.
- The extension-host smoke suite now covers hover, definition, completion, references, rename, document/workspace
  symbols, native Cython navigation, managed restart stability, imported-helper save refresh, and optional native Sage
  source-tree lookups.
- Use `npm run test:full` when you need the whole repository suite plus the extension-host smoke test in one command.
