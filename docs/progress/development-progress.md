# Development Progress

## Status Snapshot

- Date: 2026-03-31
- Repository: bootstrap in progress
- Process tracking: baseline in place
- Extension package: baseline scaffold added
- Language server package: baseline scaffold added
- Syntax package: baseline scaffold added and synced into extension resources

## Current Focus

1. Tighten bootstrap documentation links and repository onboarding flow.
2. Add initial CI placeholders that match the package layout.
3. Start the first implementation-facing design note for `.sage` preprocessing.

## Milestone Tracker

| Milestone | Status | Notes |
| --- | --- | --- |
| Repository bootstrap | In progress | Root docs and all three package scaffolds are now present; onboarding polish remains. |
| Process baseline | Done | Commit policy, task flow, and progress templates are now committed. |
| Design baseline | Done | Initial overview, workspace, server boundary, and source mapping notes are committed. |
| Extension scaffold | Done | Minimal VS Code client package, commands, configuration model, and language client wiring are committed. |
| LSP scaffold | Done | Minimal `pygls` package, entrypoint, and server settings model are committed. |
| Syntax scaffold | Done | Syntax package, sync script, and generated extension assets are committed. |

## Change Log Notes

- Initialized an independent repository and set `main` as the default branch.
- Added root governance and architecture documents.
- Reserved `docs/design`, `docs/process`, and `docs/progress` for ongoing repository records.
- Added process templates for commit policy, task state flow, development logs, and milestone reviews.
- Added the first `extension-core` scaffold with commands, settings mapping, and stdio language-client wiring.
- Added the first `sage-lsp` scaffold with a `pygls` entrypoint, server settings model, and basic tests.
- Added the first `syntax-pack` scaffold plus a sync script that materializes extension-owned runtime assets.
