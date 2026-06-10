# Install and Configure

## Fast Path

Use this path when you want to try the extension quickly from this repository:

```bash
npm install
npm run package:vsix
npm run configure:workspace -- --workspace /path/to/your/project --profile auto
code --install-extension dist/sage-vscode-extension-0.1.0.vsix --force
```

Open your Sage workspace in VS Code, then run:

1. `Sage: Open Getting Started`
2. `Sage: Select Interpreter`
3. `Sage: Configure Workspace`
4. `Sage: Show Environment Details`
5. `Sage: Show Index Status`

For Sage-heavy `.py` files, choose the `Sage-heavy Python workspace` profile. For mixed `.sage`, `.py`, and Cython
projects, choose `Full Sage research workspace`.

## Minimal Settings

Most local Sage checkouts only need these settings:

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

Use an explicit `sage.analysis.sourceRoots` entry for predictable Sage source indexing and fast definition jumps into the
Sage library. Leave it empty only when you want automatic nearby/runtime discovery.

## Development Bootstrap

1. Install Node.js, Rust/Cargo, and Python 3.9 or newer.
2. From the repository root, run `npm install`.
3. Build the Rust language server with `npm run build:rust`.
4. Sync syntax assets with `npm run sync:syntax`.
5. Build the workspace with `npm run build`.
6. Optional: install the legacy Python LSP test package with `python -m pip install -e ./packages/sage-lsp[dev]`.

Note: the primary language server is the Rust `sage-ls` binary. Python is still used by the legacy test suite and by
optional runtime-backed Sage documentation probes, but it is no longer the default LSP transport.

## One-Command Development Setup

- Run `npm run dev:vscode` to sync syntax assets, build the repository, and open the plugin repository in VS Code.
- Run `npm run dev:vscode:smoke` to do the same prep work and then print the GUI checks for launching the smoke
  workspace in an extension-development host.
- Run `./scripts/dev-vscode.sh --bootstrap --python /path/to/python` when you want the helper script to install Node
  dependencies and editable-install the legacy `sage-lsp` package before opening VS Code.
- Run `npm run configure:workspace -- --workspace /path/to/project --profile auto` when you want a cross-platform
  command-line setup for `.sage`, Sage-heavy Python, Cython, or mixed research workspaces before opening VS Code. Add
  `-- --sage /path/to/sage --source-root /path/to/sage/src` when Sage is not on `PATH`.
- The sync and dev helper path is compatible with Node 20 and newer; it no longer depends on `import.meta.dirname`.

## Extension Development Host

1. Open the repository in VS Code.
2. Press `F5`. `Sage Plugin: Smoke Workspace` is the first/default launch target for GUI smoke testing; choose
   `Sage Plugin: Extension Host` only when you specifically want the repository itself in the host.
3. The repository-level `build` task runs automatically before the extension host starts.
4. In the new `[Extension Development Host]` window, open `src/01_hover_and_definition.sage` and confirm the status bar
   language mode is `SageMath`, not `Plain Text`.
5. Confirm the left status bar contains `Sage: ...`, then run `Sage: Show Index Status` or `Sage: Run UX Self Check`
   from the command palette before deeper inspection.
6. If the host opens with no folder, use `Open Folder` inside that `[Extension Development Host]` window and select
   `examples/manual-smoke-workspace`.
7. If `.sage` files show `Plain Text` or Sage commands are missing, you are in a normal VS Code window rather than the
   extension-development host; close it and start again from the repository with `F5`.
8. Do not use a direct `code --extensionDevelopmentPath=...` command as the manual smoke path. It is not a reliable
   user-facing VS Code CLI flow and can open a normal window where the development extension is not loaded.
9. Use `Sage: Open Getting Started` when you want the guided first-run path inside VS Code. It walks through interpreter
   selection, workspace profile setup, index/docs status, and UX self-check.
