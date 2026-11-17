# User Guide

## Install a Released Package

Download the file for your machine from the
[latest release](https://github.com/MingLLuo/sage-vscode-plugin/releases/latest). Each file bundles its own language
server, so no Rust, Node.js, or build step is needed. VS Code 1.97 or newer is required.

| Machine | File |
| --- | --- |
| macOS, Apple silicon | `sage-vscode-extension-<version>-darwin-arm64.vsix` |
| macOS, Intel | `sage-vscode-extension-<version>-darwin-x64.vsix` |
| Windows x64 / Arm64 | `...-win32-x64.vsix` / `...-win32-arm64.vsix` |
| Linux x64 / Arm64 (any glibc or musl distribution) | `...-linux-x64.vsix` / `...-linux-arm64.vsix` |
| Alpine Linux x64 / Arm64 | `...-alpine-x64.vsix` / `...-alpine-arm64.vsix` |

Install it from the command palette with **Extensions: Install from VSIX...**, or from a terminal:

```bash
code --install-extension sage-vscode-extension-0.2.0-darwin-arm64.vsix
```

**Remote SSH, WSL, and Dev Containers.** The extension runs where your files are. Open the remote window first, then
run **Extensions: Install from VSIX...** in that window and pick the file for the remote machine (usually
`linux-x64`), not the one for your laptop.

**Updating.** Install the newer file the same way; VS Code replaces the old version. The index cache format is checked
on startup and rebuilt automatically when it changes.

**Sage itself is optional for navigation.** Hover, go to definition, references, rename, and diagnostics work from Sage
source files. Running files, the REPL, and runtime-backed documentation need a Sage installation; the extension finds
it on `PATH` or asks you to select it with **Sage: Select Sage Interpreter**.

## First Steps

1. Open a trusted folder that contains a `.sage` file. The status bar item that starts with `Sage:` shows startup and
   indexing; click it for details and recovery actions.
2. Hover a name, press `F12` to go to its definition, or `Shift+F12` for references.
3. Run **Sage: Run Current File**, select lines and press `Shift+Enter`, or split a file into `# %%` cells and press
   `Ctrl+Enter` (`Cmd+Enter` on macOS).
4. **Sage: Open Getting Started** walks through the same steps.

### Project types

`.sage` files need no settings. For other projects run **Sage: Configure Workspace** once and pick a profile:

| Profile | Use for |
| --- | --- |
| Standard Sage workspace | mostly `.sage` worksheets and scripts |
| Sage-heavy Python workspace | `.py` files that import `sage.all` (turns on `sage.analysis.enablePythonFiles`) |
| Sage native/Cython workspace | `.pyx`, `.pxd`, `.pxi` sources |
| Full Sage research workspace | a mix of the above |

From a terminal, `npm run configure:workspace -- --workspace /path/to/project --profile auto` writes the same settings
(add `--sage /path/to/sage --source-root /path/to/sage/src` when Sage is not on `PATH`).

### When Sage or its sources are not found

- **Sage: Select Sage Interpreter** lists detected Sage installations, or lets you enter a path. It sets
  `sage.interpreter.path`.
- Set `sage.analysis.sourceRoots` to your Sage `src` directory when automatic discovery picks the wrong tree or you
  want predictable indexing:

```json
{
  "sage.interpreter.path": "/path/to/sage",
  "sage.analysis.sourceRoots": ["/path/to/sage/src"]
}
```

### How navigation decides

Go to Definition jumps only when the target is certain. When a method name exists on several classes and the receiver's
type is unknown, VS Code shows the candidates instead, and references and rename stay unavailable rather than editing
the wrong method. Assigning the receiver from a known constructor or keeping its import visible lets the index narrow
the target.

## Commands

| Command | ID | What it does |
| --- | --- | --- |
| Open Getting Started | `sage.openGettingStarted` | first-run walkthrough |
| Select Sage Interpreter | `sage.selectInterpreter` | choose the Sage used for running code and runtime docs |
| Configure Workspace | `sage.configureWorkspace` | write a project profile (see above) |
| Restart Language Server | `sage.restartLanguageServer` | restart `sage-ls` |
| Run Current File | `sage.runCurrentFile` | run the file in a terminal or the REPL (`sage.run.target`) |
| Run Selection in REPL | `sage.runSelection` | send the selection (`Shift+Enter`) |
| Run Current Cell | `sage.runCurrentCell` | send the `# %%` or `# region` cell (`Ctrl/Cmd+Enter`) |
| Start REPL | `sage.startRepl` | start or show the project's Sage REPL; restart it after changing import paths |
| Show Documentation | `sage.showDocumentation` | full documentation for the symbol in a side panel |
| Find References | `sage.findReferences` | references, including from read-only Sage library sources |
| Show Environment Details | `sage.showEnvironmentDetails` | Sage, source roots, server path, and mode |
| Show Index Status | `sage.showIndexStatus` | indexed files, symbols, cache path, pending work |
| Show Docs Status | `sage.showDocsStatus` | static docs and runtime docs worker health |
| Copy Support Bundle | `sage.copySupportBundle` | settings and status as JSON, without source contents, for bug reports |
| Run UX Self Check | `sage.runUxSelfCheck` | run every editor feature at the cursor and log the results |
| Build Persistent Database | `sage.buildDatabase` | index the Sage library now and keep the database from automatic cleanup |
| Rebuild Index | `sage.rebuildIndex` | re-index from scratch after source roots or Sage itself changed |

## Settings

| Setting | Default | Meaning |
| --- | --- | --- |
| `sage.interpreter.path` | `sage` | Sage executable for running code, the REPL, and runtime docs |
| `sage.interpreter.args` | `[]` | extra arguments for that executable |
| `sage.languageServer.rustPath` | `auto` | `sage-ls` binary; `auto` uses the packaged one |
| `sage.analysis.mode` | `default` | workspace-symbol breadth: `light` 50, `default` 200, `full` 1000 results |
| `sage.analysis.extraPaths` | `[]` | extra import paths for analysis and the REPL |
| `sage.analysis.sourceRoots` | `[]` | Sage source trees to index; empty means discover nearby and from the Sage installation |
| `sage.analysis.enableDiagnostics` | `true` | report syntax errors |
| `sage.analysis.enableRuntimeIntrospection` | `true` | ask a running Sage for docs, definitions, and signatures that static analysis misses |
| `sage.analysis.enablePyxParsing` | `true` | index `.pyx`, `.pxd`, `.pxi` |
| `sage.analysis.enablePythonFiles` | `false` | attach the server to `.py` files (Sage-heavy Python projects only) |
| `sage.indexing.exclude` | `.git`, `__pycache__`, `.venv`, `build` | globs matched inside each source root |
| `sage.docs.preferredSource` | `auto` | prefer `workspace`, `runtime`, or `reference` documentation |
| `sage.docs.showOnHover` | `true` | show the documentation preview in hovers |
| `sage.logging.level` | `info` | log detail in the Sage output channels |
| `sage.run.target` | `terminal` | run files in a terminal or `load(...)` them into the REPL |
| `sage.run.showCellCodeLens` | `true` | show Run Cell above `# %%` cells |
| `sage.run.cleanupGeneratedPython` | `false` | delete the `.sage.py` file a terminal run creates |

## Index Cache

The index is stored in VS Code's global storage (`rust-index-v2`). All projects share one database per index format, so
a Sage tree used by several projects is indexed once (about 200 MB for the full Sage library). Changed files are
re-indexed on startup and on save. Databases unused for 30 days are deleted automatically; **Build Persistent Database**
marks the current one to keep. **Rebuild Index** starts over when results look stale.

## Troubleshooting

1. Click the `Sage:` status bar item; it lists the problem and the fix (select Sage, restart, rebuild).
2. Open the **Sage** and **Sage Language Server** output channels. For more detail set `sage.logging.level` to
   `debug` and run **Restart Language Server**.
3. **Show Environment Details**, **Show Index Status**, and **Show Docs Status** show what the server uses.
4. For a bug report, attach the output of **Copy Support Bundle** (no source code is included). See
   [SUPPORT.md](../SUPPORT.md).

## Offline Reference

`npm run export:reference -- --workspace /path/to/project --source-root /path/to/sage/src` (from a source checkout)
writes `.sage-reference/index.html`, a read-only, searchable reference of a project's symbols, documentation,
definitions, and references that opens in any browser without VS Code or Sage. Local home paths are replaced by
virtual paths such as `project://src/demo.py`.

## Limits

- Static analysis covers common Sage API patterns, not every dynamic construct; runtime introspection fills some gaps.
- No notebook or Jupyter kernel integration.
