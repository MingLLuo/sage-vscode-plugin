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
- Commit: `5285b09`

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

## Entry 11

- Date: 2026-03-31
- Task ID: LSP-002
- Scope: lsp
- Related milestone: Source mapping v1
- Commit: `02de2b4`

### Goal

Land the first real `.sage` preprocessing feature by rewriting caret exponent syntax into Python power syntax while preserving usable source-position mapping.

### Decisions

- Decision: support only standalone code-region `^` rewrites in v1.
- Reason: this delivers a real Sage-specific transform without overreaching into a full preparser.
- Decision: skip strings, comments, and triple-quoted blocks.
- Reason: these regions should remain lexically stable in the first mapping implementation.

### Verification

- Checks run:
  - `python -m pytest packages/sage-lsp/tests`
  - `npm run build`
  - `npm run test`
- Result: Python tests passed with 6 total tests; repository build and test commands remained green

### Follow-ups

- Next task: LSP-003
- Risks or blockers: only caret rewrite is supported, and hover currently uses preprocessing only for preview rather than full diagnostics/navigation

## Entry 12

- Date: 2026-03-31
- Task ID: LSP-004
- Scope: lsp
- Related milestone: Static source intelligence baseline
- Commit: `7ec87db`

### Goal

Move the language server from a narrow scaffold to a usable static-analysis baseline by porting parser, workspace index,
fixture corpus, and richer symbol-resolution request handling into the `pygls` server.

### Decisions

- Decision: reuse the proven local static-index architecture instead of rebuilding parser and import-resolution logic from scratch.
- Reason: the nearby repository already validated this shape against reduced Sage source fixtures.
- Decision: keep `pygls` as the transport layer while replacing the analysis core underneath it.
- Reason: transport and analysis are separate concerns, and `pygls` remains a good fit for the editor-facing protocol layer.

### Verification

- Checks run:
  - `python -m pytest packages/sage-lsp/tests`
  - `npm run build`
  - `npm run test`
- Result: Python parser/index/source-map suite passed with 11 tests; repository build and full test path remained green

### Follow-ups

- Next task: EXT-002
- Risks or blockers: diagnostics, references, and rename are still pending

## Entry 13

- Date: 2026-03-31
- Task ID: EXT-002
- Scope: extension
- Related milestone: Extension workflow baseline
- Commit: `2b5c899`

### Goal

Upgrade the VS Code client from a thin bootstrap shell into a richer editor workflow with environment presentation,
documentation rendering, run commands, and a stronger settings model.

### Decisions

- Decision: port the stronger local extension-side configuration and documentation modules into this repository.
- Reason: these modules already fit the current monorepo structure and materially improve usability.
- Decision: keep syntax asset generation rooted in this repository's existing `resources/generated` layout.
- Reason: the current syntax package and root sync workflow were already stable and tested.

### Verification

- Checks run:
  - `npm run build`
  - `npm run lint`
  - `npm run test`
- Result: extension unit tests passed with 11 tests and the repository-wide build/lint/test path stayed green

### Follow-ups

- Next task: EXT-003
- Risks or blockers: extension coverage is still unit-heavy rather than extension-host-heavy

## Entry 14

- Date: 2026-03-31
- Task ID: RUNTIME-001
- Scope: runtime
- Related milestone: Runtime hardening
- Commit: `65f2a6a`

### Goal

Close the runtime gap between extension settings and actual server execution by aligning interpreter launch, path resolution,
shell-safe command construction, and file-URI normalization.

### Decisions

- Decision: derive the language-server launch command from the configured interpreter instead of hardcoding `python3`.
- Reason: language intelligence and run commands must target the same Sage or Python environment to stay trustworthy.
- Decision: resolve configured relative source roots and extra paths against workspace folders rather than process cwd.
- Reason: workspace settings should be portable across machines and editor launches.

### Verification

- Checks run:
  - `npm run lint`
  - `python -m pytest packages/sage-lsp/tests`
  - `npm run test`
- Result: runtime launch and path-handling fixes landed with the full repository test path green

### Follow-ups

- Next task: LSP-005
- Risks or blockers: request handlers still needed direct regression coverage

## Entry 15

- Date: 2026-03-31
- Task ID: LSP-005
- Scope: lsp
- Related milestone: Runtime hardening
- Commit: `39cadbb`

### Goal

Exercise the actual `pygls` request handlers instead of only testing the lower-level parser and index modules.

### Decisions

- Decision: call registered `pygls` features through an initialized in-memory workspace instead of mocking the handlers.
- Reason: request-level tests should catch runtime wiring mistakes, including protocol registration and import drift.

### Verification

- Checks run:
  - `python -m pytest packages/sage-lsp/tests`
  - `npm run test`
- Result: initialize, hover, definition, completion, document symbols, and custom documentation requests are now covered

### Follow-ups

- Next task: EXT-004
- Risks or blockers: some extension settings still had no concrete runtime effect

## Entry 16

- Date: 2026-03-31
- Task ID: EXT-004
- Scope: extension
- Related milestone: Runtime hardening
- Commit: `ffc1d3b`

