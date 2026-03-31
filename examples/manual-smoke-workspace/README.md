# Manual Smoke Workspace

This workspace is a self-contained set of `.sage`, `.py`, and `.pyx` files for testing the current Sage VS Code
plugin baseline without depending on a full Sage source checkout.

## What It Covers

- Hover and documentation requests for local Python symbols
- Definition jumps into source roots and `analysis.extraPaths`
- Completion from direct imports and star imports
- Package imports and re-exports
- Lightweight `.pyx` indexing
- `.sage` document symbols and preparser-style `R.<x> = ...` assignments
- Manual runtime checks for caret syntax and REPL execution
- Static lazy-import resolution in `.sage` files

## Recommended Flow

1. Start the extension host with `F5`.
2. Choose `Sage Plugin: Smoke Workspace`.
3. In the new window, run `Sage: Select Interpreter` and point it at your Sage executable.
4. Open the files below and follow the checklist.

## Smoke Checklist

### 01 Hover and Definition

Open `src/01_hover_and_definition.sage`.

- Hover `make_demo_matrix`, `alternating_square_sum`, and `named_polynomial`.
- Run `Sage: Show Documentation` on `vendor_banner`.
- Use definition on `fast_square`, `EXTERNAL_LABEL`, and `AffineNote`.
- Check document symbols for `R`, `x`, `demo_matrix`, `square_fast`, and `note_box`.

### 02 Star Import and Completion

Open `src/02_star_import_and_completion.sage`.

- Place the cursor after `alt` on the last line and request completion.
- Verify `alternating_square_sum` appears from the extra-path module.
- Place the cursor after `Aff` on the last line and verify `AffineNote` appears.

### 03 Source Mapping and Runtime Syntax

Open `src/03_source_mapping_cases.sage`.

- Run the file with `Sage: Run Current File`.
- Confirm Sage accepts the caret expressions such as `2^10`.
- Confirm strings, triple-quoted text, and comments keep literal `^` characters.

Note: current editor-side source mapping is still an early slice, so this file is primarily for manual runtime checks and
future navigation regression work.

### 04 Lazy Import and Packages

Open `src/04_lazy_import_and_packages.sage`.

- Hover or use definition on `alt_square_sum`, `named_polynomial`, and `NotebookAlias`.
- Confirm the lazy import aliases resolve to the underlying Python modules.

### 05 Symbols and Locals

Open `src/05_symbols_and_locals.sage`.

- Check document symbols for `LocalContainer`, `local_builder`, `GAMMA`, `R`, and `z`.
- Use completion on the final `loc` prefix to confirm local top-level names appear.

## Layout

- `src`
  Main `.sage` files plus local Python and `.pyx` modules indexed as source roots.
- `vendor`
  Extra-path modules used to verify `sage.analysis.extraPaths`.