10. Use `Sage: Configure Workspace` to write a conservative workspace profile. Choose `Sage-heavy Python workspace` for
   `.py` projects that import `sage.all`, `Sage native/Cython workspace` for `.pyx/.pxd/.pxi`, or `Full Sage research
   workspace` for mixed projects.
11. Use `Sage: Select Interpreter` to pick a complete detected environment first.
12. The preferred local-development path is `Local Sage development environment`, which points run commands and runtime
   docs at a nearby Sage checkout such as `.../sage/sage`.
13. The preferred stable-runtime path is `System Sage (stable)`, which points run commands and runtime-backed docs at an
   installed Sage executable.
14. Leave `sage.languageServer.rustPath = auto` for local development so the extension uses `target/debug/sage-ls` after
   `npm run build:rust`; set it only when you want a specific binary.
15. The picker still includes advanced actions for custom Sage runtimes, custom legacy Python paths, and an explicit
   `auto` reset entry.
16. Pick `sage.run.target = terminal` to run files as standalone commands, or `sage.run.target = repl` to load the current file into the managed Sage REPL with `load(...)`.
17. Set `sage.run.cleanupGeneratedPython = true` if you want terminal or managed-REPL `.sage` runs to remove generated `.sage.py` helper files automatically on POSIX shells.
18. Toggle `sage.docs.showOnHover` if you want hover popups to show either the short signature only or the full documentation preview.
19. Leave `sage.analysis.sourceRoots` empty to index the workspace plus nearby/runtime-discovered Sage roots, or set it
   to explicit roots when you want deterministic smoke fixtures. Configured roots are still supplemented with nearby
   and interpreter-discovered Sage roots.
20. Leave `sage.analysis.enableRuntimeIntrospection = true` if you want the server to fall back to a live Sage runtime for docs, definition jumps, and signature help when static indexing misses a symbol.
21. Open or create a `.sage` file to exercise hover, completion, definition, references, rename, signature help, document symbols,
   workspace symbols, diagnostics, and docs requests.

## Runtime Split

- `sage.interpreter.path`
  Controls the Sage executable used for `Run Current File`, `Run Selection`, and `Start REPL`.
- `sage.languageServer.pythonPath`
  Legacy compatibility setting for old workspaces and Python LSP tests. The primary server path is
  `sage.languageServer.rustPath`.
- `sage.languageServer.rustPath`
  Controls the Rust `sage-ls` executable. `auto` prefers `SAGE_LS_PATH`, local `target/debug/sage-ls`, local
  `target/release/sage-ls`, then `sage-ls` on `PATH`.
- `Sage: Select Interpreter`
  Presents environment-first choices before the advanced path actions. The main built-in profiles are:
- `Sage: Configure Workspace`
  Writes a small workspace profile for standard Sage, Sage-heavy Python, native Cython, or mixed research projects.
  The command keeps `sage.languageServer.rustPath = auto` so future local rebuilds or packaged server updates are picked
  up without rewriting the workspace by hand.
  `Local Sage development environment` for a nearby checkout, and `System Sage (stable)` for an installed standalone
  Sage executable.
- `sage.analysis.sourceRoots`
  When set, these roots seed the project index. The extension then supplements them with nearby Sage checkouts and
  roots inferred from the selected interpreter.
- `sage.analysis.enableRuntimeIntrospection`
  Defaults to `true` and lets the language server query the selected Sage runtime for documentation, source locations,
  and signatures when static analysis alone is not enough.
- `sage.run.cleanupGeneratedPython`
  Defaults to `false`. When enabled, standalone terminal runs and managed-REPL file loads of `.sage` files remove the
  generated `.sage.py` helper file automatically after the command finishes on POSIX shells.
- Why this split exists:
  the editor needs a fast native LSP process while run commands and runtime-backed docs still target the selected Sage
  executable.

## Command Reference

