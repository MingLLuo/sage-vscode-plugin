# `@sage-vscode/syntax-pack`

This package owns editor-facing syntax assets for Sage source files and Sage-native Cython sources.

## Current Scope

- language configuration
- TextMate grammar for `.sage`, `.pyx`, `.pxd`, `.pxi`, and `.spyx`
- SageMath snippets for common functions, algebraic structures, plotting, and cached methods
- sync hook for the extension package; check mode also rejects stale generated files outside the expected asset list
