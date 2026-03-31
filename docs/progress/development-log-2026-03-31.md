# Development Log: 2026-03-31

## Entry 1

- Date: 2026-03-31
- Task ID: BOOT-001
- Scope: repo
- Related milestone: Repository bootstrap
- Commit: `833666b`

### Goal

Create the independent repository, define monorepo package boundaries, and land root governance documents before feature scaffolding begins.

### Decisions

- Decision: start from an independent repository instead of embedding work inside a local Sage checkout.
- Reason: repository ownership and commit granularity need to stay under plugin-specific control.

### Verification

- Checks run: `git init`, branch renamed to `main`
- Result: repository bootstrap completed

### Follow-ups

- Next task: BOOT-002
- Risks or blockers: none

## Entry 2

- Date: 2026-03-31
- Task ID: BOOT-002
- Scope: docs/process
- Related milestone: Process baseline
- Commit: `5bfa5c2`

### Goal

Add commit rules, task states, milestone review structure, and a repeatable development log template before more code lands.

### Decisions

- Decision: require progress updates and design-note updates when milestone or architecture state changes.
- Reason: the user asked for commit-level traceability and detailed development records.

### Verification

- Checks run: manual doc review
- Result: process templates committed

### Follow-ups

- Next task: BOOT-003
- Risks or blockers: none

## Entry 3

- Date: 2026-03-31
- Task ID: BOOT-003
- Scope: docs/design
- Related milestone: Design baseline
- Commit: `c59d923`

### Goal

Pin the initial design envelope for package layout, language-server responsibilities, and `.sage` source mapping risk.

### Decisions

- Decision: document source mapping as a bootstrap risk instead of pretending the first scaffold solves it.
- Reason: `.sage` preprocessing is a core technical constraint and should remain explicit.

### Verification

- Checks run: manual doc review
- Result: baseline design notes committed

### Follow-ups

- Next task: EXT-001
- Risks or blockers: full preparser mapping still deferred

## Entry 4

- Date: 2026-03-31
- Task ID: EXT-001
- Scope: extension
- Related milestone: Extension scaffold
- Commit: `69289d4`

### Goal

Add a minimal VS Code extension package with configuration mapping, command registration, and stdio language-client startup.

### Decisions

- Decision: keep the first client thin and configuration-driven.
- Reason: the architecture intends to move analysis behavior into the Python server rather than the extension host.

### Verification

- Checks run: structural review of package layout
- Result: extension scaffold committed

### Follow-ups

- Next task: LSP-001
- Risks or blockers: Node dependencies not yet installed locally in this repository

## Entry 5

- Date: 2026-03-31
- Task ID: LSP-001
- Scope: lsp
- Related milestone: LSP scaffold
- Commit: `cde47b1`

### Goal

Create a minimal `pygls` package with a runnable entrypoint, typed server settings, and a placeholder hover path.

### Decisions

- Decision: make the first tests dependency-light by centering them on settings normalization.
- Reason: this allows early validation before the full Python dependency set is installed.

### Verification

- Checks run: later validated through `PYTHONPATH=packages/sage-lsp/src python -m pytest packages/sage-lsp/tests`
- Result: scaffold committed; validation completed after a later package-init fix

### Follow-ups

- Next task: SYN-001
- Risks or blockers: server runtime still requires external Python packages

## Entry 6

- Date: 2026-03-31
- Task ID: SYN-001
- Scope: syntax
- Related milestone: Syntax scaffold
- Commit: `e4c641d`

### Goal

Add an isolated syntax package and a sync script that materializes runtime assets into the extension package.

### Decisions

- Decision: use generated extension-local syntax assets instead of direct cross-package runtime references.
- Reason: extension runtime paths stay stable while syntax assets remain independently owned.

### Verification

- Checks run: `node scripts/sync-syntax-assets.mjs`
- Result: syntax assets synced successfully

### Follow-ups

- Next task: OPS-001
- Risks or blockers: TypeScript build still depends on installing npm packages

## Entry 7

- Date: 2026-03-31
- Task ID: OPS-001
- Scope: repo
- Related milestone: Repository bootstrap
- Commit: `e7ef736`

### Goal

Add onboarding documents, align root scripts with package order, and introduce a bootstrap GitHub Actions workflow.

### Decisions

- Decision: make Python tests work with `PYTHONPATH` from the repository root.
- Reason: local editable installation should be recommended, but bootstrap checks should still be explainable from repo state.

### Verification

- Checks run:
  - `node scripts/sync-syntax-assets.mjs --check`
  - `PYTHONPATH=packages/sage-lsp/src python -m pytest packages/sage-lsp/tests`
- Result: syntax sync check passed; Python tests passed after narrowing `sage_lsp.__init__`

### Follow-ups

- Next task: first `.sage` preprocessing design/implementation slice
- Risks or blockers: npm-based build and extension-host tests were not run because this new repository has not installed Node dependencies yet

## Entry 8

- Date: 2026-03-31
- Task ID: OPS-002
- Scope: repo
- Related milestone: Repository bootstrap
- Commit: `9ccef30`

### Goal

Fix the first root-level `npm install` failure so Node dependencies can be installed and the real build/test chain can start.

### Decisions

- Decision: remove the non-essential `workspace:*` dependency from the extension package.
- Reason: syntax assets are already copied into extension-owned resources by the sync script, so runtime dependency wiring was unnecessary.

### Verification

- Checks run: reran `npm install`
- Result: root install succeeded after the manifest fix

### Follow-ups

- Next task: OPS-003
- Risks or blockers: build-time TypeScript issues could still appear after dependencies are installed

## Entry 9

- Date: 2026-03-31
- Task ID: OPS-003
- Scope: extension
- Related milestone: Repository bootstrap
- Commit: `211c49c`

### Goal

Fix the first TypeScript build error exposed by the newly installed toolchain.

### Decisions

- Decision: treat the language client instance itself as the subscription and await `start()` separately.
- Reason: `start()` returned a promise, not a disposable object expected by VS Code subscriptions.

### Verification

- Checks run: `npm run build`
- Result: root build succeeded

### Follow-ups

- Next task: OPS-004
- Risks or blockers: full lint and test chain still needed to be exercised

## Entry 10

- Date: 2026-03-31
- Task ID: OPS-004
- Scope: repo
- Related milestone: Repository bootstrap
- Commit: pending

### Goal

Run the full local bootstrap validation chain after dependency installation and initial build fixes.

### Decisions

- Decision: keep the validation path repository-root first, so CI and local development use the same commands.
- Reason: one canonical command chain reduces drift between developer machines and automation.

### Verification

- Checks run:
  - `npm install`
  - `python -m pip install -e './packages/sage-lsp[dev]'`
  - `npm run build`
  - `npm run lint`
  - `npm run test`
- Result: all commands completed successfully

### Follow-ups

- Next task: first real `.sage` preprocessing slice
- Risks or blockers: extension tests still cover only a placeholder path