- `npm run export:reference -- --workspace /path/to/project --source-root /path/to/sage/src`: generate
  `/path/to/project/.sage-reference/index.html`, a static offline reference viewer for sharing symbol docs, definitions,
  references, and source snapshots without VS Code, Sage, this extension, or a local server.
- `sage.openGettingStarted`: open the VS Code Getting Started walkthrough for first-run setup.
- `sage.selectInterpreter`: choose a detected Sage runtime profile or a custom path.
- `sage.configureWorkspace`: write a conservative workspace profile for standard Sage, Sage-heavy Python, native Cython,
  or full research projects.
- `sage.restartLanguageServer`: restart the Rust language server after rebuilding `sage-ls` or changing analysis settings.
- `sage.runCurrentFile`: run the active `.sage` file with the selected Sage runtime.
- `sage.runSelection`: run the current editor selection with the selected Sage runtime.
- `sage.startRepl`: start or reveal the managed Sage REPL terminal.
- `sage.showDocumentation`: open the documentation panel for the active symbol.
- `sage.findReferences`: find Sage-aware references for the active symbol, including workspace usages when the cursor is in
  an indexed external Sage source definition.
- `sage.showEnvironmentDetails`: show interpreter, source-root, runtime-introspection, and language-server details.
- `sage.showIndexStatus`: show indexed file, symbol, doc, cache, and pending-job counts.
- `sage.showDocsStatus`: show static docs and runtime docs worker health.
- `sage.copySupportBundle`: copy a reviewable JSON diagnostics bundle with paths/settings/status but no source contents,
  selected text, or environment variables.
- `sage.runUxSelfCheck`: run hover, docs, definition, completion, references, rename, signature, and diagnostics checks
  for the active editor position.
- `sage.rebuildIndex`: rebuild the Rust source index for the current workspace.

## Setting Reference

- `sage.interpreter.path`: Sage executable used by run commands, REPL startup, and runtime-backed docs.
- `sage.interpreter.args`: extra arguments passed to the Sage executable.
- `sage.languageServer.pythonPath`: legacy Python LSP runtime path retained for older workspaces and migration tests.
- `sage.languageServer.rustPath`: Rust `sage-ls` path; leave it as `auto` for local development.
- `sage.languageServer.pythonArgs`: extra arguments passed to the legacy Python language server.
- `sage.analysis.mode`: analysis depth, one of `light`, `default`, or `full`.
- `sage.analysis.extraPaths`: additional import paths supplied to the language server.
- `sage.analysis.sourceRoots`: explicit source roots to index before discovered workspace and runtime roots.
- `sage.analysis.enableDiagnostics`: enable or suppress language-server diagnostics.
- `sage.analysis.enableRuntimeIntrospection`: allow live Sage runtime fallback for docs, definitions, and signatures.
- `sage.analysis.enablePyxParsing`: enable lightweight `.pyx`, `.pxd`, and `.pxi` indexing.
- `sage.analysis.enablePythonFiles`: attach the Sage language server to ordinary `.py` files while keeping Python language mode.
  Keep this disabled for general Python workspaces; enable it for Sage-heavy Python projects that import `sage.all`.
  The extension may activate on Python files, but it does not start the Sage LSP or show Sage editor commands for ordinary
  Python files unless this setting or explicit Sage source roots are configured.
- `sage.indexing.exclude`: glob patterns excluded from indexing.
- `sage.docs.preferredSource`: documentation source preference, one of `auto`, `workspace`, `runtime`, or `reference`.
- `sage.docs.showOnHover`: show documentation previews in hover content when available.
- `sage.logging.level`: extension and language-client log verbosity.
- `sage.run.target`: choose standalone terminal execution or managed REPL loading.
- `sage.run.cleanupGeneratedPython`: remove generated `.sage.py` helpers after POSIX terminal or managed-REPL file runs when enabled.
- `sage.experimental.notebookSupport`: reserved preview switch for future notebook integration.

## Recommended Profiles

