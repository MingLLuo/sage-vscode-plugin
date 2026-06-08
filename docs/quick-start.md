# Quick Start

This is the shortest path to use the Sage VS Code extension from this repository.

## Use the Extension Locally

```bash
npm install
npm run package:vsix
npm run doctor:mac
code --install-extension dist/sage-vscode-extension-0.1.0.vsix --force
```

Then open your Sage workspace in VS Code.

## First VS Code Setup

1. Run `Sage: Open Getting Started`.
2. Run `Sage: Select Interpreter` and choose a Sage executable.
3. Run `Sage: Configure Workspace`.
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

Run these commands from the VS Code command palette:

- `Sage: Show Environment Details`
- `Sage: Show Index Status`
- `Sage: Show Docs Status`
- `Sage: Run UX Self Check`

In a working setup, hover, definition, completion, references, rename preview, signature help, diagnostics, symbols, and
documentation should respond from the Rust language server. Cold indexing can take longer; warm hover and definition
queries should be fast after the index is hydrated.

## Develop the Extension Without Installing

```bash
npm install
npm run dev:vscode:smoke
```

Press `F5` in the opened repository window and choose `Sage Plugin: Smoke Workspace`. Use this path when you are changing
the extension itself.
