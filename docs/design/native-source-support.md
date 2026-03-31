# Native Source Support

## Purpose

This note defines how the plugin should treat Sage-native library sources such as `.pyx`, `.pxd`, and `.pxi` files.

## Decisions

- Register `.pyx`, `.pxd`, and `.pxi` as first-class editor documents under a dedicated `sagemath-cython` language id.
- Reuse the shared Sage grammar for both `sagemath` and `sagemath-cython` so `.sage` and native source files stay
  visually aligned while still recognizing Cython-specific constructs.
- Parse native source files with a lightweight static pass instead of a full Cython parser.
  The current pass extracts top-level classes, functions, constants, `cimport`, and `from ... cimport ...` bindings.
- Merge module records when the same logical module is represented by multiple files.
  Current precedence is `.py`/`.sage` over `.pyx` over `.pxd` over `.pxi`.
  This keeps implementation files authoritative while still exposing declarations that only exist in `.pxd`.

## Constraints

- The current parser is intentionally lightweight and does not aim to validate full Cython syntax.
- Native-source support currently targets navigation, completion, hover, and highlighting.
  Richer diagnostics, references, and rename remain future work.