- Local Sage development:
  choose `Local Sage development environment` when you work against a nearby `sage` checkout.
- Stable installed Sage:
  choose `System Sage (stable)` when you want run commands and runtime-backed docs to target the installed Sage
  executable instead of the mutable checkout.
- Advanced overrides:
  only fall back to the custom path entries when the automatic environment profiles do not match your machine.

If the language server still fails to start, run `npm run build:rust`, keep `sage.languageServer.rustPath = auto`, and
check the `Sage Language Server` output channel for the resolved binary path.

## Manual Smoke Workspace

- A ready-made manual test workspace lives in `examples/manual-smoke-workspace`.
- It includes `.sage`, `.py`, `.pyx`, `.pxd`, and `.pxi` files plus workspace-local settings for `sourceRoots` and `extraPaths`.
- Advanced files now cover graph constructors, elliptic curves, polynomial ideals, symbolic integration, combinatorics,
  and dotted runtime lookups that rely on runtime-backed docs or signature help.
- Use the `Sage Plugin: Smoke Workspace` launch configuration to open it in the extension host. A correct GUI session has
  a `[Extension Development Host]` window title, `SageMath` language mode for `.sage`, Sage commands in the command
  palette, and a left status-bar item beginning with `Sage:`.
- The automated `npm run test:extension-host` path reuses a copied version of this workspace in an unattended local VS
  Code session so it can validate real extension behavior without mutating repository fixtures.
- The Browser Use workbench is available through `npm run debug:web`; it exposes scopes, semantic spans, diagnostics,
  symbols, index/docs status, and UX matrix checks in a browser surface suitable for MCP-driven inspection.
- The release performance gate is available through `npm run test:performance`. Set `SAGE_SOURCE_ROOT=/path/to/sage/src`
  when the Sage checkout is not beside this repository; use `-- --skip-workbench` to run only the release index budgets.

## Offline Reference Package

Use the offline reference export when you need a read-only project reference that another person can open locally without
installing the Sage VS Code plugin or Sage:

```bash
npm run export:reference -- --workspace /path/to/project --source-root /path/to/sage/src
```

The generated `.sage-reference/` directory contains only static files:

- `index.html`: browser entrypoint.
- `assets/viewer.css` and `assets/viewer.js`: light/dark interactive viewer.
- `data/manifest.js`: project name, generated time, and counts.
- `data/symbols.js`: symbol summaries, search index, references, and source metadata.
- `data/sources/source-*.js`: lazily loaded source snapshots or snippets.

The viewer is designed for quick reading and navigation: global search, grouped symbol list, source browser with line
numbers, definition/reference highlights, documentation panel, recent history, copy actions, theme toggle, and URL hashes
for sharing state. It is intentionally read-only; use VS Code for editing, rename, diagnostics, and live LSP behavior.

By default, project source is snapshotted and Sage source is limited to related files. Use `--source-mode snippets` to
reduce Sage source size, or `--source-mode none` when you only want symbol metadata and docs. The exporter writes virtual
paths such as `project://src/main.py` and `sage://sage/rings/...`, sanitizes source/doc text, and fails if a generated
package contains private local home paths.

## Current Limits

- Static analysis is broad enough for the current smoke fixtures and common Sage API lookup paths, but it is not a full
  Sage runtime model.
- Source mapping still focuses on the implemented `.sage` preprocessing slices and should be extended cautiously.
- Interpreter discovery now prioritizes local `sage` checkout plus `sage-dev` and installed system Sage profiles, but unusual layouts may still require the advanced custom-path actions.
- Runtime-derived source-root discovery covers common source and site-packages layouts, but unusual Sage packaging may still need manual `sage.analysis.sourceRoots`.
- Runtime fallback currently focuses on docs, definitions, and signature help; completion and richer semantic analysis still rely on static indexing.
- Notebook and kernel integration are not wired yet.
- Marketplace-ready native binary packaging and signing are not wired yet.
