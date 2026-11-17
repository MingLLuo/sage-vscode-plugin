# Task Board

This file intentionally tracks only active and recent release work. Detailed historical logs were removed to keep the
repository focused on code, tests, and the current product state.

## Active

The 2026-11-17 [continuation audit](../process/runtime-audit-2026-11-17.md) reconciles Claude's production-readiness
plan with implemented changes, measured results, and the remaining M1–M5 work. Green local gates below do not imply
that native cross-platform CI, shared-library caching, localization, or the 0.2.0 release is complete.

| ID | Area | Status | Notes |
| --- | --- | --- | --- |
| V2-STABLE | Rust LSP | active | Keep Sage API hover, definition, references, rename, completion, and docs under the release latency budgets. |
| UX-SMOKE | VS Code UX | active | Validate the real Extension Host and the Browser debug workbench after visible editor or activation changes. |
| RELEASE-GATE | Packaging/Test | active | Keep `test:ci`, `test:release`, VSIX package checks, and extension-host smoke green. |

## Recent

| ID | Area | Status | Notes |
| --- | --- | --- | --- |
| PR-RUNTIME | Runtime/lifecycle | done locally | Isolated worker sessions, bounded protocol/cache, process-tree cancellation, server preflight and startup recovery; six real editor scenarios passed. |
| PR-DIAGNOSTICS | Correctness | done locally | Precise mapped diagnostics and multiline function boundaries; Sage symbol/highlight snapshot unchanged; 321 Rust tests, release gate and six editor scenarios pass. |
| V2-109 | Hot Sage symbol paths | done | Exact Sage lookups use materialized export caches and hot symbol candidates before broad SQLite search. |
| V2-110 | LSP startup | done | `sage-ls` uses a bounded Tokio runtime so initialization remains below the latency target. |
| V2-111 | VS Code activation | done | Sage commands activate on demand, and Sage/Python files wake the extension without activating it in unrelated non-Python windows. |
