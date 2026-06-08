# Plugin Completeness

This is the single English checklist for manifest-facing Sage VS Code features. It stays concise so command and setting
coverage remains testable without maintaining parallel manuals.

## Required Surfaces

- Languages: `sagemath` and `sagemath-cython` must have generated grammar, snippets, and language configuration assets.
- Activation: Sage files, Sage Cython files, Sage-named Python files, startup command registration, and Sage commands must
  be covered by manifest activation events.
- Commands: every contributed `sage.*` command must appear in this document, `README.md`, or `docs/install-and-configure.md`.
- Settings: every contributed `sage.*` setting must appear in this document, `README.md`, or `docs/install-and-configure.md`.
- Safety: runtime features require trusted local workspaces; syntax browsing remains available in restricted contexts.
- Packaging: VSIX contents must include the compiled extension entrypoint, generated syntax assets, icon, walkthrough
  resources, package README, changelog, license, and the staged Rust language-server binary.

## Commands

- `sage.openGettingStarted`: open the first-run walkthrough.
- `sage.selectInterpreter`: select a Sage runtime profile or custom runtime path.
- `sage.configureWorkspace`: write a conservative workspace profile.
- `sage.restartLanguageServer`: restart `sage-ls`.
- `sage.runCurrentFile`: run the active Sage file.
- `sage.runSelection`: run the current selection.
- `sage.runCurrentCell`: run the current `# %%` or `# region` cell.
- `sage.startRepl`: start or reveal the managed Sage REPL.
- `sage.showDocumentation`: open documentation for the active symbol.
- `sage.findReferences`: show Sage-aware references for the active symbol.
- `sage.showEnvironmentDetails`: show interpreter, source-root, runtime, and language-server details.
- `sage.showIndexStatus`: show Rust index file, symbol, docs, cache, and pending-job status.
- `sage.showDocsStatus`: show static and runtime documentation worker status.
- `sage.copySupportBundle`: copy a support JSON bundle without source contents or environment variables.
- `sage.runUxSelfCheck`: run hover, docs, navigation, references, rename, completion, and diagnostics checks.
- `sage.rebuildIndex`: rebuild the Rust index for the current roots.

## Settings

- `sage.interpreter.path`
- `sage.interpreter.args`
- `sage.languageServer.pythonPath`
- `sage.languageServer.rustPath`
- `sage.languageServer.pythonArgs`
- `sage.analysis.mode`
- `sage.analysis.extraPaths`
- `sage.analysis.sourceRoots`
- `sage.analysis.enableDiagnostics`
- `sage.analysis.enableRuntimeIntrospection`
- `sage.analysis.enablePyxParsing`
- `sage.analysis.enablePythonFiles`
- `sage.indexing.exclude`
- `sage.docs.preferredSource`
- `sage.docs.showOnHover`
- `sage.logging.level`
- `sage.run.target`
- `sage.run.showCellCodeLens`
- `sage.run.cleanupGeneratedPython`
- `sage.experimental.notebookSupport`

## Verification

- `npm run test --workspace sage-vscode-extension`
- `npm run test:debug-web`
- `npm run test:product-readiness`
- `npm run test:lsp-latency`
- `npm run test:real-file-smoke`
- `npm run test:extension-host`

## Modern Plugin Alignment

Use Pyright and rust-analyzer as the comparison bar for user experience:

- Interaction: commands should be discoverable, iconed, context-aware, and backed by a first-run walkthrough.
- Latency: warm hover, definition, completion, references, and documentation queries should stay inside the release
  budgets tracked by the latency and real-file smoke tests.
- Status: startup, indexing, cache hydration, source roots, docs runtime state, and degraded modes should be visible
  through the status bar, index/docs status commands, and support bundle.
- Workspace: ordinary Python projects should not be forced into Sage analysis; Sage-heavy Python and Cython projects
  should opt in predictably.
- Diagnostics: syntax diagnostics, UX self-check output, support bundles, and performance issue templates should make
  failures reproducible without exposing private source contents.
