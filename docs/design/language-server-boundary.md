# Language Server Boundary

## Responsibilities

- Accept LSP requests over stdio through `pygls`.
- Own analysis-side configuration and environment normalization.
- Provide minimal health, hover, and future source-intelligence entrypoints.
- Remain the place where `.sage` preprocessing and source mapping logic will eventually live.

## Explicit Non-Responsibilities

- Rendering editor UI
- Owning syntax grammar assets
- Hardcoding local Sage source checkout paths

## Bootstrap Capability

The first server revision only needs to prove that the repository can host a valid Python package, start a `pygls`
server process, and expose typed extension points for later Sage-specific analysis.

