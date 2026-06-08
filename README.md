# Sage VS Code Plugin

`Sage VS Code Plugin` is an independent monorepo for a SageMath-focused development experience in Visual Studio Code.

It now includes a usable local development baseline: a VS Code client, a Rust language server, a persistent
Sage/Python/Cython source index, Sage-aware `.sage` preprocessing, runtime-backed documentation fallback, and curated
manual plus automated smoke tests.

## Quick Start

Install and use the current local macOS build:

```bash
npm install
npm run package:vsix
code --install-extension dist/sage-vscode-extension-0.1.0.vsix --force
```

Then open a Sage workspace in VS Code and run:

1. `Sage: Open Getting Started`
2. `Sage: Select Interpreter`
3. `Sage: Configure Workspace`
4. `Sage: Show Index Status`

For Sage-heavy Python projects, enable Sage analysis for `.py` files through `Sage: Configure Workspace` or this
workspace setting:

```json
{
  "sage.languageServer.rustPath": "auto",
  "sage.analysis.enablePythonFiles": true,
  "sage.analysis.sourceRoots": ["/path/to/sage/src"],
  "sage.interpreter.path": "/path/to/sage"
}
```

See [Quick Start](./docs/quick-start.md) for the short user path.
Use [Install and Configure](./docs/install-and-configure.md) for the full reference.

## Goals

- Deliver a maintainable Sage editor experience for `.sage` files.
- Keep the client, Rust language server, legacy Python migration baseline, and syntax assets cleanly separated.
- Record design decisions, progress, and commit-level development history in-repo from the start.

## Workspace Layout

- `packages/extension-core`: VS Code extension and LSP client bootstrap.
- `crates/sage-ls`: primary Rust language-server process.
- `crates/sage-index`: Rust source index, SQLite cache, query, diagnostics, and semantic-token engine.
- `packages/sage-lsp`: legacy Python `pygls` server retained as a migration and regression baseline.
- `packages/syntax-pack`: grammar, snippets, and language configuration.
- `docs/`: concise design notes, release gates, and current progress.
- `examples/manual-smoke-workspace`: ready-to-open sample workspace for manual and automated smoke checks.

## Current Status

- Repository bootstrap, Rust build, TypeScript build, syntax sync, and Python legacy tests are locally validated.
- The extension launches `sage-ls` from `SAGE_LS_PATH`, `sage.languageServer.rustPath`, local `target/*`, or `PATH`.
- The extension provides a Getting Started walkthrough, environment-first interpreter selection, status presentation, run
  commands, managed REPL, documentation panel, index/docs status commands, support bundle capture, rebuild, and an editor
  UX self-check command.
- The Rust language server covers hover, documentation, definition, completion, signature help, inlay hints, diagnostics,
  semantic tokens, document symbols, workspace symbols, references, rename, save/watch refresh, and native Cython
  navigation.
- Source indexing handles `.sage`, `.py`, `.pyx`, `.pxd`, and `.pxi`, persists SQLite cache data, and can supplement
  workspace roots with nearby or runtime-discovered Sage source roots.
- Runtime documentation fallback can query the selected Sage executable when static indexed docs or locations are weak.
- A Browser Use debug workbench and a real VS Code extension-host smoke suite validate the user-facing edit loop.
- VSIX packaging includes a generated extension icon, gallery banner metadata, bundled walkthrough resources, and
  package-content smoke tests.
- The extension is marked as a preview workspace extension because the Rust LSP and optional docs worker need access to
  workspace-local files and processes.

## Reference Inputs

- `deep-research-report.md` in the sibling `sage-src` workspace defines the target product direction.
- A sibling Sage checkout such as `../sage` can be used as a local source calibration checkout; alternatively set
  `SAGE_SOURCE_ROOT` explicitly for performance and release smokes.
- Nearby repositories may be consulted for patterns, but this repository remains independently owned.

## Development Workflow

