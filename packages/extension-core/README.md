# Sage VS Code Extension

SageMath support for VS Code: `.sage` files, Sage-heavy Python, and Sage's Cython sources, backed by a fast Rust
language server. Extension identifier: `sage-vscode.sage-vscode-extension`.

## What It Provides

- Hover with rendered Sage documentation, completion, signature help, and inlay hints.
- Go to definition, declaration, and implementation; references, rename, call hierarchy, document and workspace
  symbols, semantic highlighting, folding, and syntax diagnostics.
- Navigation into the Sage library from its sources; Sage does not have to be installed for editing.
- Run Current File, Run Selection in REPL (`Shift+Enter`), Run Cell (`Ctrl+Enter` / `Cmd+Enter` in files with `# %%`
  cells), and a managed Sage REPL per project.
- English and Simplified Chinese.

## First Run

1. Open a trusted folder containing a `.sage` file. The `Sage:` status bar item shows startup and indexing; click it
   when something is not ready.
2. Hover a name, press `F12`, or run `Sage: Run Current File`.
3. If Sage is not found, run `Sage: Select Sage Interpreter`.
4. For Sage-heavy `.py` files or Cython projects, run `Sage: Configure Workspace` and pick the matching profile.

`Sage: Open Getting Started` walks through these steps. When a method name exists on several classes and the
receiver's type is unknown, Go to Definition shows the candidates instead of guessing, and rename stays disabled.

## Important Settings

| Setting | Use |
| --- | --- |
| `sage.interpreter.path` | Sage executable for running code, the REPL, and runtime documentation |
| `sage.analysis.sourceRoots` | Sage source trees to index when automatic discovery picks the wrong one |
| `sage.analysis.enablePythonFiles` | attach the server to `.py` files in Sage-heavy Python projects |
| `sage.docs.preferredSource` | prefer `workspace`, `runtime`, or `reference` documentation |

The full list is in the repository's User Guide.

## Troubleshooting

`Sage: Show Environment Details`, `Sage: Show Index Status`, and `Sage: Show Docs Status` show what the server is
using. `Sage: Run UX Self Check` tests every editor feature at the cursor; `Sage: Find References` also works from
read-only Sage library files. For bug reports attach `Sage: Copy Support Bundle` (settings and status, no source code).

In untrusted or virtual workspaces only syntax highlighting is available.

## Limitations

- No notebook or Jupyter kernel integration.
- Distributed through GitHub releases; not signed or published to the Marketplace.

## Support and Security

Use the repository issue templates. See `SUPPORT.md` for what to include and `SECURITY.md` for reporting
vulnerabilities; do not post exploit details or private workspace contents in public issues.
