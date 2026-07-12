# Quick Start

This is the shortest path to use the Sage VS Code extension from this repository.

Select the exact Node version in `.node-version` before packaging; the repository's `rust-toolchain.toml` pins Rust.

## Use the Extension Locally

```bash
npm ci
npm run package:vsix
npm run doctor:mac
npm run configure:workspace -- --workspace /path/to/your/project --profile auto
code --install-extension dist/sage-vscode-extension-0.1.0.vsix --force
```

Then open your Sage workspace in VS Code.

## First VS Code Setup

1. Run `npm run configure:workspace -- --workspace /path/to/your/project --profile auto` before opening VS Code, or run
   `Sage: Configure Workspace` from the command palette after opening it.
2. Run `Sage: Open Getting Started`.
3. Run `Sage: Select Interpreter` if the script did not find Sage automatically.
4. Pick the closest profile:
   - `Standard Sage workspace` for `.sage` files.
   - `Sage-heavy Python workspace` for `.py` files that import `sage.all`.
   - `Sage native/Cython workspace` for `.pyx`, `.pxd`, or `.pxi`.
   - `Full Sage research workspace` for mixed projects.
5. Open a `.sage`, Sage-heavy `.py`, or Cython file.
6. Check the left status bar item beginning with `Sage:`.

## Minimal Workspace Settings

For a Sage-heavy Python project, this is usually enough:

```json
{
  "sage.languageServer.rustPath": "auto",
  "sage.analysis.enablePythonFiles": true,
  "sage.analysis.sourceRoots": [
    "/path/to/sage/src"
  ],
  "sage.interpreter.path": "/path/to/sage"
}
```

Leave `sage.analysis.sourceRoots` empty if you want the extension to discover nearby Sage roots automatically. Set it
explicitly when you want faster and more predictable indexing.

## Check That It Works

Before opening VS Code, `npm run doctor:mac` should report `ready` or `usable-with-warnings`. If it reports
`action-needed`, run the listed command, usually `npm run package:vsix`, then run the doctor again.

For projects outside this repository, the fastest fix is usually:

```bash
npm run configure:workspace -- --workspace /path/to/project --profile python --sage /path/to/sage --source-root /path/to/sage/src
```

Run these commands from the VS Code command palette:

- `Sage: Show Environment Details`
- `Sage: Show Index Status`
- `Sage: Show Docs Status`
- `Sage: Run UX Self Check`

In a working setup, hover, definition, completion, references, rename preview, signature help, diagnostics, symbols, and
documentation should respond from the Rust language server. Cold indexing can take longer; warm hover and definition
queries should be fast after the index is hydrated.

## Share an Offline Reference

To share a readable project reference without requiring VS Code, Sage, or this extension on another machine:

```bash
npm run export:reference -- --workspace /path/to/project --source-root /path/to/sage/src
```

Open `/path/to/project/.sage-reference/index.html` in a browser. The offline reference viewer includes search, symbol
details, documentation, definitions, references, source snippets, light/dark theme, and URL hashes such as
`#symbol=PolynomialRing`. It stores virtual paths only, not local home paths.

## Develop the Extension Without Installing

```bash
npm install
npm run dev:vscode:smoke
```

Press `F5` in the opened repository window and choose `Sage Plugin: Smoke Workspace`. Use this path when you are changing
the extension itself.
