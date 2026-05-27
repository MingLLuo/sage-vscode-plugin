# Workspace Layout

## Repository Structure

- `packages/extension-core`: VS Code extension client and command surfaces.
- `packages/sage-lsp`: `pygls` language server and analysis package.
- `packages/syntax-pack`: language grammar, snippets, and editor metadata.
- `docs/design`: design notes and accepted decisions.
- `docs/process`: release-gate notes that are still test-covered.
- `docs/progress`: current status and task tracking.

## Repository Conventions

- Root documents define repository-wide policy and architecture.
- Package-local READMEs explain package purpose and local commands.
- Cross-package changes should update the short progress tracker only when current release state changes.
