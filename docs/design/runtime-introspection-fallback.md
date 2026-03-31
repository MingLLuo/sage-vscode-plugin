# Runtime Introspection Fallback

## Purpose

This note defines how the language server should use a live Sage runtime when static indexing cannot provide enough
information for documentation, source navigation, or signatures.

## Decisions

- Keep static indexing as the primary path for hover, definition, signature help, and docs requests.
- Add a runtime fallback only after static resolution fails.
- Resolve runtime symbols through Sage's own `sage.misc.sageinspect` helpers instead of raw Python `inspect`.
  This keeps Cython-backed Sage objects and wrapped callables inspectable.
- Cache runtime lookup results by symbol name inside the language server process.
- Enable runtime fallback by default, but make it configurable through `sage.analysis.enableRuntimeIntrospection`.
- Preserve dotted symbol names such as `graphs.PetersenGraph` when invoking runtime fallback.
  Many Sage APIs are accessed through dotted generators and families rather than bare names.

## Constraints

- Runtime fallback currently targets documentation, definition lookups, and signature help.
- The fallback resolves bare names and dotted names that exist in the selected Sage runtime; it does not model arbitrary
  workspace-local Python state.
- If the selected runtime cannot import `sage`, the server falls back to its static-only behavior.
