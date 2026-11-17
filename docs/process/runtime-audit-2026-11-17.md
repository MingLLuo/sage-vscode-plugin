# Claude continuation audit — 2026-11-17

## Reconciled starting state

This audit compares the supplied Claude plan with repository state and local Claude session records. The plan is historical
context, not evidence that its milestones have shipped. Work remains on branch `production-readiness`; the continuation
has not been committed, pushed, installed into the user's normal VS Code profile, or published.

- M0 was committed as `035edc1..49f58cd`.
- The plan's final entry was stale: tower-lsp-server migration was already committed in `25597f9`; tree-sitter migration
  was committed in HEAD `aee9517` (tree-sitter 0.27, Python grammar 0.25).
- Claude's M1 session `337fd917-a2d3-422d-bee5-10f55bfc9720` had reached performance probing after the migrations,
  then hit its five-hour limit at `00:56:37Z`. After the old probe was stopped, its background-task notification triggered
  another attempt at `01:27:51Z`, which hit the same limit. No resumed source editing appears in that session.
- The M3 session `95e2dbdd-d927-4280-9199-534c77e97e0c` ended after code reading with a rate-limit response at
  `2026-11-17T00:49:59Z`; the planned client redesign was not implemented.
- The M4 session `756f63fe-cf98-405b-86d8-48c26d5ba2af` ended with a rate-limit response at `00:50:16Z`, leaving
  six modified packaging scripts and four untracked helpers. Those changes were retained and completed/integrated here.
  This describes the recorded sessions, not the live status of the Claude application.
- The old M1 performance probe and its server were still sleeping after about 40 minutes. Its sampler exit listener was
  registered only after the long reference queries, allowing an already-delivered exit event to be missed. After verifying
  their exact PID/parent/command, the stale probe and its server were terminated; the Claude application was left running.

## Implemented and locally checked

### Server responsiveness and document consistency

- Synchronous, versioned immutable document snapshots; stale changes are rejected. A cached line table handles UTF-16
  positions and LF, CRLF, and CR. The randomized edit test applies 1,000 Unicode edits against an independent character model.
- `ArcSwap` index snapshots with shared indexed files, an in-memory generation accessor, and a single coalescing writer.
  Startup hydration/reconciliation is off the initialize path; save/watcher notifications enqueue without waiting.
  Last-event-wins path queues prevent overlapping refreshes from being discarded by generation races.
- Linked-document prewarming is debounced and version-checked. Only paths are queued; the actor reads their latest contents
  when executing the job, preventing delayed preloads from overwriting newer saves or reviving deleted files.
- Heavy hover, completion, navigation, token, outline, and reference work moved off the transport workers. Reference scans
  have a separate bounded lane, cooperative cancellation, and stale-document checks. This does not yet cover every LSP handler.
- Diagnostics debounce for 200 ms and publish the current version only. Token results are cached per immutable document;
  range results filter that cache. Outline and token positions use precomputed line offsets.
- Warm symbol cache entries use shared immutable records and independently owned maps per snapshot. The final latency
  gate exposed lost prewarming after background publication; a regression test now checks retention, shared allocation and
  independent invalidation.
- Import resolution filters candidates by module before cloning and releases its shared cache lock before filtering/deduplication.
  Lexical-scope scans index function body lines once instead of rescanning the full file for each identifier.
- Runtime docs locks recover from poisoning; syntax prefix slicing checks character boundaries.
- `--version` works without starting a runtime. Tokio workers are bounded by `min(4, cpus)`; exit finishes with stdin open,
  and an exit without shutdown returns status 1. Index/diagnostic queues are stopped on shutdown.

### Client and packaging

- A no-folder controller supports standalone `.sage` files; configuration uses an explicit resource/null scope, eliminating
  the resource-scope warnings exposed by the new editor regression.
- Explicit Rust path wins over the environment override; packaged binaries win over development builds; production
  mode ignores repository build artifacts. All six native platform/architecture directory names are resolved.
- A failure in `afterStarted` stops the partially started client before reporting failure.
- Target selection is forwarded through the entire package build. Per-target VSIX names/manifests, binary checksums,
  metadata and third-party notices are validated; repeated packaging is byte deterministic.
- Six native binaries map to **eight** VSIX target identifiers, including two Alpine aliases. The plan's count of seven
  was inconsistent with its listed platform/architecture pairs. Linux builds select musl/mimalloc; Windows requests static CRT.
- Added native packaging workflow, executable version check, isolated editor installation, Linux editor regression and tag
  release artifacts. The original CI reads the pinned Rust version through rustup rather than repeating it in YAML.
