# Sage VS Code Plugin

`Sage VS Code Plugin` is an independent monorepo for a SageMath-focused development experience in Visual Studio Code.
The repository starts from a minimal VS Code LSP client scaffold plus a `pygls` server scaffold so architecture and
process can evolve incrementally with full Git traceability.

## Goals

- Deliver a maintainable Sage editor experience for `.sage` files.
- Keep the client, language server, and syntax assets cleanly separated.
- Record design decisions, progress, and commit-level development history in-repo from the start.

## Workspace Layout

- `packages/extension-core`: VS Code extension and LSP client bootstrap.
- `packages/sage-lsp`: Python language server built on `pygls`.
- `packages/syntax-pack`: grammar, snippets, and language configuration.
- `docs/`: design notes, process rules, and progress tracking.

## Current Status

- Repository bootstrap is in progress.
- Process tracking and design templates are being established before feature work expands.
- Package scaffolds are intentionally minimal until the first implementation milestones are committed.

## Reference Inputs

- `deep-research-report.md` in the sibling `sage-src` workspace defines the target product direction.
- `/workspace/sage` is used as a local Sage source calibration checkout.
- Nearby repositories may be consulted for patterns, but this repository remains independently owned.

## Development Workflow

- Use Conventional Commits with narrow scopes.
- Keep each small action or feature in its own commit when practical.
- Update the progress tracker together with relevant implementation or design work.
- Add or update design notes whenever an architectural decision moves.

## Documentation Index

- [Developer Guide](./docs/developer-guide.md)
- [Install and Configure](./docs/install-and-configure.md)
- [Design Overview](./docs/design/overview.md)
- [Development Progress](./docs/progress/development-progress.md)

## Next Steps

1. Finalize process and progress templates.
2. Land the minimal extension, server, and syntax package scaffolds.
3. Start milestone-driven implementation from the documented package boundaries.
