# Task Board

| Task ID | Title | Milestone | Subsystem | Status | Owner | Exit Criteria | Design Notes | Related Commits |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| BOOT-001 | Bootstrap independent repository | Repository bootstrap | repo | done | MingLLuo | Root docs and workspace metadata committed on `main`. | N/A | `833666b` |
| BOOT-002 | Establish process and progress templates | Process baseline | docs/process | done | MingLLuo | Core templates and trackers exist and are linked from root docs. | N/A | `5bfa5c2` |
| BOOT-003 | Add baseline design notes | Repository bootstrap | docs/design | done | MingLLuo | Initial design notes define package boundaries and bootstrap constraints. | `docs/design/overview.md` | `c59d923` |
| EXT-001 | Scaffold minimal VS Code client package | Extension scaffold | extension | done | MingLLuo | Extension package can build structurally and defines activation entrypoints. | `docs/design/overview.md` | `69289d4` |
| LSP-001 | Scaffold minimal pygls server package | LSP scaffold | lsp | done | MingLLuo | Python package exposes a runnable `pygls` server entrypoint. | `docs/design/language-server-boundary.md` | `cde47b1` |
| SYN-001 | Scaffold syntax assets package | Syntax scaffold | syntax | done | MingLLuo | Package includes language configuration, snippets, grammar placeholders, and extension sync output. | `docs/design/workspace-layout.md` | pending |
| OPS-001 | Add bootstrap CI placeholder | Repository bootstrap | ops | done | MingLLuo | Repository has a non-destructive CI placeholder aligned with root scripts. | `docs/developer-guide.md` | pending |
