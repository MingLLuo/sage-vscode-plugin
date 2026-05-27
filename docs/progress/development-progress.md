# Development Progress

## Current Baseline

- Primary language server: Rust `sage-ls`.
- Index/cache engine: `crates/sage-index` with SQLite-backed symbols, docs, references, and Sage export caches.
- Extension surface: VS Code commands, status, documentation panel, REPL/run helpers, support bundle, UX self-check, and
  Extension Host smoke coverage.
- Retained baseline: legacy Python LSP stays in `packages/sage-lsp` for regression comparison.

## Current Quality Gates

- `cargo test`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `npm run lint`
- `npm run test`
- `npm run test:lsp-latency`
- `npm run test:real-file-smoke`
- `npm run test:extension-host`
- `git diff --check`

## Notes

Keep this file short. Move durable design decisions into `docs/design/`, and prefer tests over long process notes.