- The historical `doctor:mac` entry point now resolves host-specific binaries and artifact names.

The workflow labels and artifact actions were checked against the
[GitHub runner documentation](https://docs.github.com/en/actions/reference/runners/github-hosted-runners),
[upload-artifact v7](https://github.com/actions/upload-artifact/releases/tag/v7) and
[download-artifact v7](https://github.com/actions/download-artifact/releases/tag/v7).
The workflow is implemented but has **not run remotely**. Local `gh` was unauthenticated. No Windows, Linux or Intel Mac
success is inferred from passing macOS arm64 checks.

## Measurements

See [performance comparison](../perf/README.md) and [raw baseline](../perf/baseline-2026-11-17.json).
Both runs used the same darwin-arm64 host, eight CPUs, and local Sage tree.

| Case | Before | After |
| --- | ---: | ---: |
| Hover after saves during cold indexing | 3,125 ms | 36.4 ms |
| Large-file full semantic tokens, p95 (5 requests) | 1,158 ms | 73.2 ms |
| Large-file document symbols, p50 | 333 ms | 44.6 ms |
| matrix references (1,898 locations) | 28.8 s | 4.5 s |
| Hover during matrix references | 28.9 s | 1.5 ms |
| QQ references (2,299 locations) | 6.2 s | 1.7 s |
| Hover during QQ references | 6.0 s | 4.2 ms |
| Final edit to version-101 diagnostics | measurement corrected | 209 ms |
| Cold-index / post-rebuild RSS | 476 / 820 MB | 627 / 694 MB |

Cached token p50 is not parsing time. The first large-file full request took 73.2 ms and the request after 100 edits took
68.5 ms. Memory is not solved: cold-index RSS increased, and both cold/rebuild values exceed the 400 MB target. Matrix references
still exceed the 2 s target; QQ now meets it. These are single-host observations, not cross-platform performance guarantees.

Golden snapshot comparison with pre-continuation HEAD: **3,921 files, 126,514 symbols, 273,949 semantic spans, 78 diagnostics,
zero differences**. Normal indexed discovery includes 3,842 files because it applies different traversal filters. The
snapshot does not cover every dynamic navigation result; identity and protocol regression tests provide separate coverage.

## Validation record

The final release, editor and packaging gates passed on macOS arm64:

- `SAGE_SOURCE_ROOT=../sage/src npm run test:release`: 215 sage-index tests, 92 sage-ls tests, clippy with warnings denied,
  TypeScript lint and 219 extension tests, fixture/real-Sage workbench checks, LSP navigation/shutdown/concurrency/Unicode/
  highlighting, target selection, generated assets, binary/VSIX contents, reproducible packaging, isolated VSIX installation,
  cache/repository/product checks, reference export, performance/latency gates and real-file smoke.
- `npm run test:extension-host`: all **five** scenarios exited 0 with the extension log checks passing — no-folder,
  first-run, plain Python, full functional smoke with real Sage sources, and multi-root.
- `npm run package:vsix`: produced `dist/sage-vscode-extension-0.1.0-darwin-arm64.vsix` (4,641,494 bytes).
- The final performance baseline completed after the editor/package checks, without overlapping builds/tests.
- `git diff --check`: passed. Artifact hash and command results are recorded in
  [validation-2026-11-17.json](../perf/validation-2026-11-17.json).

The additional environment doctor returned `usable-with-warnings`: all required package checks, VS Code CLI and Sage
source-root checks passed, but `sage.all` did not import within its 30-second probe budget. A separate 45-second probe
also timed out; periodic Python stacks moved from p-adic extensions through combinatorics to matrix-group imports,
showing continued module loading rather than proving a deadlock. A subsequent probe with a 120-second ceiling completed
`from sage.all import ZZ; ZZ(2) + ZZ(2)` successfully in **21.6 seconds**. Repeating the ordinary doctor then returned
**`ready`**, with every check passing. These observations establish working arithmetic after startup; they do not establish
stable cold-import latency or complete runtime-documentation worker behavior.

These are local gates, not proof of all acceptance criteria below. The editor runs used isolated user-data and extension
directories; the user's normal extension installation was not changed.

## Second continuation: runtime lifetime and startup recovery

Implemented after the first checkpoint above:

- Replaced the runtime-doc worker's shared process lock design with versioned sessions. Configuration replacement cannot
  wait for old IO and old work cannot populate a new session. One asynchronous semaphore bounds blocking requests, prefetch
  coalesces at most 16 symbols, and caches retain at most 256 entries / 16 MiB. Negative results expire after 30 seconds.
- Shutdown and cancellation can terminate a worker independently of the request lock. Unix process groups include descendants;
  Windows Job Objects terminate the worker tree on close. Cache directories are unique, private, and removed after the process
  is reaped. HOME is deliberately preserved because prior real-Sage checks showed that replacing it breaks GAP configuration.
- Protocol replies must echo both request ID and symbol. A private descriptor carries JSON; Python and native stdout go to
  stderr. Frames are limited to 8 MiB. Python uses isolated mode, and configured source paths are appended only after importing
  Sage. Unknown symbols stay a normal miss; failures trigger exponential startup retry backoff.
- The background Sage import budget is now 60 seconds, matching the slow imports observed during the first checkpoint, while
  the existing real-Sage smoke still verifies responsive cold hover and successful eventual docs/navigation.
- Added a bounded, cancellable server version handshake. Missing, incompatible or invalid executables do not enter a restart
  loop. A single startup-failure notification offers logs, restart and settings; logs include the attempted executable.
  Correcting configuration resumes the failed controller. The exact executable that passed preflight is used for that start.
- Runtime-source discovery is aborted on invalidation/deactivation. Probe timeouts and cancellation terminate Unix process
  groups (Windows uses taskkill). Client shutdown uses one shared budget instead of spending it separately on start and stop;
  extension-wide deactivation is capped at four seconds. Configuration edits avoid starting unused folder controllers.
- Added real VS Code `startup-failure` coverage: an invalid path launches no LSP client, stops recovery, then automatically
  restores document features after the path is corrected. The editor suite now contains six scenarios.

Targeted checks passed: Rust tests (including wrong IDs/symbols, noisy stdout, negative cache, cancellation, process descendants,
reconfiguration and backoff); 227 extension unit tests; real Sage docs and source navigation; real editor startup-failure recovery.
Consolidated release, all six editor scenarios and packaging passed. The checkpoint contains 313 Rust tests and 227
extension tests; the VSIX is 4,659,784 bytes. See [validation-runtime-2026-11-17.json](../perf/validation-runtime-2026-11-17.json).
Artifact hashes in earlier records describe their historical checkpoints, not subsequent replacements of the same filename.

## Third continuation: syntax diagnostics and function boundaries

- Syntax diagnostics now traverse tree-sitter ERROR/MISSING nodes rather than reporting every parse failure at 0:0.
  A private preprocessing map accounts for exponent/range/empty-index rewrites, generator-assignment prefixes and inserted
  generator bindings, including multiline RHS and Unicode byte columns. Public preprocessing output remains compatible.
- Incomplete-exponent messages are restricted to actual `.sage` parse errors and code positions. Comments, multiline
  strings, Python/Cython XOR and valid continued Sage exponents no longer trigger the unconditional caret check. Multiple
  independent parser errors are retained, with a maximum of 100 diagnostics. Diagnostic parsing normalizes CR/CRLF/LF;
  this does not establish lone-CR support in every index feature.
- Function/body folding now shares the indexer's logical-line, header and string handling. Multiline signatures and
  column-zero text inside triple-quoted strings retain the entire function body, so call hierarchy includes later calls.
- Full-tree comparison: all 126,514 symbols and 273,949 semantic spans remain identical over 3,921 files. Diagnostic count
  changes from 78 to 50 across 30 files: 29 old caret reports disappear (including a valid continued Cython XOR expression),
  and `partial_cube.py` retains two existing Python XOR warnings previously hidden by the early return. Each affected file
  and the old line are recorded in [snapshot-diagnostics-2026-11-17.json](../perf/snapshot-diagnostics-2026-11-17.json).
- Regression coverage includes comment/string carets, continued exponents, separate syntax errors, CR lines, Unicode and
  generator-coordinate mapping, multiline call-hierarchy function ownership, and dedented standalone strings.
- Final validation passed: full release gate, 321 Rust tests on the final source, Clippy, 227 extension unit tests, all six
  real VS Code scenarios, and VSIX packaging (4,674,997 bytes). The final extra string-boundary regression was checked with
  another full Rust test/Clippy run after the release gate had started; the editor/package checks use the final source.
  See [validation-diagnostics-2026-11-17.json](../perf/validation-diagnostics-2026-11-17.json).
- The new isolated performance run records large-file outline p95 **44.4 ms**, folding p95 **10.2 ms**, final-edit diagnostics
  **280.3 ms**, semantic full first request **63.2 ms**, and hover/completion during typing p95 **74.4/13.0 ms**. Removing the
  false early caret return means diagnostics now actually parse `generic_graph.py`, rather than stopping in its docstring.
  Cold/rebuild RSS remains **648.1/793.1 MiB**, and matrix references **3.70 s**, so those acceptance targets still fail.
  Results: [baseline-diagnostics-2026-11-17.json](../perf/baseline-diagnostics-2026-11-17.json).

## Remaining work — production readiness is not complete

1. **M1:** finish moving synchronous handlers to bounded query execution; propagate cancellation/errors consistently;
   semantic full/delta, completion parse/prefix caches, SQLite connection reuse, unchanged-import prewarm skipping;
   foreground runtime docs/definition latency while a cold worker imports Sage (startup budget is up to 60 seconds);
   progress/metrics and panic reports; full-parser CR support;
   one-hour soak and complete cancellation coverage. Active detached index disk work stops publishing after shutdown but
   is not cooperatively cancelled at every filesystem step.
2. **M2:** shared library/workspace cache design (or explicitly evaluated fallback), format version and migration/GC,
   corruption/kill recovery validation; canonical URI response handling and RST hover rendering. The 205 MB combined database
   remains duplicated across workspace/source combinations; this continuation does not claim the storage redesign.
3. **M3:** environment state machine and Sage detection;
   activation/watcher scoping, terminal/REPL behavior, settings/commands, walkthrough, logging, localization and esbuild.
   No-folder support, startup preflight, failure actions and lifecycle/probe fixes above are only a subset of the client milestone.
4. **M4:** execute native CI, repair actual platform failures and require green cross-platform checks; add calibrated
   performance regression thresholds. The existing permissive cross-platform Rust job was not promoted to a required gate
   without evidence. A configured workflow is not a verified deliverable on another OS. `gh auth status` was rechecked during
   this continuation and reports no authenticated GitHub hosts; no remote workflow was dispatched.
5. **M5:** only after the above, align extension/root versions with Rust 0.2.0, update release-wide documentation and changelog,
   repeat acceptance tests, and perform the clean-install/remote-host walkthrough. The extension remains preview 0.1.0.

## Review of the continuation (Claude, 2026-11-17)

Three read-only reviews of the uncommitted continuation found new defects. The continuation was committed as area
checkpoints (`6131de8`..`4ff23fb`) and each defect was fixed in its own commit with a regression test:

- CRLF sources with non-ASCII text could panic in `LexicalScopeMap` (offsets assumed one-byte line terminators).
- Every typing pause re-installed unchanged prewarmed files, rebuilt the symbol map, bumped the generation, and sent
  client refreshes.
- References failed with ContentModified whenever any other open document changed.
- Cancelling a runtime-docs request killed the warm Sage worker; go-to-definition could wait up to 60 s for it; worker
  globals shadowed Sage names (`line`); `sage-python` and interpreter arguments broke the worker launch.
- Valid Sage syntax (`^^`, `[1, 3, .., 11]`, `f(x) = x^2`, `.spyx` Cython) was reported as errors, an unclosed bracket
  underlined the rest of the file, and XOR warnings could crowd out syntax errors. Diagnostics on the Sage library went
  from 78 to 0; every removed one was reviewed as a false positive.
- A `.sage` file outside every workspace folder had no language server; a corrected server path could be dropped
  during the version handshake; tag releases did not depend on the CI gate or check the version.
- After a restart, workspace definitions of hot Sage names (for example a local `PolynomialRing`) were hidden by an
  empty warm-up cache entry (pre-existing).

Measured on the reference host (`docs/perf/baseline-2026-11-17-review.json`): references for `matrix` 1.0 s (was
28.8 s, then 4.5 s), physical memory after cold index 175 MB and after Rebuild Index 250 MB (peaks 419 / 605 MB).
macOS `ps` RSS overstated memory by counting pages already returned as reusable.

Crash recovery: the server was killed with SIGKILL at 0.4, 1.2, 2.0, 2.6, and 3.2 s into a cold index of the full Sage
tree (during parsing and during the SQLite persist). Every restart recovered: 3,842 files indexed, no index error,
hover and definition for `PolynomialRing` correct, recovery 0.1-4.3 s.

Also delivered in this review: bracketed-paste REPL input (verified against a real Sage REPL), Shift+Enter /
Ctrl+Enter keybindings, Sage presence detection, recovery actions first in the status menu, Simplified Chinese for the
manifest, walkthrough, and runtime messages, RST-to-Markdown hovers, automatic deletion of index databases unused for
30 days, and the cache format recorded in each database.

Still open: native CI on Linux/Windows/Intel macOS has never run remotely; the shared-library cache design (one Sage
database for all projects); remaining synchronous handlers, `$/progress`, metrics, and a one-hour soak; LogOutputChannel,
esbuild bundling, and the extension version bump to 0.2.0.
