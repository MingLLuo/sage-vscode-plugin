# Performance Baselines

`npm run perf:baseline -- --out docs/perf/baseline-<date>.json` records what users feel while working against a full
Sage source tree (`SAGE_SOURCE_ROOT` or `../sage/src`): cold and warm start, latency while indexing and while typing in
a 27k-line file, project-wide references, memory, and cache size. It never fails on a slow number; compare files to see
regressions and improvements.

`npm run snapshot:index -- /path/to/sage/src > before.jsonl` writes a deterministic per-file digest of symbols,
semantic spans, and diagnostics. Run it before and after a parser or dependency change and compare with
`npm run snapshot:diff -- before.jsonl after.jsonl`; any difference must be explained before the change lands.

## 2026-11-16 baseline (release build, darwin-arm64, 8 cores, Sage 36e49d91b05, 3,842 indexed files)

| Measurement | Result | Target |
| --- | --- | --- |
| Cold start: index ready | 3.3 s | keep |
| Warm start: index ready / first hover | 82 ms / 16 ms | keep |
| Hover right after six saves during the first index pass | 3,125 ms | < 250 ms |
| `semanticTokens/full`, `generic_graph.py` (27,706 lines) | 1,064 ms | < 150 ms |
| `semanticTokens/range`, 60 lines of the same file | 217 ms | < 50 ms |
| `documentSymbol`, same file | 333 ms | < 150 ms |
| Hover / completion p95 while typing in the same file | 65 ms / 66 ms | keep |
| References for `matrix` (1,898 locations) | 28.8 s | < 2 s |
| Hover issued while that references request runs | 28.9 s | < 250 ms |
| References for `QQ` (2,299 locations) | 6.2 s | < 2 s |
| RSS: warm steady state | 56 MB | keep |
| RSS: after the first index pass / after `Rebuild Index` | 476 MB / 820 MB | < 400 MB |
| Persistent cache for one Sage tree | 205 MB | one copy per Sage tree |

## 2026-11-17 continuation (same host and Sage source)

| Measurement | Before | After |
| --- | ---: | ---: |
| Hover after six saves during indexing | 3,125 ms | 36.4 ms |
| Large-file semantic full, p50 / p95 | 1,064 / 1,158 ms | 0.4 / 73.2 ms |
| Large-file semantic range, p50 | 217 ms | 0.3 ms |
| Large-file document symbols, p50 | 333 ms | 44.6 ms |
| References: matrix / QQ | 28.8 / 6.2 s | 4.5 / 1.7 s |
| Hover during matrix / QQ references | 28,941 / 6,019 ms | 1.5 / 4.2 ms |
| Last edit to final versioned diagnostics | not reliably measured | 209 ms |
| Warm index ready / first hover | 82 / 16 ms | 74 / 10 ms |
| RSS after cold index / rebuild | 476 / 820 MB | 627 / 694 MB |

Full/range samples reuse an unchanged document; cached p50 must not be interpreted as uncached parsing time. The first
large-file full request took 73.2 ms; after 100 edits it took 68.5 ms. The diagnostic timing now starts at the final edit's
notification and waits for that exact version (101), correcting the earlier timestamp/waiter handling.
RSS is sampled periodically, so `peak_rss_mb` can miss the separate final reading; use both when assessing memory.
Matrix references still exceed the 2-second target and cold/rebuild memory still exceeds 400 MB. Cache remains about
205 MB per combined workspace/source database; shared-library storage has not been implemented. The final run includes
the cache-retention fix discovered by the release latency gate; it supersedes earlier measurements from this day.

`snapshot-comparison-2026-11-17.json` compares HEAD `aee9517` with the continuation's parser/snapshot implementation:
3,921 files, 126,514 symbols, 273,949 semantic spans and 78 diagnostics, with zero differences. The snapshot export walks
more files than normal indexed discovery (3,842); these counts describe different traversal filters. It does not compare
all dynamic navigation results; protocol and identity tests cover those separately.

The later diagnostic checkpoint intentionally changes diagnostics; see `snapshot-diagnostics-2026-11-17.json` for all
30 affected files and explanations. Symbol and semantic digests remain identical. Historical validation JSON files record
the VSIX hash at each checkpoint; rebuilding replaces the artifact at the same path.

## Later diagnostic and folding checkpoint

`baseline-diagnostics-2026-11-17.json` is the latest isolated run. Large-file outline p50/p95 is 43.6/44.4 ms; folding
p50/p95 is 10.1/10.2 ms. Version-101 diagnostics arrive 280.3 ms after the final edit (under 500 ms). Unlike the earlier
implementation, diagnostic parsing now reaches the end of `generic_graph.py` instead of returning on a caret in a docstring.
First semantic-full is 63.2 ms; typing hover/completion p95 is 74.4/13.0 ms. Matrix references still take 3.70 s; cold and
post-rebuild RSS is 648.1/793.1 MiB, above the 400 MiB target. These measurements do not establish full production readiness.

`validation-diagnostics-2026-11-17.json` records the final local gates and current VSIX hash. The earlier validation files
are historical checkpoints. Native CI and the remaining storage/client milestones are tracked in the continuation audit.
