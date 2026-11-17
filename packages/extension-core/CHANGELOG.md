# Changelog

## Unreleased

### Interaction

- Hover and completion documentation render Sage reStructuredText as Markdown: ``literals`` and `:class:` references
  become code, `INPUT:`-style headings are bold, `EXAMPLES::` doctests and `.. MATH::` become code blocks, and
  `.. NOTE::` gets a label. Shortened previews never cut a code block in half.
- The Getting Started walkthrough now covers installing or selecting SageMath, running a first file, the editing and
  REPL shortcuts, project configuration, and troubleshooting, in English and Simplified Chinese; it no longer asks
  new users to run the developer self check.
- Simplified Chinese: command names, settings, the walkthrough, status bar labels, and run/REPL/navigation/startup
  messages follow the VS Code display language (`package.nls.zh-cn.json`, `l10n/bundle.l10n.zh-cn.json`).
- The status bar checks the configured Sage once (`sage --version`) and shows "Sage not found" instead of "ready"
  when it cannot be started; run and REPL commands then explain why and offer Install SageMath or Select Sage
  Interpreter. Editing features keep working without Sage. The tooltip shows the detected SageMath version.
- `Shift+Enter` sends the selection and `Ctrl+Enter` / `Cmd+Enter` runs the current cell in Sage editors.
- Multi-line selections and cells reach the REPL as one bracketed paste, so blocks with blank lines run correctly; the
  import-path setup is sent once per REPL session.
- "Run UX Self Check" and "Build Persistent Database" left the editor menu and command palette (both remain in the status
  menu); the duplicate Find References menu entry is gone (use Shift+F12). Commands are renamed "Run Selection in REPL"
  and "Select Sage Interpreter".
- The unused `sage.experimental.notebookSupport` setting is removed, and the extension no longer activates for any
  Python file whose name merely contains "sage".

### Removed

- Removed the retired Python `pygls` language server (`packages/sage-lsp`), its test and CI steps, and the
  `sage.languageServer.pythonPath` / `sage.languageServer.pythonArgs` settings. The interpreter picker now lists Sage
  runtimes only; `Configure Workspace` drops the stale keys from existing settings files.

### Fixed

- Incoming call hierarchy now reports callers in the definition's own file while the background index is still being
  built, instead of returning an empty list until indexing finishes.
- Run, REPL, and cell commands no longer wait for an unanswered error notification before returning.

### Changed

- Definition navigation now opens a single target only for unique, high-confidence type/import/scope evidence; ambiguous
  members return ordered definition candidates for VS Code's multi-target preview, while references and rename stay disabled.
- Made definition, references, rename, and call hierarchy navigation scope-aware for parameters, import aliases,
  receiver members, and same-name imports, including read-only indexed Sage source roots.
- Added hover, signature help, document links, and call hierarchy support for `sage-source` documents while preserving
  the virtual URI in editor results.
- Improved run, REPL, interpreter, status, configuration, and language-server lifecycle feedback, including dirty-file
  save handling, bounded operations, actionable recovery choices, and active-workspace behavior in multi-root windows.
- Split language-server initialization and extension-side workspace, external-source, run preparation, and lifecycle
  concerns into focused modules with regression coverage.
- Hardened reproducible VSIX packaging with explicit content and dependency allowlists, executable-mode preservation,
  Node/npm range and runtime-pair compatibility checks, and VS Code 1.97 API type compatibility.
- Made the public CI self-contained by sparsely checking out the latest Sage source for the real navigation UX matrix,
  while keeping a fixture-only debug smoke for ordinary Node test runs.
- Added Sage 10.10.beta8 PEP 695 declaration support, including precise `Hom[...]` navigation without false whole-file
  syntax diagnostics, and invalidated older indexes that omitted these declarations.
- Fixed explicit imports being shadowed by earlier Sage `import *` bindings, preserving the exact imported definition
  instead of returning unrelated ambiguous candidates.
- Hardened runtime diagnostics so worker cache directories exist before Sage starts and the macOS doctor requires an
  importable `sage.all` runtime instead of accepting a version-only source checkout.
- Split the static Sage export, owner-module, method, and alias catalogs into focused modules with cross-table invariant
  tests for easier review as Sage evolves.
- Split language-client lifecycle and runtime source-root discovery out of the activation entrypoint, with race-focused
  tests for queued restarts, shutdown during startup, stale discovery results, and discovery retry triggers. Runtime
  probes are keyed and serialized across folder-scoped settings so editor switches neither lose roots nor repeat the
  same Sage subprocess probe.
- Updated the extension-host harness to recognize current macOS VS Code `Code` binaries while retaining compatibility
  with older `Electron` and Insiders app layouts.

## 0.1.0 - 2026-05-24

Initial preview release surface for the Sage VS Code Plugin.

### Added

- Rust-backed Sage language server for hover, documentation, definition, completion, signature help, diagnostics,
  semantic tokens, document/workspace symbols, references, rename, and index status.
- Persistent SQLite-backed Sage/Python/Cython source index with warm hydrate, root-aware cache namespaces, cache
  maintenance tooling, and real Sage checkout performance gates.
- Sage-heavy Python support through opt-in `sage.analysis.enablePythonFiles`, including `sage.all` constructor and common
  matrix/polynomial method navigation.
- VS Code client commands for runtime selection, workspace configuration, language-server restart, run/repl workflows,
  documentation, environment details, index/docs status, index rebuild, Getting Started, and UX self-check.
- Inline `Run Cell` / `Run Region` CodeLens actions for Sage cells and regions, with a setting to hide the inline
  execution affordances when preferred.
- Human-readable docs fallback status that distinguishes static indexed docs, disabled/unavailable runtime lookup,
  idle runtime fallback, and degraded runtime worker states.
- Generated Sage syntax assets for `.sage`, `.pyx`, `.pxd`, and `.pxi` files.
- Browser debug workbench, real-file Sage-heavy smoke checks, non-desktop release gate, extension-host smoke, packaged
  Rust binary staging, local VSIX packaging, generated icon, gallery metadata, walkthrough resources, and package legal
  files.
- VSIX archive validation for manifest details metadata, content-type coverage, entry CRC integrity, and isolated
  `code` CLI installation into temporary user-data/extensions directories.
- Marketplace-facing README guidance for first run, settings, troubleshooting, preview limitations, support, and
  security reporting.
- Public GitHub issue forms, PR checklist, security/support docs, and repository hygiene smoke coverage for future
  maintainers.

### Notes

- The extension is marked as preview.
- The Rust language server is the primary runtime path; the legacy Python LSP remains in the repository as a migration and
  regression baseline.
- Marketplace support links, signing, and non-macOS binary release automation remain deferred until the publishing channel
  is finalized.
