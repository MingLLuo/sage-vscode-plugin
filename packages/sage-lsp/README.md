# `sage-lsp`

This package contains the Python language server for the Sage VS Code Plugin monorepo.

## Current Scope

- provide a `pygls`-based server entrypoint
- deserialize nested initialization settings from the VS Code client
- parse Python, `.sage`, and lightweight `.pyx` sources
- build a static workspace index with import and lazy-import resolution
- serve hover, completion, definition, document symbols, and docs payloads
- expose a first `.sage` source-mapping implementation

## Deferred Work

- richer `.sage` preprocessing and mapping
- diagnostics, references, and rename
- runtime-aware documentation and environment inspection
