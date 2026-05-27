# Sage VS Code Extension

Preview VS Code support for SageMath projects, Sage-heavy Python files, and Sage native Cython sources.

The extension identifier is `sage-vscode.sage-vscode-extension`.

## What It Provides

- Sage language support for `.sage`, `.pyx`, `.pxd`, and `.pxi`.
- Opt-in Sage-aware analysis for ordinary `.py` files through `sage.analysis.enablePythonFiles`.
- Rust-backed hover, documentation, definition, completion, signature help, diagnostics, semantic tokens, symbols,
  references, rename, inlay hints, folding, selection ranges, document links, call hierarchy, and quick fixes.
- Fast SQLite-backed indexing for Sage, Python, and Cython source roots.
- Static Sage API resolution for common `sage.all` constructors, matrix/vector/polynomial/free-module/finite-field
  methods, and real Sage source navigation.
- Runtime-backed documentation fallback when a configured Sage runtime is available.
- Run current file, run selection, run cell/region, and managed Sage REPL commands.
- Status, environment, index, docs, support bundle, and UX self-check commands for troubleshooting.

## First Run

1. Install the VSIX or marketplace package.
2. Open a local trusted workspace containing Sage files, Sage-heavy Python files, or native Sage Cython files.
3. Run `Sage: Open Getting Started`.
4. Run `Sage: Select Interpreter` and choose a local Sage checkout or system Sage runtime.
5. Run `Sage: Configure Workspace`.
6. Choose `Sage-heavy Python workspace` for `.py` files that import `sage.all`, or `Full Sage research workspace` for
   mixed Sage/Python/Cython projects.
7. Check `Sage: Show Environment Details`, `Sage: Show Index Status`, and `Sage: Run UX Self Check`.

For predictable Sage library navigation, configure the Sage source root when it is not discovered automatically:

```json
{
  "sage.languageServer.rustPath": "auto",
  "sage.analysis.enablePythonFiles": true,
  "sage.analysis.sourceRoots": ["/path/to/sage/src"],
  "sage.interpreter.path": "/path/to/sage"
}
```

The extension runs in the VS Code workspace extension host because it starts local language-server and optional Sage
runtime processes. In untrusted or virtual workspaces, syntax browsing remains available while runtime/indexing features
are intentionally limited.

## Important Settings

- `sage.interpreter.path`: Sage executable for run commands, REPL, and runtime documentation fallback.
- `sage.languageServer.rustPath`: Rust language-server binary path; `auto` prefers packaged or local binaries.
- `sage.analysis.sourceRoots`: Sage or project source roots to index.
- `sage.analysis.extraPaths`: Extra import/search roots.
- `sage.analysis.enablePythonFiles`: Attach Sage LSP features to ordinary `.py` files.
- `sage.analysis.enableRuntimeIntrospection`: Allow bounded Sage runtime documentation fallback.
- `sage.docs.preferredSource`: Choose `auto`, `workspace`, `runtime`, or `reference` documentation behavior.
- `sage.docs.showOnHover`: Control whether hover includes documentation previews.

## Troubleshooting

Use these commands before filing an issue:

- `Sage: Show Environment Details`
- `Sage: Show Index Status`
- `Sage: Show Docs Status`
- `Sage: Find References`
- `Sage: Run UX Self Check`
- `Sage: Copy Support Bundle`

For slow hover, definition, completion, or documentation, include the support bundle plus the affected symbol and whether
the query is a cold start or warm query. For navigation bugs, include the file type and the expected Sage source target.

## Preview Limitations

- Packaged native binaries are staged per platform; cross-platform signing and marketplace upload automation are still
  deferred.
- Notebook and kernel UX are not part of this preview.
- Pyright sidecar integration and deeper `.sage.py` overlay behavior remain future work.
- The legacy Python LSP remains in the repository as a migration and regression baseline, but Rust is the primary runtime
  path.

## Support and Security

Use the repository issue templates for bugs, performance regressions, and feature requests. See `SUPPORT.md` for the
expected diagnostic information and `SECURITY.md` for vulnerability reporting. Do not post exploit details or private
workspace contents in public issues.
