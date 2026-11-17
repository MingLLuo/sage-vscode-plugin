# Performance

Measured on a release build, darwin-arm64 (8 cores), against the full Sage library (3,842 indexed files).
`baseline-2026-11-16.json` is before the 0.2.0 work; `baseline-2026-11-17-review.json` is 0.2.0.

| Measurement | Before | 0.2.0 | Target |
| --- | ---: | ---: | --- |
| Hover right after six saves during the first index pass | 3,125 ms | 21 ms | < 250 ms |
| `semanticTokens/full` on a 27,706-line file, p95 | 1,064 ms | 60 ms | < 150 ms |
| `documentSymbol`, same file, p50 | 333 ms | 41 ms | < 150 ms |
| Last edit to final diagnostics | not measured | 251 ms | < 500 ms |
| Hover / completion p95 while typing | 65 / 66 ms | 66 / 10 ms | < 250 ms |
| References for `matrix` (1,898 locations) | 28.8 s | 1.0 s | < 2 s |
| Hover while that references request runs | 28.9 s | 1 ms | < 250 ms |
| References for `QQ` (2,299 locations) | 6.2 s | 0.5 s | < 2 s |
| Memory after cold index / peak | 476 MB | 175 / 419 MB | < 400 MB / < 1 GB |
| Memory after Rebuild Index / peak | 820 MB | 250 / 605 MB | < 400 MB / < 1 GB |
| Warm start: index ready / first hover / memory | 82 / 16 ms / 56 MB | 69 / 8 ms / 46 MB | keep |
| Second project using the same Sage tree: ready / extra disk | 4–5 s / +200 MB | 0.5 s / 0 MB | one copy per tree |

Memory is macOS `phys_footprint` (the "Before" column used `ps` RSS, which also counts pages the allocator already
returned). Reference results are byte-identical before and after the optimizations.

## Measuring

```bash
npm run perf:baseline -- --out docs/perf/baseline-<date>.json   # uses SAGE_SOURCE_ROOT or ../sage/src
```

It never fails on a slow number; compare the JSON with the files here. Before and after a parser or dependency change,
compare analysis output too:

```bash
npm run snapshot:index -- /path/to/sage/src > before.jsonl
npm run snapshot:index -- /path/to/sage/src > after.jsonl
npm run snapshot:diff -- before.jsonl after.jsonl
```

Every difference in symbols, highlighting, or diagnostics must be explained before the change lands.
