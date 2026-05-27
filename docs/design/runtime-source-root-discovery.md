# Runtime Source-Root Discovery

## Purpose

This note defines how the extension discovers indexable Sage source roots when a workspace does not explicitly provide
`sage.analysis.sourceRoots`.

## Decisions

- Keep workspace-owned source-root discovery in the extension host.
  The extension already knows the selected Sage runtime path and the VS Code workspace layout, so it can derive a
  stable list without asking the Rust language server to guess editor-specific context.
- Merge two discovery channels when `sage.analysis.sourceRoots` is empty:
  workspace-local roots and interpreter-derived Sage roots.
- Prefer cheap filesystem heuristics first.
  The extension checks for adjacent `src/sage` trees and nearby `site-packages/sage` roots around the selected
  interpreter path.
- Keep the startup path non-blocking.
  Language-client initialization and status-bar refreshes use configured roots, workspace roots, nearby Sage checkouts,
  and cheap filesystem heuristics only. They explicitly skip runtime subprocess probes.
- Use a short asynchronous runtime probe as a fallback.
  When heuristics are insufficient, the extension invokes the selected `sage` or `python` runtime with a short script
  that imports `sage` and reports its package root. This probe runs only when the current workspace/editor context
  already exposes the Sage experience; ordinary Python workspaces stay quiet even if `sage` is available on `PATH`.
- Apply runtime-discovered roots in memory.
  If the asynchronous probe discovers new roots, the extension restarts the Sage language client once with those roots
  added to the initialization payload. It does not rewrite the user's settings.
- Treat explicit `sage.analysis.sourceRoots` as authoritative.
  Manual configuration still wins over any automatic discovery logic.

## Constraints

- Discovery is intentionally conservative and only runs for common filesystem layouts plus a short asynchronous runtime
  probe.
- The runtime probe is not a substitute for richer runtime introspection of docs, signatures, or semantic analysis.
- If a Sage installation does not expose importable sources or the runtime cannot import `sage`, users may still need
  to configure `sage.analysis.sourceRoots` manually.
