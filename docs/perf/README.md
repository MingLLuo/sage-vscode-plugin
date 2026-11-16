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
