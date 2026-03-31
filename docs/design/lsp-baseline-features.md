# LSP Baseline Features

## Purpose

This note records the minimum feature set that should make the plugin feel like a real language tool rather than a
demo extension.

## Baseline

The current baseline now includes:

- hover
- completion
- definition
- signature help
- document symbols
- workspace symbols
- references
- rename
- low-noise unresolved-import diagnostics
- conservative syntax diagnostics for Python and `.sage`
- custom documentation requests
- dotted singleton-member resolution for common Sage patterns such as `graphs.PetersenGraph`
- member completion for statically understood singleton APIs
- completion responses serialized as concrete LSP `CompletionItem` objects under real clients

## Scope

- These features are primarily static and index-driven, with runtime fallback for documentation, definitions, and
  signatures when static resolution misses Sage runtime objects.
- Static resolution now includes class-body imports, singleton instance aliases, and dotted member traversal so common
  Sage generator objects remain navigable even when the selected Sage runtime cannot answer introspection requests.
- They are designed to stay predictable and low-noise while still remaining usable against real Sage installations.
- Diagnostics are intentionally conservative and currently focus on unresolved imports plus syntax errors that can be
  validated safely without pretending to approximate a full Python or Cython type checker.

## Follow-up Areas

- `.sage` source mapping still needs to feed more of the diagnostics and navigation surface.
- Semantic tokens, code actions, inlay hints, richer diagnostics, and deeper runtime-aware analysis remain open work.
