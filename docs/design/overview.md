# Design Overview

## Product Direction

The plugin aims to make SageMath feel native in VS Code while staying compatible with Python and Jupyter workflows.
The design favors explicit package boundaries and incremental capability growth over a monolithic extension.

## Primary User Capabilities

- Edit `.sage` files with dedicated language registration and editor behavior.
- Start from minimal LSP-backed hover, completion, diagnostics, and navigation.
- Understand which Sage interpreter or analysis root is active.
- Grow toward notebook and runtime-aware workflows without making them bootstrap blockers.

## Non-Goals for the Bootstrap Phase

- Full notebook integration
- Debug adapter support
- Exhaustive static understanding of all dynamic Sage constructs

