# Sage VS Code Rust LSP V2

## Scope

This design record tracks the V2 rewrite milestone that makes Rust the primary language-server runtime. The first milestone proved the performance skeleton; the current stabilization slice restores the most important editing behaviors while keeping Pyright sidecar integration out of scope.

The Python LSP remains in the repository until deletion is explicitly approved. The extension launch path now selects `sage-ls` first, while legacy Python launch helpers stay available for compatibility tests and migration reference.

## Runtime Boundary

- `crates/sage-ls` owns the LSP process, request dispatch, open-document overlays, semantic-token encoding, and custom status commands.
- `crates/sage-index` owns file discovery, Sage/Python/Cython source scanning, source preprocessing, SQLite persistence, deferred FTS storage, symbol lookup, and documentation cache status.
- VS Code starts the Rust binary through `sage.languageServer.rustPath`, `SAGE_LS_PATH`, a repository-local `target/*/sage-ls`, or `sage-ls` on `PATH`.
- Pyright is represented in the initialization payload and status model, but it is not active in this milestone.
- Runtime documentation uses a persistent Sage `-python` JSONL worker when runtime introspection is enabled. Hover keeps this path non-blocking through cached/prefetch behavior; explicit documentation requests may wait up to the worker timeout for fresh runtime docs. Successful explicit runtime lookups are written back into the SQLite runtime docs table so later sessions can reuse them.

## Indexing Model

The Rust index scans configured workspace roots, Sage source roots, and analysis extra paths with `ignore` and Rayon. It extracts:

- Python and Cython class/function declarations.
- Sage preparser generator declarations such as `R.<x, y>`.
- Import binding names with source-module metadata for `from x import y` resolution.
- Module and symbol docstrings when statically available.

The cache is SQLite with files, symbols, docs, and FTS5 docs tables. Cache filenames are keyed by normalized source roots, excludes, and native parsing mode, so changing Sage checkouts or workspace roots creates an independent cache instead of mixing rows from unrelated source trees. File fingerprints use file size plus modified time since the Unix epoch so cache rows are stable across process lifetime. The VS Code extension runs conservative cache maintenance before starting `sage-ls`, keeping the newest namespaces and pruning old or over-budget SQLite databases under its own `rust-index-v2` storage. Long-lived development machines can also inspect these root-aware databases with `npm run cache:status`; manual cleanup is dry-run by default through `npm run cache:prune:dry-run` and only deletes matching `sage-index-<digest>.sqlite` databases plus SQLite sidecars when `--yes` is explicitly supplied to `scripts/cache-maintenance.mjs`.

Full rebuilds rewrite cache tables in one transaction and materialize the Sage export/method fast caches from the in-memory symbol map, avoiding a second pass of SQLite lookups after inserting the full Sage tree. Save and watcher events use file-level upsert/delete through `refresh_paths(changed, deleted)` so one batch increments the generation once; those smaller incremental refreshes keep the database-backed materialized-cache path. Warm startup hydrates file shells and cache counters first; symbol, document-symbol, completion, workspace-symbol, and hover documentation queries lazily read matching rows from SQLite until the background reconcile finishes.

## Request Behavior

- `initialize` returns after cache hydration, installs capabilities, and schedules background reconcile.
- `textDocument/hover`, `definition`, `completion`, workspace symbols, document symbols, references, rename, static signature help, inlay hints, diagnostics, and semantic tokens are served by Rust.
- Hover ranges use the request document range. Definitions prefer import-source matches before falling back to same-name global symbols.
- Diagnostics are conservative: Python-like files use parser errors, Cython files avoid noisy early false positives, and `.sage` keeps legal preparser assignments such as `R.<x, y>`.
- Semantic tokens skip strings/comments and preserve the existing legend while adding more accurate spans for Sage namespaces, constructors, decorators, and preparser generators.
- `workspace/executeCommand` exposes internal `sage.__rust.indexStatus`, `sage.__rust.docsStatus`, `sage.__rust.rebuildIndex`, and `sage.__rust.getDocumentation` commands. The visible VS Code commands remain `Sage: Show Index Status`, `Sage: Show Docs Status`, and `Sage: Rebuild Index`.
- Runtime documentation first uses static index docs, then persisted runtime docs, then the live runtime worker. If only placeholder docs are available, hover triggers a non-blocking runtime prefetch, while `Sage: Show Documentation` waits briefly for the persistent Sage worker before falling back.

## Performance Baseline

Release benchmark on a local Sage source checkout from May 24, 2026:

- Files indexed: 3794
- Symbols extracted: 119848
- Offline docs: 64699
- Parse-only time: 303 ms
- Cold full rebuild internal index time: 385 ms
- Cold full rebuild end-to-end benchmark time: 1487 ms
- Warm SQLite hydrate time: 0 ms
- Persistent LSP hot-path queries on the internship Sage-heavy file: `PolynomialRing` hover/definition `31/31 ms`,
  `.rank` hover/definition `40/37 ms`
- Persistent LSP inlay hints on the internship Sage-heavy file: `4 ms` for the first 320 lines

Debug builds are suitable for local development but are substantially slower; performance validation should use
`npm run test:performance -- --skip-workbench` or `cargo run --release -p sage-index --bin sage-index-bench`. The npm
performance gate isolates `XDG_CACHE_HOME` by default so cold rebuild measurements are not affected by old SQLite cache
fragmentation.

## Follow-Up Work

- Start Pyright as a sidecar and route `.sage.py` overlays through source maps.
- Expand runtime docs writeback from the dedicated runtime docs table into the main docs/materialized cache rows where a precise indexed symbol target exists.
- Replace the current conservative Cython scanner with a fuller parser once native navigation needs exceed declarations/imports/includes.
- Populate and query docs FTS asynchronously; cold startup currently prioritizes direct docs table lookups over blocking FTS rebuilds.
- Evaluate Python LSP removal only after Rust V2 reaches explicit parity and deletion is separately approved.
