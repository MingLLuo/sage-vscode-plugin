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
- The sync and dev helper path is compatible with Node 20 and newer; it no longer depends on `import.meta.dirname`.

## Extension Development Host

1. Open the repository in VS Code.
2. Press `F5` and choose `Sage Plugin: Extension Host` for the repository itself, or `Sage Plugin: Smoke Workspace` to open the ready-made sample workspace under `examples/manual-smoke-workspace`.
3. The repository-level `build` task runs automatically before the extension host starts.
4. Use `Sage: Select Interpreter` to pick from detected Sage runtimes, detected Python environments, or custom paths.
5. Sage selections update `sage.interpreter.path` for run commands, the managed REPL terminal, and future Sage-aware runtime context.
6. Python selections update `sage.languageServer.pythonPath` for the language server; the picker also includes an explicit `auto` reset entry when you want VS Code to go back to automatic Python selection.
7. Pick `sage.run.target = terminal` to run files as standalone commands, or `sage.run.target = repl` to load the current file into the managed Sage REPL with `load(...)`.
8. Toggle `sage.docs.showOnHover` if you want hover popups to show either the short signature only or the full documentation preview.
9. Leave `sage.analysis.sourceRoots` empty if you want the extension to infer Sage library roots from the selected interpreter path; this is now the default path for docs, definitions, and navigation into Sage itself.
10. Leave `sage.analysis.enableRuntimeIntrospection = true` if you want the server to fall back to a live Sage runtime for docs, definition jumps, and signature help when static indexing misses a symbol.
11. Open or create a `.sage` file to exercise hover, completion, definition, references, rename, signature help, document symbols,
   workspace symbols, diagnostics, and docs requests.

## Runtime Split

- `sage.interpreter.path`
  Controls the Sage executable used for `Run Current File`, `Run Selection`, and `Start REPL`.
- `sage.languageServer.pythonPath`
  Controls the Python executable used to run `sage-lsp` itself.
- `Sage: Select Interpreter`
  Presents detected Sage runtimes from PATH and common system locations, plus detected Python environments from PATH,
  workspace virtual environments, and common local installs such as `miniforge3` or `conda` roots.
- `sage.analysis.sourceRoots`
  When set, these roots are indexed exactly as configured. When left empty, the extension combines workspace-local
  roots with Sage roots inferred from the selected interpreter.
- `sage.analysis.enableRuntimeIntrospection`
  Defaults to `true` and lets the language server query the selected Sage runtime for documentation, source locations,
  and signatures when static analysis alone is not enough.
- Why this split exists:
  many Sage distributions bundle an older Python or omit `pygls` and `lsprotocol`, so the language server must be
  able to run in a normal Python environment while still targeting Sage for execution.

If the language server still fails to start, point `sage.languageServer.pythonPath` at the Python where you installed
`sage-lsp`, `pygls`, and `lsprotocol`.

## Manual Smoke Workspace

- A ready-made manual test workspace lives in `examples/manual-smoke-workspace`.
- It includes `.sage`, `.py`, `.pyx`, `.pxd`, and `.pxi` files plus workspace-local settings for `sourceRoots` and `extraPaths`.
- Advanced files now cover graph constructors, elliptic curves, polynomial ideals, symbolic integration, combinatorics,
  and dotted runtime lookups that rely on runtime-backed docs or signature help.
- Use the `Sage Plugin: Smoke Workspace` launch configuration to open it directly in the extension host.

## Current Limits

- Static analysis is intentionally reduced and fixture-backed; it is not yet a full Sage runtime model.
- Source mapping only handles the first `.sage` transform slice.
- Interpreter discovery covers PATH, common system locations, workspace virtual environments, and common local Python installs, but it is not exhaustive yet.
- Runtime-derived source-root discovery covers common source and site-packages layouts, but unusual Sage packaging may still need manual `sage.analysis.sourceRoots`.
- Runtime fallback currently focuses on docs, definitions, and signature help; completion and richer semantic analysis still rely on static indexing.
- Notebook and kernel integration are not wired yet.