### Goal

Turn the exposed execution settings into real behavior by honoring `sage.run.target`, managing REPL lifecycle explicitly, and
avoiding unnecessary language-server restarts.

### Decisions

- Decision: separate run and REPL terminals, and bootstrap the REPL terminal lazily.
- Reason: file execution and interactive evaluation have different state expectations and should not trample each other.
- Decision: avoid restarting the language server for run-target-only settings.
- Reason: execution-target changes are extension-host concerns, not analysis-server concerns.

### Verification

- Checks run:
  - `npm run lint`
  - `npm run test`
- Result: extension command planning and restart filtering are covered by unit tests and the repository test suite stayed green

### Follow-ups

- Next task: LSP-006
- Risks or blockers: hover preferences still needed to be consumed by the server instead of only flowing through config payloads

## Entry 17

- Date: 2026-03-31
- Task ID: LSP-006
- Scope: lsp
- Related milestone: Runtime hardening
- Commit: `52f9432`

### Goal

Make server behavior reflect the documentation-related settings already exposed by the client payload.

### Decisions

- Decision: extend the server environment model to parse documentation, logging, and experimental sections now.
- Reason: the initialization payload should not silently discard stable settings that the client already treats as first-class.
- Decision: keep hover enabled when documentation previews are disabled, but trim the docstring body.
- Reason: type or signature detail is still useful even when the user does not want long documentation text in hover.

### Verification

- Checks run:
  - `python -m pytest packages/sage-lsp/tests`
  - `npm run test`
- Result: hover behavior now follows the client preference and the expanded environment parsing is covered by tests

### Follow-ups

- Next task: extend source mapping into diagnostics and navigation
- Risks or blockers: extension-host coverage and richer `.sage` transforms are still pending

## Entry 18

- Date: 2026-03-31
- Task ID: LSP-007
- Scope: lsp
- Related milestone: Runtime hardening
- Commit: `1fe8303`

### Goal

Make `analysis.extraPaths` affect the static-analysis graph instead of only the subprocess import environment.

### Decisions

- Decision: resolve extra paths on the client before sending initialization options, then include those roots in server index construction.
- Reason: extra paths are user-facing analysis inputs and should participate in symbol discovery, not just runtime module loading.

### Verification

- Checks run:
  - `npm run lint`
  - `python -m pytest packages/sage-lsp/tests`
  - `npm run test`
- Result: server initialization now indexes modules from configured extra paths and the full repository suite remained green

### Follow-ups

- Next task: extend source mapping into diagnostics and navigation
- Risks or blockers: extension-host coverage and richer `.sage` transforms are still pending

## Entry 19

- Date: 2026-03-31
- Task ID: EXT-005
- Scope: extension
- Related milestone: Local debugging baseline
- Commit: `56b7f3b`

### Goal

Make extension debugging first-class in the repository by providing a local `F5` launch path instead of requiring a manual
`code --extensionDevelopmentPath=...` command.

### Decisions

- Decision: add repository-local launch and task definitions instead of depending on user-global VS Code state.
- Reason: the plugin repository should be self-describing and runnable by any contributor who clones it.
- Decision: enable source maps for the extension package build output.
- Reason: stepping through compiled JavaScript is unnecessary friction when the source is TypeScript.

### Verification

- Checks run:
  - `npm run build`
  - `npm run test`
- Result: the repository debug scaffolding landed without regressing the existing build and test path

### Follow-ups

- Next task: add extension-host smoke coverage
- Risks or blockers: launch configuration still opens the current repository as the default workspace in the extension host

## Entry 20

- Date: 2026-03-31
- Task ID: QA-001
- Scope: examples
- Related milestone: Manual smoke workspace
- Commit: `517ef2b`

### Goal

Ship a ready-made workspace full of `.sage`-oriented examples so manual testing does not depend on inventing ad hoc files every
time the extension changes.

### Decisions

- Decision: make the smoke workspace self-contained with local Python and `.pyx` modules plus an extra-path `vendor` directory.
- Reason: manual testing should stay reproducible even when a full Sage source checkout is unavailable.
- Decision: add a dedicated `Sage Plugin: Smoke Workspace` launch configuration.
- Reason: the fastest way to use manual examples is to open them directly in the extension host, not by navigating there later.

### Verification

- Checks run:
  - `python -m py_compile examples/manual-smoke-workspace/src/local_docs.py examples/manual-smoke-workspace/src/package_demo/__init__.py examples/manual-smoke-workspace/src/package_demo/polynomials.py examples/manual-smoke-workspace/vendor/external_series.py`
  - `PYTHONPATH=packages/sage-lsp/src python - <<'PY' ... WorkspaceIndex([root / 'src', root / 'vendor'], (), True) ... PY`
- Result: sample Python modules compile and the language-server index discovers the source-root, package, extra-path, and `.pyx` modules

### Follow-ups

- Next task: add extension-host smoke coverage
- Risks or blockers: the source-mapping sample includes future-facing cases that exceed the current editor-side mapping integration
