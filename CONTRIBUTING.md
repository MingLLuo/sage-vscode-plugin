# Contributing

## Branch Model

- `main`: stable, releasable history only.
- `develop`: integration branch for normal development.
- `feature/*`: single scoped feature or design change.
- `release/*`: freeze and release preparation.
- `hotfix/*`: urgent production fixes.
- `experimental/*`: risky or exploratory work.

## Commit Rules

- Use Conventional Commits.
- Keep scope aligned with the subsystem you changed.
- Prefer one coherent change per commit.
- Update progress records in the same change when milestone state moves.

## Documentation Rules

- Design-level changes belong in `docs/design/`.
- Process or workflow changes belong in `docs/process/`.
- Milestone and task movement belongs in `docs/progress/`.
- New packages or important scripts must be reflected in the root docs.

## Repository Hygiene

- Respect `.editorconfig` for indentation, final newlines, and LF line endings.
- Keep `.gitattributes` aligned with source, generated text assets, packaged binaries, and VSIX archives so cross-platform
  checkouts remain reviewable.
- Do not add private workstation paths or machine-specific generated files to public docs, tests, or release scripts.

## Review Expectations

- Preserve package boundaries unless a design note explicitly changes them.
- Add tests or test placeholders with each behavior change.
- Do not rely on untracked local environment assumptions.

## Verification Gates

- Use `npm run test:ci` for the public GitHub-compatible gate. It avoids private machine paths, desktop VS Code, and
  mandatory Sage runtime availability while still running Rust tests, clippy, TypeScript lint/tests, debug-web smoke,
  legacy Python tests, VSIX content/package checks, cache maintenance, and portable performance smoke.
- Use `npm run test:repo-hygiene` after changing GitHub issue templates, PR templates, `SECURITY.md`, `SUPPORT.md`, or
  CI/release gate definitions, `.gitattributes`, or `.editorconfig`.
- Use `npm run test:release` for local release candidates. It additionally exercises persistent LSP latency and
  real-file Sage-heavy smoke against the configured local Sage checkout and local research fixtures.
- Use `npm run test:full` only when the machine may launch the VS Code Extension Host.
