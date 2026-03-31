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

## Review Expectations

- Preserve package boundaries unless a design note explicitly changes them.
- Add tests or test placeholders with each behavior change.
- Do not rely on untracked local environment assumptions.

