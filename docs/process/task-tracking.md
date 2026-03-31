# Task Tracking

## Task States

- `proposed`
- `planned`
- `in_progress`
- `blocked`
- `done`

## Required Fields

Each task entry must capture:

- task ID
- title
- milestone
- subsystem
- status
- owner
- exit criteria
- related design notes
- related commits

## Operating Rules

- New work starts as `planned` unless it is only an idea, in which case it stays `proposed`.
- Only one task should be actively `in_progress` per narrow implementation stream.
- Move work to `blocked` instead of silently stalling it.
- Mark work `done` only when the repository state and progress tracker agree.

## Recommended ID Format

- `BOOT-###` for bootstrap and governance work
- `EXT-###` for extension work
- `LSP-###` for server work
- `SYN-###` for syntax work
- `OPS-###` for tooling, CI, and release operations

