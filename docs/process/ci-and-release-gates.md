# CI and Release Gates

This repository has three intentionally different verification levels. Keep them separate so public GitHub checks stay
portable while local release candidates still exercise real Sage-heavy workloads.

## `npm run test:ci`

`test:ci` is the GitHub Actions gate for the maintained macOS release path. It must not require private paths under
`/Users/...`, a desktop VS Code app, or a working Sage runtime. It runs:

- Locked Rust tests and `cargo clippy --locked --all-targets --all-features -- -D warnings`.
- Syntax and extension lint.
- TypeScript, debug workbench, and legacy Python regression tests through `npm run test`.
- The full 23-scenario real-Sage debug-workbench matrix through `npm run test:debug-web:sage`.
- Repository-local LSP navigation contract smoke covering one-target high-confidence jumps, ordered ambiguous
  `LocationLink` candidates, exact `.pxd` declaration/`.pyx` implementation role selection, explanatory hover content,
  and safety gates for weak references, rename, and call hierarchy.
- LSP shutdown smoke proving rebuild and cache-reconcile work cannot block restart or process exit.
- Generated asset drift smoke for extension-local syntax assets, stale generated syntax files, and the deterministic
  package icon.
- macOS Rust binary staging plus VSIX content/package smokes.
- Cache-maintenance smoke.
- Repository hygiene smoke for GitHub issue templates, PR template, `SECURITY.md`, `SUPPORT.md`, `.gitattributes`,
  `.editorconfig`, and gate boundaries.
- Product readiness smoke for interaction, language coverage, visual polish, latency gates, debuggability, Mac packaging,
  future Sage update resilience, and maintainability.
- Offline reference export smoke for the static `.sage-reference/` viewer, search index, source shards, and private-path
  stripping.
- Performance smoke with `--skip-workbench`; GitHub supplies the sparse Sage source root, while direct script runs still
  report a structured skip when no source checkout is present.
- `git diff --check` for whitespace errors.

Do not add `test:lsp-latency`, `test:real-file-smoke`, `test:native-smoke`, or `test:extension-host` to `test:ci`.
Those gates intentionally depend on local Sage/source/VS Code state.

Running `test:ci` directly requires a nearby Sage checkout or `SAGE_SOURCE_ROOT=/path/to/sage/src`; the public workflow
provides this with a sparse checkout of the latest Sage default branch. A Sage executable is not required.

## `npm run test:release`

`test:release` is the local non-desktop release gate. It includes `test:ci`-level coverage plus:

- VS Code CLI install smoke when the `code` CLI is available.
- Release index performance against `SAGE_SOURCE_ROOT` or the nearby `../sage/src` checkout.
- Persistent JSON-RPC LSP latency checks.
- Real Sage-heavy file smoke through the checked-in public synthetic fixture, or through `SAGE_REAL_FILE_SMOKE_PATH` /
  `SAGE_REAL_FILE_SMOKE_PATHS` when maintainers want to exercise private local projects.

Use this before claiming a release candidate is ready. A local Sage source checkout is required for the full UX matrix;
set `SAGE_SOURCE_ROOT` when it is not discoverable nearby. Individual lower-level smokes still report explicit skipped
status when their optional inputs are absent. If maintainers provide `SAGE_REAL_FILE_SMOKE_PATH` or
`SAGE_REAL_FILE_SMOKE_PATHS`, every configured file path is treated as required and a missing file fails the smoke instead
of being silently ignored.

## `npm run test:full`

`test:full` adds the desktop Extension Host smoke. It can open VS Code, so it stays outside CI and should be run only on
a machine where GUI automation is acceptable.

## Workflow Rules

- GitHub Actions runs on `macos-latest`, uses the Node 22 baseline declared by `.node-version` and the exact Rust version
  in `rust-toolchain.toml`, installs Python dependencies, restores Cargo build/cache state, runs `cargo fetch --locked`,
  sparsely checks out the latest default-branch `sagemath/sage` sources for the real navigation UX matrix, then executes
  `npm run test:ci`. A lightweight Linux matrix covers Node.js 22/npm 11 and Node.js 26/npm 12, while a second macOS
  job packages the VSIX end to end with Node.js 26/npm 12.
- Generated syntax assets must pass lint before build writes anything.
- `npm run test:generated-assets` must pass after changing syntax resources, generated extension-local assets,
  `scripts/generate-extension-icon.mjs`, or package branding files. `npm run package:vsix` runs the same gate before
  packaging so stale generated syntax files do not get bundled accidentally.
- `npm run package:vsix` stages the current macOS release `sage-ls` binary before package-content checks, so direct local
  packaging does not reuse a stale server binary. Non-macOS script paths are retained only for defensive tests and are not
  a release promise.
- VSIX package and install smokes also restage `sage-ls` unconditionally, so an existing binary cannot hide newer Rust
  source changes.
- Release Rust staging uses `cargo build --locked` and remaps repository, Cargo-home, and user-home source prefixes before
  copying the binary. The VSIX smoke rejects binaries that retain build-machine home or repository paths.
- `npm run doctor:mac` is a local diagnostic check for the current Mac package, staged Rust server, VS Code CLI, Sage
  runtime, and Sage source root. It is intentionally not part of `test:ci` because a clean CI checkout may not have a
  user-installed Sage runtime or VS Code CLI.
- VSIX packaging requires Node.js 22.9 or newer and npm 11 or newer; `.node-version` records the Node 22 baseline instead
  of an exact-version lock. The toolchain gate also reads the installed npm package's Node.js engine range and rejects
  incompatible Node/npm pairings. Packaging uses a fixed archive timestamp unless `SOURCE_DATE_EPOCH` is set, normalizes
  regular-file modes to `0644` and `sage-ls` to `0755`, and enforces a 6 MiB archive budget. `npm run
  test:vsix-package` rebuilds under both normal and restrictive umasks and verifies the archive hash remains identical.
- `npm run test:repo-hygiene` must pass after changing issue templates, PR templates, `SECURITY.md`, `SUPPORT.md`,
  `CONTRIBUTING.md`, `.gitattributes`, `.editorconfig`, or CI/release scripts.
- New public gates should be added here, in `CONTRIBUTING.md`, and in the package metadata tests in the same change.
