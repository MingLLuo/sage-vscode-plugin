# Contributing

## Branch Model

- `main`: stable, releasable history only.
- `develop`: integration branch for normal development.
- `feature/*`: single scoped feature or design change.
- `release/*`: freeze and release preparation.
- `hotfix/*`: urgent production fixes.
- `experimental/*`: risky or exploratory work.

## Commit Rules

- Use Conventional Commits, scoped to the subsystem you changed.
- Prefer one coherent change per commit, with its tests.

## Documentation Rules

- User-visible behavior: [User Guide](docs/install-and-configure.md) and the changelog.
- Build, test, debug, and release steps: [Developer Guide](docs/developer-guide.md).
- Design decisions that should outlive a code review: `ARCHITECTURE.md` or `docs/design/`.
- Delete notes that no longer describe the current code instead of adding new ones next to them.

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

See [Tests](docs/developer-guide.md#tests). In short: `cargo test` and `npm run test` for every change, `npm run test:ci`
before pushing, `npm run test:release` and `npm run test:extension-host` before a release, and
`npm run test:repo-hygiene` after editing `SECURITY.md`, `SUPPORT.md`, `.gitattributes`, `.editorconfig`, templates, or
workflows. Node.js 22.9+ and npm 11+; Rust is pinned by `rust-toolchain.toml`.
