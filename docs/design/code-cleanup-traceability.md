# Code Cleanup and Traceability

## Purpose

This cleanup keeps the current behavior intact while making future Sage language-server work easier to split, review,
and debug. It is intentionally not the SQLite/Jedi rewrite; it prepares the codebase for that work by adding stable
module boundaries, structured logs, and debug status hooks.

The first follow-up rewrite slice is tracked separately in `docs/design/core-rewrite-v1.md`.

## Cleanup Boundaries

- Keep `sage_lsp.index` as the compatibility import path.
- Move the active index implementation behind `sage_lsp.workspace_index`.
- Add narrow split-point modules for loading, resolution, summaries, diagnostics, and serialization before moving
  behavior out of `workspace_index.py` in later increments.
- Keep VS Code activation and command registration in `extension.ts`, but move operational helpers such as terminal
  state and structured logging out of the activation body.

## Trace Fields

Language-server trace events use these fields when available:

- `method`: LSP or custom request name.
- `uri`: document URI.
- `generation`: workspace-index generation.
- `cache`: cache namespace such as `documentation` or `definition`.
- `result`: `hit`, `miss`, or request outcome.
- `reason`: fallback or miss reason.
- `elapsed_ms`: request or index operation duration.

Extension logs use:

- `level`: `error`, `warn`, `info`, or `debug`.
- `component`: subsystem such as `extension`, `configuration`, or `lsp-client`.
- key-value fields after the message.

## Debug Flow

1. Check the `Sage` and `Sage Language Server` output channels.
2. Set `sage.logging.level` to `debug` and restart the language server.
3. Inspect the `sage/__debug/indexStatus` request in tests or through a temporary client hook.
4. Confirm source roots, deferred roots, module counts, summary-cache state, and runtime-introspection availability.
5. Reproduce the failing symbol in `examples/manual-smoke-workspace` or a smaller fixture before changing resolver logic.

## Task Links

- `LSP-029`: language-server cleanup and trace hooks.
- `EXT-012`: structured extension logging and terminal-manager extraction.
- `QA-006`: regression coverage for facade compatibility, debug index status, and logging filters.
