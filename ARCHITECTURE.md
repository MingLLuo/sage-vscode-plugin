# Architecture

## Runtime Layers

- `packages/extension-core` owns VS Code activation, configuration, lifecycle, commands, external-source views, and
  user-facing status and documentation surfaces.
- `crates/sage-ls` is the primary language server. It owns LSP transport and request handling, open-document overlays,
  editor feature conversion, background index jobs, and the optional persistent Sage runtime-documentation worker.
- `crates/sage-index` owns workspace and Sage source discovery, Python/Sage/Cython source analysis, strict symbol
  resolution, documentation records, incremental refresh, and the root-aware SQLite cache.
- `packages/syntax-pack` owns generated syntax assets so grammar changes remain independent of runtime analysis.
- `packages/sage-lsp` is the legacy Python implementation retained for compatibility regression tests and migration
  reference. It is not the production analysis authority.

## Navigation Contract

- A single jump is returned only for a high-confidence, identity-checked target.
- When ownership cannot be proved but multiple useful definitions exist, the server returns ordered `LocationLink`
  candidates and explanatory documentation instead of choosing one arbitrarily.
- A lone weak candidate is documentation/completion evidence, not permission to force a jump.
- References, rename, and call hierarchy require the same high-confidence identity. Import aliases keep their local
  binding domain separate from the imported source-name domain.
- Strings, comments, sibling lexical scopes, overwritten bindings, and nested call-hierarchy bodies must not create
  high-confidence navigation.

## Index and Lifecycle

- Cache namespaces are derived from normalized workspace/source roots and parser options, allowing different Sage
  checkouts to coexist without mixing symbols.
- Startup hydrates the persistent cache first and reconciles source changes in the background. Save and watcher events
  use incremental path refreshes.
- Open documents override on-disk index rows. External Sage sources remain read-only and are mapped back to their
  physical files by the extension client.
- A full Sage installation is optional for static editing. When a compatible runtime is available, it enriches docs and
  execution without becoming a prerequisite for indexing or protocol tests.

## Maintained Boundaries

- Rust is pinned; Node.js 22.9+ and npm 11+ use minimum compatibility ranges, with their runtime pairing checked
  against npm's own Node.js engine declaration and Node 22 and 26 covered in CI.
  CI uses repository-local fixtures plus a sparse checkout of the latest public Sage source and must not depend on
  maintainer-private paths or an installed Sage runtime.
- Local release gates additionally validate the latest discovered Sage source checkout, real-file behavior, and
  persistent LSP latency.
- Notebook and remote execution remain separate milestones; they must not weaken the source-navigation contract.
- Removal of the legacy Python server requires explicit parity evidence and a deliberate migration decision.
