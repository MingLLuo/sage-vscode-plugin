# Quick Start

This is the shortest path to use the Sage VS Code extension from this repository.

Use Node.js 22.9 or newer (Node.js 26 is supported) and npm 11 or newer. `.node-version` selects the Node 22 baseline
without capping newer releases. Packaging verifies the installed npm release supports the selected Node.js runtime;
the repository's `rust-toolchain.toml` pins Rust.

## Use the Extension Locally

```bash
npm ci
npm run package:vsix   # writes dist/sage-vscode-extension-0.2.0-<platform>.vsix for this machine
npm run doctor:mac
code --install-extension dist/sage-vscode-extension-0.2.0-darwin-arm64.vsix --force
```

Use the file that matches your platform (`darwin-arm64`, `darwin-x64`, `linux-x64`, `linux-arm64`, `alpine-x64`,
`alpine-arm64`, `win32-x64`, `win32-arm64`). Prebuilt packages for every platform are attached to each tagged GitHub
release and to every run of the **Platform packages** workflow.

Open a trusted local folder with a `.sage` file in VS Code. The extension uses the packaged language server and
discovers nearby Sage source roots. Hover a local name or use Go to Definition, then run **Sage: Run Current File** from
the command palette. Select a Sage executable with **Sage: Select Sage Interpreter** only if it is not found automatically.
Click the Sage status bar item for details if indexing or the language server does not become ready.

## First VS Code Setup

For `.sage` files, no workspace settings are required. For Sage-heavy `.py` or Cython projects, run **Sage: Configure
Workspace** once and pick the closest profile:

- `Sage-heavy Python workspace` for `.py` files that import `sage.all`.
- `Sage native/Cython workspace` for `.pyx`, `.pxd`, or `.pxi`.
- `Full Sage research workspace` for mixed projects.

Open the corresponding file and check the left status bar item beginning with `Sage:`.

The optional **Sage: Open Getting Started** walkthrough explains interpreter choice and diagnostics. To configure
before opening VS Code, use `npm run configure:workspace -- --workspace /path/to/project --profile auto`; do not also
run the editor configuration command for that project.

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
The doctor verifies that the selected Sage command can import `sage.all`; printing a version alone is not considered a
usable runtime. A source-only checkout can still provide accurate static navigation while runtime-backed help is shown
as degraded until Sage is built.

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
