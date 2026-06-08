# CI and Release Gates

This repository has three intentionally different verification levels. Keep them separate so public GitHub checks stay
reproducible while local release candidates still exercise real Sage-heavy workloads.

## `npm run test:ci`

`test:ci` is the GitHub Actions gate for the maintained macOS release path. It must not require private paths under
`/Users/...`, a desktop VS Code app, or a working Sage runtime. It runs:

- Rust tests and `cargo clippy --all-targets --all-features -- -D warnings`.
- Syntax and extension lint.
- TypeScript, debug workbench, and legacy Python regression tests through `npm run test`.
- Generated asset drift smoke for extension-local syntax assets, stale generated syntax files, and the deterministic
  package icon.
- macOS Rust binary staging plus VSIX content/package smokes.
- Cache-maintenance smoke.
- Repository hygiene smoke for GitHub issue templates, PR template, `SECURITY.md`, `SUPPORT.md`, `.gitattributes`,
  `.editorconfig`, and gate boundaries.
- Performance smoke with `--skip-workbench`; this reports a structured skip when no Sage source checkout is present.
- `git diff --check` for whitespace errors.

Do not add `test:lsp-latency`, `test:real-file-smoke`, `test:native-smoke`, or `test:extension-host` to `test:ci`.
Those gates intentionally depend on local Sage/source/VS Code state.

## `npm run test:release`

`test:release` is the local non-desktop release gate. It includes `test:ci`-level coverage plus:

- VS Code CLI install smoke when the `code` CLI is available.
- Release index performance against `SAGE_SOURCE_ROOT` or the nearby `../sage/src` checkout.
- Persistent JSON-RPC LSP latency checks.
- Real Sage-heavy file smoke through the checked-in public synthetic fixture, or through `SAGE_REAL_FILE_SMOKE_PATH` /
  `SAGE_REAL_FILE_SMOKE_PATHS` when maintainers want to exercise private local projects.

Use this before claiming a release candidate is ready. If a contributor does not have the local Sage checkout, Sage-root
dependent smokes report explicit skipped status. If maintainers provide `SAGE_REAL_FILE_SMOKE_PATH` or
`SAGE_REAL_FILE_SMOKE_PATHS`, every configured file path is treated as required and a missing file fails the smoke instead
of being silently ignored.

## `npm run test:full`

`test:full` adds the desktop Extension Host smoke. It can open VS Code, so it stays outside CI and should be run only on
a machine where GUI automation is acceptable.

## Workflow Rules

- GitHub Actions runs on `macos-latest`, installs Node and Python dependencies, then executes `npm run test:ci`.
- Generated syntax assets must pass lint before build writes anything.
- `npm run test:generated-assets` must pass after changing syntax resources, generated extension-local assets,
  `scripts/generate-extension-icon.mjs`, or package branding files. `npm run package:vsix` runs the same gate before
  packaging so stale generated syntax files do not get bundled accidentally.
- `npm run package:vsix` stages the current macOS release `sage-ls` binary before package-content checks, so direct local
  packaging does not reuse a stale server binary. Non-macOS script paths are retained only for defensive tests and are not
  a release promise.
- VSIX packaging is deterministic by default. The packager uses a fixed archive timestamp unless `SOURCE_DATE_EPOCH` is
  set, and `npm run test:vsix-package` verifies repeated packaging produces the same archive hash.
- `npm run test:repo-hygiene` must pass after changing issue templates, PR templates, `SECURITY.md`, `SUPPORT.md`,
  `CONTRIBUTING.md`, `.gitattributes`, `.editorconfig`, or CI/release scripts.
- New public gates should be added here, in `CONTRIBUTING.md`, and in the package metadata tests in the same change.
