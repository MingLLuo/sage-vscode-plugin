# Manual Smoke Workspace

This workspace is a self-contained set of `.sage`, `.py`, `.pyx`, `.pxd`, and `.pxi` files for testing the current Sage VS Code
plugin baseline without depending on a full Sage source checkout.

## What It Covers

- Hover and documentation requests for local Python symbols
- Definition jumps into source roots and `analysis.extraPaths`
- Completion from direct imports and star imports
- Package imports and re-exports
- Lightweight `.pyx` indexing plus `.pxd` native declaration resolution
- `.sage` document symbols and preparser-style `R.<x> = ...` assignments
- Manual runtime checks for caret syntax and REPL execution
- Static lazy-import resolution in `.sage` files
- Native-component highlighting for Cython keywords, includes, and typed declarations
- Runtime-backed docs and definition fallback for Sage library APIs exposed through dotted access and heavier algebraic objects
- Synthetic Sage-heavy Python patterns for `sage.all` constructors, matrix methods, polynomial ring/ideal methods, and
  wrong-jump suppression on ambiguous dotted calls

## Recommended Flow

1. Open the repository root in VS Code.
2. Press `F5`. `Sage Plugin: Smoke Workspace` is the default launch target.
3. In the new `[Extension Development Host]` window, open `src/01_hover_and_definition.sage`.
4. Confirm the status bar language mode is `SageMath` and the left status bar contains `Sage: ...`.
5. If the extension host opens empty, use `Open Folder` inside that host and select this `manual-smoke-workspace`
   folder.
6. If `.sage` shows `Plain Text` or the command palette has no Sage commands, you opened a normal VS Code window; close
   it and relaunch from the repository with `F5`.
7. Run `Sage: Select Interpreter` when you want to point run commands and runtime-backed docs at a specific Sage
   executable.
8. Keep `sage.languageServer.rustPath = auto` unless you want to test a specific `sage-ls` binary.
9. Run `Sage: Show Index Status`, `Sage: Show Docs Status`, or `Sage: Run UX Self Check` from any sample file when you
   need a quick health report before deeper manual inspection.
10. Open the files below and follow the checklist.

For browser/MCP inspection, run `npm run debug:web` from the repository root and open the printed localhost URL. The
workbench renders this same workspace with TextMate scopes, Rust semantic tokens, diagnostics, symbols, index/docs
status, and a UX matrix for common Sage targets.

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

### 06 Native Components and Highlighting

Open `src/cythonish_bridge.pyx`, `src/native_support.pxd`, and `src/native_include.pxi`.

- Confirm `cdef`, `cpdef`, `cimport`, `include`, Cython types, and Sage helpers are highlighted.
- Use definition on `NativeAccumulator` and `native_step` from `cythonish_bridge.pyx`.
- Check document symbols in `cythonish_bridge.pyx` for `fast_square`, `StepCounter`, and `stepped_square`.
- Confirm `native_support.pxd` exposes the declared class and typed functions.

### 07 Runtime Graphs and Number Theory

Open `src/06_runtime_graphs_and_number_theory.sage`.

- Hover and use definition on `graphs.PetersenGraph`, `EllipticCurve`, `ideal`, `factor`, and `sigma`.
- Confirm dotted names such as `graphs.PetersenGraph` still return documentation instead of dropping to an unknown symbol.
- Run `Sage: Show Documentation` on `groebner_basis` and `automorphism_group`.

### 08 Symbolic and Combinatorics

Open `src/07_symbolic_and_combinatorics.sage`.

- Hover and use definition on `integrate`, `Partitions`, `Combinations`, `SymmetricGroup`, and `NumberField`.
- Confirm symbolic expressions using `^`, `oo`, and named generators still execute under `Sage: Run Current File`.
- Check document symbols for `Q`, `z`, `cyclotomic_field`, and `advanced_symbolic_targets`.

### 09 Highlighting Structures

Open `src/08_highlighting_structures.sage`.

- Confirm `toric_varieties`, `ChowGroup`, `PolynomialRing`, `NumberField`, `FreeModule`, and `FilteredSimplicialComplex`
  now land in distinct visual scopes instead of collapsing into one generic support color.
- Confirm keyword arguments such as `color=`, `legend_label=`, and `default=` are visually distinct from ordinary
  assignment targets.
- Confirm method calls such as `.ambient_space()` and `.hilbert_data()` stand out from variables, while dotted namespaces
  such as `codes.HammingCode(...)` still keep the namespace readable.
- Confirm `@cached_method`, `lazy_import`-style helpers, and factory-style names look different from ordinary function
  calls when your theme supports the richer scopes.
- Verify the preparse assignment `R.<u, v>` still highlights the parent name, generators, and assignment operator
  separately.
- Verify `@interact` controls and Sage ranges such as `[1..5]` are highlighted as Sage-specific syntax.

### 10 Advanced Sage Patterns

Open `src/09_advanced_sage_patterns.sage`.

- Confirm nested `PolynomialRing`, `NumberField`, `GF`, `FunctionField`, `ProjectiveSpace`, and `codes.HammingCode`
  code remains readable with distinct constructors, namespaces, and keyword arguments.
- Confirm method chains such as `.groebner_basis()`, `.right_kernel().dimension()`, and `.derivative(...)`
  stand out from variables.
- Confirm Sage ranges, list/dict comprehensions, lambda matrix builders, keyword-only function parameters, and
  cached functions do not collapse into a flat Python-like color treatment.

### 11 Sage-Heavy Python Patterns

Open `src/10_sage_heavy_python.py`.

- Hover and use definition on `PolynomialRing`, `GF`, `matrix`, `vector`, and `zero_matrix`; they should resolve through
  `sage.all` to the real Sage source modules.
- Use definition on `mat.rank()`, `amat.solve_right(...)`, `ring.ideal(...)`, `ideal.variety(...)`,
  `eqs[i].derivative(...)`, `pivot.resultant(...)`, and `gcd_poly.gcd(...)`.
- Confirm method resolution shows a high-confidence Sage owner type in the debug workbench for matrix, polynomial ring,
  polynomial element, and ideal calls.
- Confirm low-confidence or unsupported dotted methods do not jump to unrelated global functions.

## Layout

- `src`
  Main `.sage` files plus local Python, `.pyx`, `.pxd`, and `.pxi` modules indexed as source roots.
- `vendor`
  Extra-path modules used to verify `sage.analysis.extraPaths`.
