# Install and Configure

## Bootstrap Setup

1. Install Node.js and Python 3.9 or newer.
2. From the repository root, run `npm install`.
3. Install the Python language server package with `python -m pip install -e ./packages/sage-lsp[dev]`.
4. Sync syntax assets with `npm run sync:syntax`.
5. Build the workspace with `npm run build`.

Note: this Python environment is the default host for the language server itself. It does not need to match the Sage
executable used to run `.sage` files.

## One-Command VS Code Setup

- Run `npm run dev:vscode` to sync syntax assets, build the repository, and open the plugin repository in VS Code.
- Run `npm run dev:vscode:smoke` to do the same prep work and then remind yourself to launch the smoke-workspace
  extension-host configuration with `F5`.
- Run `./scripts/dev-vscode.sh --bootstrap --python /path/to/python` when you want the helper script to install Node
  dependencies and editable-install `sage-lsp` before opening VS Code.

## Extension Development Host

1. Open the repository in VS Code.
2. Press `F5` and choose `Sage Plugin: Extension Host` for the repository itself, or `Sage Plugin: Smoke Workspace` to open the ready-made sample workspace under `examples/manual-smoke-workspace`.
3. The repository-level `build` task runs automatically before the extension host starts.
4. Use `Sage: Select Interpreter` to point the extension at the Sage executable used by run commands, the managed REPL terminal, and future Sage-aware runtime context.
5. Leave `sage.languageServer.pythonPath = auto` to use the active Python environment for the language server, or set it explicitly if VS Code cannot find the right Python on its own.
6. Pick `sage.run.target = terminal` to run files as standalone commands, or `sage.run.target = repl` to load the current file into the managed Sage REPL with `load(...)`.
7. Toggle `sage.docs.showOnHover` if you want hover popups to show either the short signature only or the full documentation preview.
8. Open or create a `.sage` file to exercise hover, completion, definition, document symbols, and docs requests.

## Runtime Split

- `sage.interpreter.path`
  Controls the Sage executable used for `Run Current File`, `Run Selection`, and `Start REPL`.
- `sage.languageServer.pythonPath`
  Controls the Python executable used to run `sage-lsp` itself.
- Why this split exists:
  many Sage distributions bundle an older Python or omit `pygls` and `lsprotocol`, so the language server must be
  able to run in a normal Python environment while still targeting Sage for execution.

If the language server still fails to start, point `sage.languageServer.pythonPath` at the Python where you installed
`sage-lsp`, `pygls`, and `lsprotocol`.

## Manual Smoke Workspace

- A ready-made manual test workspace lives in `examples/manual-smoke-workspace`.
- It includes `.sage`, `.py`, `.pyx`, `.pxd`, and `.pxi` files plus workspace-local settings for `sourceRoots` and `extraPaths`.
- Use the `Sage Plugin: Smoke Workspace` launch configuration to open it directly in the extension host.

## Current Limits

- Static analysis is intentionally reduced and fixture-backed; it is not yet a full Sage runtime model.
- Source mapping only handles the first `.sage` transform slice.
- Runtime Sage interpreter discovery is still manual.
- Notebook and kernel integration are not wired yet.
