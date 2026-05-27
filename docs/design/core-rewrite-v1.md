# Core Rewrite V1

## Purpose

This slice starts the Sage VS Code plugin core rewrite without replacing the working parser, resolver, or `pygls`
request surface in one step. The goal is to put the expensive, frequently queried parts of the language server behind
stronger storage and analysis boundaries while preserving current LSP behavior.

## Scope

- Keep `sage_lsp.index` as the public compatibility facade.
- Keep `WorkspaceIndex` as the request-facing index API for now.
- Move persisted lightweight summaries from JSON-first storage to a SQLite-first store with a JSON fallback.
- Add a narrow Jedi bridge for local Python context completion.
- Merge Jedi completion results with existing static Sage completions only for non-dotted completions.
- Keep dotted Sage member completion on the existing static resolver path.

## SQLite Summary Store

`sage_lsp.index_db.IndexDatabase` owns the new summary-cache database. It persists:

- file identity: path, module name, mtime, and size
- serialized module summary payloads
- symbol names and containers for indexed query paths
- export names for import-candidate and global symbol lookups
- schema and completeness metadata

The workspace index writes SQLite summaries first and falls back to the previous JSON summary file when the database is
unavailable. Reads follow the same order so existing cache directories remain usable across incremental migration.
Cold global queries ask SQLite for matching module, symbol, container, and export rows before loading the whole summary
cache into memory.

## Jedi Bridge

`sage_lsp.jedi_bridge.JediBridge` is intentionally small:

- import Jedi lazily and degrade to no results when unavailable
- ask Jedi only for the open document's local Python context
- filter by the already computed completion prefix
- map Jedi completion types into LSP completion kinds
- return plain dictionaries so server serialization stays centralized

The bridge does not resolve Sage runtime symbols, replace static import resolution, or take over dotted member
completion.

## Debugging Notes

- Use `sage/__debug/indexStatus` to confirm summary-cache state before investigating resolver failures.
- Use debug-level LSP traces to see whether a completion request took the static, dotted-member, or merged local-context
  path.
- SQLite files are named `workspace-summary-<digest>.sqlite` next to the existing `workspace-index-<digest>.json` cache
  snapshot.
- If SQLite cache writes fail, the JSON fallback remains the compatibility path; this should show up as missing SQLite
  files in cache-focused tests, not as changed LSP responses.

## Task Links

- `LSP-030`: SQLite summary-index persistence.
- `LSP-031`: Jedi local-context completion bridge.
- `QA-007`: regression coverage for core rewrite v1 behavior.