- Use Conventional Commits with narrow scopes.
- Keep each small action or feature in its own commit when practical.
- Update the short progress tracker when current release state changes.
- Add or update design notes only when an architectural decision needs to survive code review.

## Documentation Index

- [Developer Guide](./docs/developer-guide.md)
- [Changelog](./CHANGELOG.md)
- [Quick Start](./docs/quick-start.md)
- [Install and Configure](./docs/install-and-configure.md)
- [Plugin Completeness and Verification](./docs/plugin-completeness.md)
- [Rust LSP V2](./docs/design/rust-lsp-v2.md)
- [Native Source Support](./docs/design/native-source-support.md)
- [Design Overview](./docs/design/overview.md)
- [Development Progress](./docs/progress/development-progress.md)
- [Manual Smoke Workspace](./examples/manual-smoke-workspace/README.md)

## Quick Verification

```bash
npm install
npm run build
npm run test:ci
npm run test:repo-hygiene
npm run test:product-readiness
npm run package:rust-binary
npm run package:vsix
npm run test:vsix-install
npm run test:release
npm run test:native-smoke
npm run cache:status
npm run clean:dry-run
```

`npm run test:ci` is the public GitHub-compatible gate. It avoids private local files and desktop VS Code while covering
Rust tests, clippy, lint, extension/debug/Python tests, generated asset drift checks, VSIX content/package smoke,
cache-maintenance smoke, portable performance smoke, and whitespace checks.

`npm run test:release` is the local non-desktop release gate. It adds VS Code CLI install smoke, release index
performance against a local Sage checkout, persistent LSP latency, and real-file Sage-heavy smoke.

`npm run test:repo-hygiene` verifies public GitHub maintenance files such as issue templates, `SECURITY.md`,
`SUPPORT.md`, and CI/release-gate boundaries.

`npm run test:product-readiness` verifies the high-level editor experience matrix: interaction, language coverage,
latency gates, debuggability, Mac packaging, future Sage-update resilience, and maintainability.

`npm run package:vsix` rebuilds and stages the current macOS release `sage-ls` binary, verifies generated assets and
package contents, then writes `dist/sage-vscode-extension-0.1.0.vsix`.

`npm run test:extension-host` should be used when the local machine can launch the desktop VS Code app.
`npm run debug:web` starts the browser workbench surface used by MCP/Browser Use inspection.

The VSIX package root includes its own `README.md`, `CHANGELOG.md`, and `LICENSE`.
`npm run test:vsix-contents` verifies these release artifacts together with runtime resources.
`npm run test:vsix-package` verifies the generated VSIX archive structure, production dependency closure,
content-type coverage, and entry CRCs.
`npm run test:vsix-install` uses the VS Code CLI, when available, to install the generated VSIX into temporary user-data
and extension directories without opening the desktop app.

`npm run cache:status` inventories the root-aware Rust SQLite caches.
`npm run cache:prune:dry-run` previews old-cache cleanup.
Actual deletion requires:

```bash
node scripts/cache-maintenance.mjs --prune --max-age-days 30 --yes
```

`npm run clean:dry-run` previews macOS local build and test artifacts that can be removed after packaging or validation.
Use `npm run clean -- --yes` to remove those artifacts.
Add `--deps` only when you also want to remove `node_modules`, local virtualenvs, and `package-lock.json`.

For manual GUI smoke testing, run `npm run dev:vscode:smoke`, press `F5`, and verify the new
`[Extension Development Host]` window shows `.sage` files as `SageMath` with a left status-bar item beginning `Sage:`.
If `.sage` opens as `Plain Text`, close that normal VS Code window and relaunch from the repository with `F5`.

## Known Deferred Work

1. Sign and publish native Rust binaries for marketplace-style distribution.
2. Add notebook and kernel surfaces.
3. Continue reducing the legacy Python LSP once Rust parity is explicitly accepted.
