# Runtime Source-Root Discovery

## Purpose

This note defines how the extension discovers indexable Sage source roots when a workspace does not explicitly provide
`sage.analysis.sourceRoots`.

## Decisions

- Keep workspace-owned source-root discovery in the extension host.
  The extension already knows the selected Sage runtime path and the VS Code workspace layout, so it can derive a
  stable list before the language server starts.
- Merge two discovery channels when `sage.analysis.sourceRoots` is empty:
  workspace-local roots and interpreter-derived Sage roots.
- Prefer cheap filesystem heuristics first.
  The extension checks for adjacent `src/sage` trees and nearby `site-packages/sage` roots around the selected
  interpreter path.
- Use a short runtime probe as a fallback.
  When heuristics are insufficient, the extension invokes the selected `sage` or `python` runtime with a short script
  that imports `sage` and reports its package root.
- Treat explicit `sage.analysis.sourceRoots` as authoritative.
  Manual configuration still wins over any automatic discovery logic.

## Constraints

- Discovery is intentionally conservative and only runs for common filesystem layouts plus a short runtime probe.
- The runtime probe is not a substitute for richer runtime introspection of docs, signatures, or semantic analysis.
- If a Sage installation does not expose importable sources or the runtime cannot import `sage`, users may still need
  to configure `sage.analysis.sourceRoots` manually.
