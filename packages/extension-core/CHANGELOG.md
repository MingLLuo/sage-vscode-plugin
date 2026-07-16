# Changelog

## Unreleased

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
  exact Node/npm checks, and VS Code 1.97 API type compatibility.

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
