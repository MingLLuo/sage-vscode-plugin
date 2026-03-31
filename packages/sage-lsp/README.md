# `sage-lsp`

This package contains the Python language server for the Sage VS Code Plugin monorepo.

## Current Scope

- provide a minimal `pygls`-based server entrypoint
- deserialize basic initialization settings
- expose a health-oriented hover placeholder for early wiring checks

## Deferred Work

- `.sage` preprocessing and source mapping
- workspace indexing
- runtime-aware documentation and environment inspection

