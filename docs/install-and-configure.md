# Install and Configure

## Bootstrap Setup

1. Install Node.js and Python 3.11 or newer.
2. From the repository root, run `npm install`.
3. Install the Python language server package with `python -m pip install -e ./packages/sage-lsp[dev]`.
4. Sync syntax assets with `npm run sync:syntax`.
5. Build the workspace with `npm run build`.

## Extension Development Host

1. Open the repository in VS Code.
2. Press `F5` and choose `Sage Plugin: Extension Host` for the repository itself, or `Sage Plugin: Smoke Workspace` to open the ready-made sample workspace under `examples/manual-smoke-workspace`.
3. The repository-level `build` task runs automatically before the extension host starts.
4. Use `Sage: Select Interpreter` to point the extension at the Sage executable used by the language server, run commands, and the managed REPL terminal.
5. Pick `sage.run.target = terminal` to run files as standalone commands, or `sage.run.target = repl` to load the current file into the managed Sage REPL with `load(...)`.
6. Toggle `sage.docs.showOnHover` if you want hover popups to show either the short signature only or the full documentation preview.
7. Open or create a `.sage` file to exercise hover, completion, definition, document symbols, and docs requests.

## Manual Smoke Workspace

- A ready-made manual test workspace lives in `examples/manual-smoke-workspace`.
- It includes `.sage`, `.py`, and `.pyx` files plus workspace-local settings for `sourceRoots` and `extraPaths`.
- Use the `Sage Plugin: Smoke Workspace` launch configuration to open it directly in the extension host.

## Current Limits

- Static analysis is intentionally reduced and fixture-backed; it is not yet a full Sage runtime model.
- Source mapping only handles the first `.sage` transform slice.
- Runtime Sage interpreter discovery is still manual.
- Notebook and kernel integration are not wired yet.
