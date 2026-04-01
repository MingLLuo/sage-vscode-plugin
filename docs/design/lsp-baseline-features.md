# LSP Baseline Features

## Purpose

This note records the minimum feature set that should make the plugin feel like a real language tool rather than a
demo extension.

## Baseline

The current baseline now includes:

- hover
- completion
- definition
- signature help
- semantic tokens for Sage namespaces, constructors, decorators, readonly library values, and preparser declarations
- document symbols
- workspace symbols
- references
- rename
- low-noise unresolved-import diagnostics
- conservative syntax diagnostics for Python and `.sage`
- custom documentation requests
- dotted singleton-member resolution for common Sage patterns such as `graphs.PetersenGraph`
- member completion for statically understood singleton APIs
- completion responses serialized as concrete LSP `CompletionItem` objects under real clients
- Python-like `.sage` parsing through the AST path when no preparser assignment is present
- hybrid `.sage` parsing that merges preparser-aware top-level extraction with AST-driven class/method/import analysis
- saved `.sage` modules participating in workspace indexing instead of only live open-document parsing
- cached symbol and member resolution for indexed modules to reduce repeated definition and documentation lookup cost
- persistent module-cache reuse for indexed source roots, with automatic invalidation when file size or mtime changes
- open-document overlay caching so repeated requests against unchanged editor buffers reuse parsed records instead of re-parsing the same text

## Scope

- These features are primarily static and index-driven, with runtime fallback for documentation, definitions, and
  signatures when static resolution misses Sage runtime objects.
- Python-heavy `.sage` files now stay on the richer Python AST path unless they use Sage preparser assignment syntax
  such as `R.<x, y> = ...`; that keeps class, method, import, and assignment tracking closer to ordinary Python-editor
  behavior for mixed Sage/Python projects.
- Preparser-heavy `.sage` files now keep their generator declarations from the loose parser while still regaining AST
  structure for the rest of the file through a sanitized hybrid parse.
- Static resolution now includes class-body imports, singleton instance aliases, and dotted member traversal so common
  Sage generator objects remain navigable even when the selected Sage runtime cannot answer introspection requests.
- Indexed-module resolution caches currently target repeated symbol and member lookups; this is a first step toward a
  broader split between hot document state and colder library/workspace indexes.
- Persistent cache writes are best-effort. If the preferred cache directory is unavailable, the server falls back to a
  temporary cache location or disables persistence without breaking analysis.
- Open documents now act like a hot overlay above the colder workspace/library index, which is closer to how modern
  language tools separate live editor state from background index state.
- Server request wiring now routes common documentation, definition, overlay-refresh, and diagnostics flows through
  shared helpers so the handler layer stays thinner as more LSP features accumulate.
- Saved and watched workspace files now refresh the indexed module graph incrementally, including correct recomposition
  of mixed `.pyx`/`.pxd` native modules when only one component changes or disappears.
- Batched workspace-change application now updates that incremental index in one pass per event burst, which reduces
  repeated cache persistence and cache-reset churn under larger watcher batches.
- They are designed to stay predictable and low-noise while still remaining usable against real Sage installations.
- Diagnostics are intentionally conservative and currently focus on unresolved imports plus syntax errors that can be
  validated safely without pretending to approximate a full Python or Cython type checker.

## Follow-up Areas

- `.sage` source mapping still needs to feed more of the diagnostics and navigation surface.
- Semantic tokens should expand beyond the current baseline into more classifiable Sage runtime objects.
- Code actions, inlay hints, richer diagnostics, and deeper runtime-aware analysis remain open work.
- Library-index persistence and incremental background indexing remain open performance work.
