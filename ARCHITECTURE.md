# Architecture

## Layered Structure

- The VS Code client owns activation, command registration, configuration plumbing, and user-facing surfaces.
- The Python language server owns analysis, source mapping, documentation lookup, and diagnostics.
- Syntax assets stay isolated so grammar and editor behaviors can evolve without coupling to runtime logic.

## Initial Decisions

- Independent monorepo with package boundaries from day one.
- TypeScript client using the standard VS Code LSP client model.
- Python server using `pygls` as the LSP transport and lifecycle framework.
- Stdio transport for the first implementation.
- Process-first repository setup so every design and implementation step is traceable.

## Early Constraints

- The repository must not depend on a full local Sage source checkout at runtime.
- Notebook and remote execution are planned but are not bootstrap blockers.
- `.sage` source mapping and preparser integration remain explicit design topics, not implicit assumptions.

