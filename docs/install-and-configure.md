# Install and Configure

## Bootstrap Setup

1. Install Node.js and Python 3.11 or newer.
2. From the repository root, run `npm install`.
3. Install the Python language server package with `python -m pip install -e ./packages/sage-lsp[dev]`.
4. Sync syntax assets with `npm run sync:syntax`.
5. Build the workspace with `npm run build`.

## Extension Development Host

1. Open the repository in VS Code.
2. Launch the extension development configuration.
3. Use `Sage: Select Interpreter` to point the extension at the Sage executable used by run and REPL commands.
4. Open or create a `.sage` file to exercise hover, completion, definition, document symbols, and docs requests.

## Current Limits

- Static analysis is intentionally reduced and fixture-backed; it is not yet a full Sage runtime model.
- Source mapping only handles the first `.sage` transform slice.
- Runtime Sage interpreter discovery is still manual.
- Notebook and kernel integration are not wired yet.
