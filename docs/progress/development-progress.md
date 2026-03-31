# Development Progress

## Status Snapshot

- Date: 2026-03-31
- Repository: bootstrap in progress
- Process tracking: baseline in place
- Extension package: baseline scaffold added
- Language server package: baseline scaffold added
- Syntax package: not yet scaffolded

## Current Focus

1. Land syntax assets and connect them to the extension package.
2. Add build and test placeholders that match the new package layout.
3. Tighten bootstrap documentation links and repository onboarding flow.

## Milestone Tracker

| Milestone | Status | Notes |
| --- | --- | --- |
| Repository bootstrap | In progress | Root docs and workspace metadata landed; package scaffolds pending. |
| Process baseline | Done | Commit policy, task flow, and progress templates are now committed. |
| Design baseline | In progress | Initial overview, workspace, server boundary, and source mapping notes are being added. |
| Extension scaffold | In progress | Minimal VS Code client package, commands, and initialization model are being added. |
| LSP scaffold | In progress | Minimal `pygls` package, entrypoint, and settings model are being added. |
| Syntax scaffold | Planned | Language configuration and grammar placeholders will follow package setup. |

## Change Log Notes

- Initialized an independent repository and set `main` as the default branch.
- Added root governance and architecture documents.
- Reserved `docs/design`, `docs/process`, and `docs/progress` for ongoing repository records.
- Added process templates for commit policy, task state flow, development logs, and milestone reviews.
- Added the first `extension-core` scaffold with commands, settings mapping, and stdio language-client wiring.
- Added the first `sage-lsp` scaffold with a `pygls` entrypoint, server settings model, and basic tests.
