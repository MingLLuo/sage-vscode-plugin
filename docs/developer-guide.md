# Developer Guide

## Purpose

This guide is for engineers working on the plugin itself. It explains how the repository is organized, how progress is
tracked, and how the current static-analysis baseline should be extended safely.

## Repository Map

- `packages/extension-core`
  VS Code extension client. Owns activation, command registration, configuration plumbing, status presentation,
  documentation panel rendering, workspace discovery, and LSP client startup.
- `packages/sage-lsp`
  Python language server. Owns `pygls` lifecycle, environment normalization, parser and index logic, static symbol
  resolution, and `.sage` preprocessing primitives.
- `packages/syntax-pack`
  Shared syntax assets. Owns grammar, snippets, and language configuration.
- `docs/design`
  Architecture notes and accepted design decisions.
- `docs/process`
  Commit rules, task flow, and operating templates.
- `docs/progress`
  Milestone status and task-level tracking.
- `.vscode`
  Repository-local launch and task definitions for starting the extension development host with `F5`.
- `examples/manual-smoke-workspace`
  Self-contained manual smoke-test workspace used to exercise hover, definition, completion, docs, `.pyx`, and
  `.sage` cases.

## Working Rules

1. Start by identifying the subsystem: `extension`, `lsp`, `syntax`, or `repo/docs`.
2. Keep the client thin unless a UI concern truly belongs in VS Code.
3. Keep analysis behavior in `sage-lsp`; extension code should consume stable server payloads rather than duplicate logic.
4. Update progress records when milestone status changes.
5. Add or update a design note when architecture or repository rules change.

## Key Extension Modules

- `src/configuration.ts`
  Reads VS Code settings into the stable `SageSettings` model.
- `src/workspaceDiscovery.ts`
  Determines which workspace roots should be indexed.
- `src/languageClient.ts`
  Starts the `pygls` server and sends custom documentation requests.
- `src/executionPlan.ts`
  Builds shell-safe run commands and REPL load commands from extension settings.
- `src/serverRestart.ts`
  Limits language-server restarts to configuration changes that actually affect analysis behavior.
- `src/documentationRequest.ts`
  Normalizes documentation payloads into a render-friendly shape.
- `src/docsPanel.ts`
  Owns the documentation webview lifecycle.

## Key Language Server Modules

- `src/sage_lsp/parser.py`
  Parses Python, loose `.sage`, and lightweight `.pyx` files into a common module model.
- `src/sage_lsp/index.py`
  Builds workspace state and resolves symbols through imports, star imports, and lazy imports.
- `src/sage_lsp/source_map.py`
  Hosts the first `.sage` preprocessing and bidirectional position mapping primitives.
- `src/sage_lsp/server.py`
  Wires the index and parser into `pygls` request handlers.
- `tests/fixtures/sage_src_lite`
  Reduced Sage-aligned source corpus used for parser and index regression tests.

## Bootstrap Commands

```bash
npm install
python -m pip install -e ./packages/sage-lsp[dev]
npm run sync:syntax
npm run build
npm run test
```

## Local Debugging

- Press `F5` in this repository and choose `Sage Plugin: Extension Host`.
- The repository-level `build` task runs first, then VS Code launches the extension from `packages/extension-core`.
- The launch configuration opens the current repository as the first workspace in the extension host.
- Use `Sage Plugin: Smoke Workspace` when you want the extension host to open the curated sample files directly.

## Verification Strategy

- TypeScript-facing work should keep `npm run build` and `npm run lint` green.
- Python-facing work should keep `npm run test:python` green.
- Syntax work should keep `npm run sync:syntax` and syntax package checks green.
- Cross-cutting changes should keep the full `npm run test` path green before commit.
