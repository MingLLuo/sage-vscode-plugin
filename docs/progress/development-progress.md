# Development Progress

## Status Snapshot

- Date: 2026-03-31
- Repository: upgraded to a usable static-analysis baseline
- Process tracking: baseline in place
- Extension package: richer workflow and docs UX added
- Language server package: static indexing and symbol resolution added
- Syntax package: baseline scaffold added and synced into extension resources

## Current Focus

1. Extend source mapping beyond caret rewrite into more `.sage` constructs.
2. Feed source mapping into diagnostics, references, and rename paths.
3. Add extension-host integration tests beyond the current unit suite.
4. Layer runtime-aware Sage introspection on top of the static index where useful.

## Milestone Tracker

| Milestone | Status | Notes |
| --- | --- | --- |
| Repository bootstrap | Done | Root docs, package scaffolds, onboarding docs, CI placeholder, and local bootstrap validation are complete. |
| Process baseline | Done | Commit policy, task flow, and progress templates are now committed. |
| Design baseline | Done | Initial overview, workspace, server boundary, and source mapping notes are committed. |
| Extension scaffold | Done | Minimal VS Code client package, commands, configuration model, and language client wiring are committed. |
| LSP scaffold | Done | Minimal `pygls` package, entrypoint, and server settings model are committed. |
| Syntax scaffold | Done | Syntax package, sync script, and generated extension assets are committed. |
| Source mapping v1 | Done | `.sage` caret rewrite, string/comment skipping, bidirectional column maps, and hover preview wiring are committed. |
| Static source intelligence baseline | Done | Parser, workspace index, lazy import resolution, docs extraction, and LSP-backed symbol features are committed. |
| Extension workflow baseline | Done | Status bar, environment presentation, run commands, docs panel, and richer settings model are committed. |

## Change Log Notes

- Initialized an independent repository and set `main` as the default branch.
- Added root governance and architecture documents.
- Reserved `docs/design`, `docs/process`, and `docs/progress` for ongoing repository records.
- Added process templates for commit policy, task state flow, development logs, and milestone reviews.
- Added the first `extension-core` scaffold with commands, settings mapping, and stdio language-client wiring.
- Added the first `sage-lsp` scaffold with a `pygls` entrypoint, server settings model, and basic tests.
- Added the first `syntax-pack` scaffold plus a sync script that materializes extension-owned runtime assets.
- Added developer onboarding docs and a bootstrap GitHub Actions workflow aligned with root scripts.
- First dependency-install pass exposed and isolated a Node workspace wiring issue in the extension package manifest.
- Installed Node and Python dependencies locally, corrected bootstrap lifecycle typing, and verified `npm run build`, `npm run lint`, and `npm run test`.
- Accepted the first concrete `.sage` mapping slice around caret-to-power rewrite with bidirectional line-local column maps.
- Implemented the first real `.sage` preprocessing module, covered it with Python tests, and surfaced mapping information in the server hover path.
- Ported a reduced Sage fixture corpus plus parser/index stack into the `pygls` server and verified static hover, completion, definition, symbol, and documentation paths through tests.
- Upgraded the VS Code client with richer configuration, source-root discovery, run commands, status presentation, documentation rendering, and unit-test coverage.
