# `@sage-vscode/extension-core`

This package contains the VS Code extension client for the Sage VS Code Plugin monorepo.

## Current Scope

- register Sage-facing commands
- define initialization settings sent to the language server
- discover source roots and present environment context in the UI
- start a stdio-based language client
- render a documentation webview panel
- expose run-current-file, run-selection, and start-REPL command paths

## Deferred Work

- notebook surfaces
- deeper environment discovery
- extension-host integration tests
