# Language Server Boundary

## Responsibilities

- Accept LSP requests over stdio through `pygls`.
- Own analysis-side configuration and environment normalization.
- Parse Python, loose `.sage`, and lightweight `.pyx` source into a shared symbol model.
- Build a static workspace index and resolve symbols through imports, star imports, and lazy imports.
- Provide hover, completion, definition, document symbols, and custom documentation lookup.
- Remain the place where `.sage` preprocessing and source mapping logic continue to evolve.

## Explicit Non-Responsibilities

- Rendering editor UI
- Owning syntax grammar assets
- Hardcoding local Sage source checkout paths

## Current Capability

The current server revision already hosts:

- a `pygls` transport and request surface
- a reduced Sage fixture-backed static index
- `.sage` loose parsing and first-pass source mapping
- documentation extraction and custom docs payloads
