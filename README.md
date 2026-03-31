# Sage VS Code Plugin

`Sage VS Code Plugin` is an independent monorepo for a SageMath-focused development experience in Visual Studio Code.
It now includes a usable static-analysis baseline: a richer VS Code client, a `pygls` language server with workspace
indexing, and Sage-aware `.sage` preprocessing primitives.

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

- Repository bootstrap is complete and locally validated.
- The extension provides interpreter selection, status presentation, run commands, and a documentation panel.
- The language server provides static hover, completion, definition, document symbols, documentation requests, and
  reduced Sage fixture-backed source indexing.
- `.sage` source mapping v1 currently rewrites standalone exponent carets while preserving bidirectional column maps.

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

1. Extend `.sage` preprocessing beyond the initial caret rewrite.
2. Feed source mapping into diagnostics and navigation paths.
3. Add deeper integration tests and runtime-aware fallbacks.
