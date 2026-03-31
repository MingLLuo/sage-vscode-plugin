# LSP Baseline Features

## Purpose

This note records the minimum feature set that should make the plugin feel like a real language tool rather than a
demo extension.

## Baseline

The current baseline now includes:

- hover
- completion
- definition
- document symbols
- workspace symbols
- references
- rename
- low-noise unresolved-import diagnostics
- custom documentation requests

## Scope

- These features are currently static and index-driven.
- They are designed to be predictable and low-noise before the plugin grows runtime-aware Sage introspection.
- Diagnostics are intentionally conservative and currently report unresolved imports instead of trying to approximate a
  full Python or Cython type checker.

## Follow-up Areas

- `.sage` source mapping still needs to feed more of the diagnostics and navigation surface.
- Signature help, semantic tokens, code actions, inlay hints, and richer diagnostics remain open work.
