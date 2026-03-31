# Commit Policy

## Objective

Keep repository history small, scoped, and auditable so design and implementation progress can be reconstructed from Git alone.

## Rules

- Use Conventional Commits.
- Always include a subsystem scope such as `repo`, `docs`, `process`, `extension`, `lsp`, or `syntax`.
- Prefer one coherent intent per commit.
- If a commit changes milestone state, update the progress tracker in the same commit.
- If a commit changes architecture or package boundaries, add or update a design note in the same commit.

## Expected Commit Shapes

- `chore(repo): ...` for repository scaffolding and workspace setup.
- `docs(process): ...` for workflow rules, templates, or operating conventions.
- `docs(design): ...` for architectural decisions and design baselines.
- `feat(extension): ...` for VS Code client features.
- `feat(lsp): ...` for language server behavior.
- `feat(syntax): ...` for grammar and editor assets.
- `test(...)` for test-only additions or repairs.

## Traceability Requirement

Each completed task should be traceable through:

1. a task entry in `docs/progress/task-board.md`
2. a progress or milestone update in `docs/progress/development-progress.md`
3. one or more Git commits with matching subsystem scope

