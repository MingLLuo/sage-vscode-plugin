# Developer Guide

## Purpose

This guide is for engineers working on the plugin itself. It explains how the repository is organized, how progress is
tracked, and how the early package boundaries should be extended safely.

## Repository Map

- `packages/extension-core`
  VS Code extension client. Owns activation, command registration, configuration plumbing, and LSP client startup.
- `packages/sage-lsp`
  Python language server. Owns `pygls` lifecycle, analysis-side settings, and future Sage-specific source handling.
- `packages/syntax-pack`
  Shared syntax assets. Owns grammar, snippets, and language configuration.
- `docs/design`
  Architecture notes and accepted design decisions.
- `docs/process`
  Commit rules, task flow, and operating templates.
- `docs/progress`
  Milestone status and task-level tracking.

## Working Rules

1. Start by identifying the subsystem: `extension`, `lsp`, `syntax`, or `repo/docs`.
2. Keep the client thin unless a UI concern truly belongs in VS Code.
3. Keep analysis behavior in `sage-lsp`, even if the first implementation is minimal.
4. Update progress records when milestone status changes.
5. Add or update a design note when architecture or repository rules change.

## Bootstrap Commands

```bash
npm install
python -m pip install -e ./packages/sage-lsp[dev]
npm run sync:syntax
npm run build
npm run test
```

## Verification Strategy

- TypeScript-facing work should keep `npm run build` and `npm run lint` green.
- Python-facing work should keep `npm run test:python` green.
- Syntax work should keep `npm run sync:syntax` and syntax package checks green.

