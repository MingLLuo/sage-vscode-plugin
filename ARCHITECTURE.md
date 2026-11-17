# Architecture

## Runtime Layers

- `packages/extension-core` owns VS Code activation, configuration, lifecycle, commands, external-source views, and
  user-facing status and documentation surfaces.
- `crates/sage-ls` is the primary language server. It owns LSP transport and request handling, open-document overlays,
  editor feature conversion, background index jobs, and the optional persistent Sage runtime-documentation worker.
- `crates/sage-index` owns workspace and Sage source discovery, Python/Sage/Cython source analysis, strict symbol
  resolution, documentation records, incremental refresh, and the root-aware SQLite cache.
- `packages/syntax-pack` owns generated syntax assets so grammar changes remain independent of runtime analysis.

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

- All projects share one SQLite database per index format, exclude list, and Cython switch. Rows are scoped by source
  root, so different Sage checkouts coexist without mixing symbols and a tree used by several projects is stored once.
  Databases unused for 30 days are deleted before the server starts.
- Initialization publishes an empty index immediately. A single index actor hydrates and reconciles the persistent
  cache on a dedicated worker. Save and watcher notifications enqueue coalesced path events; they never wait for indexing.
- Readers load immutable `ArcSwap` snapshots. Indexed files are shared through `Arc`, and one writer publishes completed
  generations without holding a reader lock. Linked-file prewarming reads files when its queued job executes, so a delayed
  discovery cannot restore a pre-save version.
- Open-document updates take a synchronous lock before their first await and replace versioned `Arc` snapshots. Heavy
  hover/completion/token work uses a bounded blocking lane; references have a separate lane and cooperative cancellation.
  Diagnostics are debounced for 200 ms and only publish against the current document version. Semantic tokens are cached
  per document snapshot, with range responses filtered from that cache.
- Syntax diagnostics use ERROR/MISSING node ranges and map generated Sage coordinates back through operator and generator
  rewrites. Comment/string carets and continued exponents do not create standalone errors. Function folding and call-hierarchy
  bounds share logical-line and string handling with lexical analysis, including multiline signatures.
- Shutdown stops queued indexing and diagnostics, and transport exit has a bounded runtime shutdown even with stdin open.
  Runtime documentation uses replaceable sessions, a bounded request lane and independent process control, so slow imports
  cannot block shutdown or reconfiguration. Unix workers own a process group; Windows workers are assigned to a kill-on-close
  Job Object. Paths are canonicalized without Windows `\\?\` prefixes so returned URIs open in VS Code.
- Runtime docs use a private protocol descriptor with echoed request ID and symbol. Python/native stdout is diverted away
  from the protocol. Isolated Python imports Sage before appending workspace paths; per-process cache directories are private
  and removed after reaping. Missing symbols have a 30-second negative cache, startup failures back off, and positive/negative
  cache storage is bounded by entry count and bytes.
- Clients validate the selected server with a cancellable `--version` probe before starting LSP. Invalid executables require
  a configuration change or manual retry; other crashes retain bounded automatic recovery. Deactivation shares one four-second
  deadline across startup, client stop and runtime-source discovery.
- Open documents override on-disk index rows. External Sage sources remain read-only and are mapped back to their
  physical files by the extension client.
- A full Sage installation is optional for static editing. When a compatible runtime is available, it enriches docs and
  execution without becoming a prerequisite for indexing or protocol tests.

## Maintained Boundaries

- Rust is pinned; Node.js 22.9+ and npm 11+ use minimum compatibility ranges, with Node 22 and 26 covered in CI.
  CI runs on macOS, Linux, and Windows with repository-local fixtures plus a sparse checkout of the latest public Sage
  source; it must not depend on maintainer-private paths or an installed Sage runtime.
- Local release gates additionally validate the latest discovered Sage source checkout, real-file behavior, and
  persistent LSP latency.
- Each platform package carries its own `sage-ls` (static musl on Linux). The extension is a workspace extension, so in
  Remote SSH, WSL, and containers it runs next to the files.
- Notebook integration remains a separate milestone; it must not weaken the source-navigation contract.
