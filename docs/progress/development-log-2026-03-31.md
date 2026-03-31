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

## Entry 21

- Date: 2026-03-31
- Task ID: RUNTIME-002
- Scope: runtime
- Related milestone: Runtime hardening
- Commit: `6c445bd`

### Goal

Fix the extension-host startup failure caused by launching `sage-lsp` inside Sage's bundled Python, which lacked
`pygls` and rejected newer Python syntax used by the server code.

### Decisions

- Decision: separate the language-server host Python from `sage.interpreter.path` and expose it as `sage.languageServer.pythonPath`.
- Reason: running the editor-side LSP transport inside Sage itself is brittle and unnecessarily couples static tooling to Sage's bundled Python environment.
- Decision: remove Python 3.10+/3.11+-only syntax from the server code and relax packaging metadata to Python 3.9+.
- Reason: even when Sage is not used to host the server, the source tree should remain importable from common Sage Python builds.

### Verification

- Checks run:
  - `npm run lint`
  - `npm run test`
  - `PYTHONPYCACHEPREFIX=/tmp/sage-lsp-pycache /workspace/sage/sage -python -m py_compile packages/sage-lsp/src/sage_lsp/*.py`
  - `PYTHONPATH=packages/sage-lsp/src python - <<'PY' ... from sage_lsp.server import create_server ... PY`
  - `PYTHONPYCACHEPREFIX=/tmp/sage-lsp-pycache PYTHONPATH=packages/sage-lsp/src /workspace/sage/sage -python - <<'PY' ... from sage_lsp.environment import SageEnvironment ... PY`
- Result: the repository test suite stayed green, the normal Python environment can host the `pygls` server, and Sage's Python 3.9 can now parse and import the server-side modules used for shared logic

### Follow-ups

- Next task: add extension-host smoke coverage
- Risks or blockers: users may still need to set `sage.languageServer.pythonPath` explicitly when VS Code is launched without access to the intended Python environment

## Entry 22

- Date: 2026-03-31
- Task ID: RUNTIME-003
- Scope: runtime
- Related milestone: Runtime hardening
- Commit: `2c4f3ef`

### Goal

Stop the extension from disposing its own language-server connection when several restart triggers arrive close together
during activation and workspace configuration churn.

### Decisions

- Decision: serialize language-server restarts in the extension and coalesce overlapping restart requests into a single lifecycle loop.
- Reason: concurrent `start()` and `stop()` calls on different `LanguageClient` instances were the most plausible explanation for repeated startup lines followed by a clean exit code.
- Decision: explicitly handle `workspace/didChangeConfiguration` on the server and stop asking the client library to auto-forward configuration changes.
- Reason: the extension already owns restart-on-config-change, so duplicate notifications only added noise without value.

### Verification

- Checks run:
  - `npm run test`
  - protocol-level stdio probe: `initialize -> initialized -> workspace/didChangeConfiguration -> didOpen -> hover`
- Result: repository tests stayed green, the manual probe kept the server alive after configuration notifications, and `stderr` stayed empty

### Follow-ups

- Next task: add extension-host smoke coverage
- Risks or blockers: the extension lifecycle fix is covered by protocol probing and repository tests, but not yet by a true VS Code extension-host automation layer

## Entry 23

- Date: 2026-03-31
- Task ID: LSP-008
- Scope: lsp
- Related milestone: Native source support
- Commit: `87c7f0f`

### Goal

Treat Sage-native library sources as first-class analysis inputs instead of reducing the plugin to `.sage` and `.pyx`
only.

### Decisions

- Decision: extend the lightweight parser to cover `.pxd` and `.pxi` inputs alongside `.pyx`.
- Reason: Sage's native components often split declarations and implementations across those file types, so skipping
  them loses useful symbols.
- Decision: merge module records when `.pyx` and `.pxd` represent the same logical module.
- Reason: declaration-only symbols should remain visible without letting `.pxd` override the authoritative implementation
  file for hover and navigation metadata.

### Verification

- Checks run: `npm run test`
- Result: request-level server tests plus parser/index coverage now include `cimport` resolution and native module merges

### Follow-ups

- Next task: register the new file types in the extension and syntax layer
- Risks or blockers: parsing remains intentionally lightweight and does not yet model the full Cython grammar

## Entry 24

- Date: 2026-03-31
- Task ID: EXT-006
- Scope: extension
- Related milestone: Native source support
- Commit: `41a6a87`

### Goal

Ensure the VS Code extension actually activates and forwards native Sage/Cython documents to the language server.

### Decisions

- Decision: introduce a dedicated `sagemath-cython` language id rather than overloading `sagemath`.
- Reason: this keeps native-source behavior explicit while still allowing the grammar and settings surface to remain
  shared.
- Decision: expand both `documentSelector` and file-system watchers to cover `.pyx`, `.pxd`, and `.pxi`.
- Reason: indexing support is wasted if the extension never routes those documents through the client lifecycle.

### Verification

- Checks run: `npm run test`
- Result: extension build and unit coverage stayed green after the new document registrations

### Follow-ups

- Next task: land matching syntax support and smoke examples
- Risks or blockers: extension-host automation for native documents is still future work

## Entry 25

- Date: 2026-03-31
- Task ID: SYN-002
- Scope: syntax
- Related milestone: Native source support
- Commit: `79ee691`

### Goal

Replace the placeholder syntax assets with something usable for both `.sage` and Sage-native Cython sources.

### Decisions

- Decision: keep one shared TextMate grammar for `.sage`, `.pyx`, `.pxd`, and `.pxi`.
- Reason: Sage and its native components overlap heavily enough that a shared grammar reduces drift and duplication.
- Decision: add native declarations and preparser assignments to the smoke workspace.
- Reason: highlighting changes are easier to validate when the repository ships its own representative files.

### Verification

- Checks run:
  - `npm run lint`
  - `npm run test`
- Result: generated syntax assets stayed in sync and the expanded smoke fixtures did not break the repository test chain

### Follow-ups

- Next task: keep expanding native-source semantic coverage beyond highlighting
- Risks or blockers: the current grammar is still hand-maintained and does not yet embed the full upstream Python grammar

## Entry 26

- Date: 2026-03-31
- Task ID: DEV-001
- Scope: repo
- Related milestone: Developer workflow
- Commit: `e3b42d4`

### Goal

Give contributors a single command that can prepare the repository and open the right VS Code development context.

### Decisions

- Decision: make the helper script repository-local and expose it through root npm scripts.
- Reason: the workflow should be discoverable from the repo itself instead of living in ad hoc shell history.
- Decision: keep bootstrap, build, and open steps controllable through flags such as `--bootstrap`, `--skip-build`, and
  `--no-open`.
- Reason: contributors need the same entrypoint to work for both first-time setup and fast local rebuild cycles.

### Verification

- Checks run:
  - `./scripts/dev-vscode.sh --help`
  - `./scripts/dev-vscode.sh --dry-run --bootstrap --python python --smoke --no-open`
  - `./scripts/dev-vscode.sh --no-open --smoke`
- Result: the script prints clear usage, correctly describes the bootstrap workflow, and successfully runs the sync/build
  path without launching a GUI when requested

### Follow-ups

- Next task: extension-host smoke automation remains the next development workflow target
- Risks or blockers: opening the GUI still depends on the user having the `code` CLI available in their PATH

## Entry 27

- Date: 2026-03-31
- Task ID: LSP-009
- Scope: lsp
- Related milestone: LSP baseline
- Commit: `4160448`

### Goal

Push the plugin closer to the everyday baseline that users expect from tools like rust-analyzer or Python LSPs instead
of stopping at hover and definition only.

### Decisions

- Decision: implement workspace symbols, references, rename, and diagnostics as static index-driven features first.
- Reason: these are high-value baseline capabilities that do not need full runtime introspection to become useful.
- Decision: keep diagnostics conservative and limited to unresolved imports for now.
- Reason: low-noise diagnostics are more valuable than a noisy pseudo-checker when the server still lacks full type and
  runtime knowledge.
- Decision: treat renamed import aliases differently from same-name imports.
- Reason: `from pkg import helper` should follow the underlying symbol across files, while `from pkg import helper as
  local_helper` should remain a local alias rename.

### Verification

- Checks run:
  - `PYTHONPATH=packages/sage-lsp/src python -m pytest packages/sage-lsp/tests/test_index.py packages/sage-lsp/tests/test_server.py`
  - `npm run lint`
  - `npm run test`
- Result: the request-level LSP suite now covers workspace symbol search, cross-file references, rename edits, and
  diagnostics publication without regressing the existing extension or server tests

### Follow-ups

- Next task: add signature help, semantic tokens, and extension-host automation on top of the current baseline
- Risks or blockers: references and rename are still lexical/static and will need deeper `.sage` source mapping before
  they can match runtime-preparsed code perfectly

## Entry 28

- Date: 2026-03-31
- Task ID: LSP-010
- Scope: lsp
- Related milestone: LSP baseline
- Commit: `2d00d29`

### Goal

Remove the false unresolved-import errors that appeared when loose `.sage` files used `lazy_import(..., "name", "alias")`
or the list-based alias form.

### Decisions

- Decision: stop parsing loose-line `lazy_import(...)` calls with ad hoc regex splitting.
- Reason: the alias form was being misread as two imported names, which broke navigation and surfaced bogus diagnostics.
- Decision: reuse the AST-based lazy-import parser even for loose `.sage` lines, then relocate source ranges back onto the
  original line.
- Reason: this keeps alias handling consistent between Python and loose `.sage` parsing without duplicating argument
  semantics.

### Verification

- Checks run:
  - `PYTHONPATH=packages/sage-lsp/src python -m pytest packages/sage-lsp/tests/test_parser.py packages/sage-lsp/tests/test_server.py packages/sage-lsp/tests/test_index.py`
  - `npm run lint`
  - `npm run test`
- Result: alias-based lazy imports now resolve to their underlying symbols and the full repository suite remains green

### Follow-ups

- Next task: keep hardening `.sage` source mapping and higher-level LSP features
- Risks or blockers: loose `.sage` parsing is still heuristic and will need more work as preparser coverage expands

## Entry 29

- Date: 2026-03-31
- Task ID: DEV-002
- Scope: repo
- Related milestone: Developer workflow
- Commit: `fc0f838`

### Goal

Fix the `npm run dev:vscode:smoke` bootstrap path so contributors on Node 20 can run the repository helper without the
syntax sync step crashing immediately.

### Decisions

- Decision: replace `import.meta.dirname` with `fileURLToPath(import.meta.url)` plus `path.dirname(...)` in the sync
  script.
- Reason: `import.meta.dirname` is not available in the Node version the user was actually running, while the URL-based
  form is stable across Node 20 and newer.

### Verification

- Checks run:
  - `npm run sync:syntax`
  - `npm run dev:vscode:smoke -- --no-open`
  - `npm run lint`
  - `npm run test`
- Result: syntax sync, build, lint, and the helper-driven smoke path all succeed after the compatibility change

### Follow-ups

- Next task: keep the helper flow stable while extension-host automation coverage is added
- Risks or blockers: the helper still depends on the `code` CLI being installed when you want it to actually launch VS Code

## Entry 30

- Date: 2026-03-31
- Task ID: LSP-011
- Scope: lsp
- Related milestone: LSP baseline
- Commit: `3659e98`

### Goal

Stop document open/change from crashing the language server when diagnostics are published through the `pygls` runtime.

### Decisions

- Decision: publish diagnostics through `textDocument/publishDiagnostics` payload objects instead of calling a missing convenience method on the server.
- Reason: the `pygls` version in the real environment exposes `text_document_publish_diagnostics(...)`, not `publish_diagnostics(...)`.

### Verification

- Checks run:
  - `PYTHONPATH=packages/sage-lsp/src python -m pytest packages/sage-lsp/tests/test_server.py`
  - `npm run lint`
  - `npm run test`
- Result: diagnostics publication now works on document open/change without raising `AttributeError`, and the full repository suite remains green

### Follow-ups

- Next task: keep tightening runtime behavior exposed by real extension-host testing
- Risks or blockers: diagnostics are still intentionally conservative and need richer Sage-aware analysis later

## Entry 31

- Date: 2026-03-31
- Task ID: EXT-007
- Scope: extension
- Related milestone: Developer workflow
- Commit: `1189a08`

### Goal

Make `Sage: Select Interpreter` behave more like mature Python tooling by pre-detecting usable Sage and Python runtimes
instead of forcing every selection through a blank input box.

### Decisions

- Decision: split detected candidates into Sage-runtime targets and language-server-Python targets.
- Reason: the extension now has separate settings for execution (`sage.interpreter.path`) and the LSP host runtime
  (`sage.languageServer.pythonPath`), so a Python choice should not overwrite the Sage runtime setting.
- Decision: detect Python candidates from PATH, workspace virtual environments, and common local install roots in
  addition to system Sage locations.
- Reason: VS Code launched from the GUI often has an incomplete PATH, so relying on PATH-only discovery is fragile.
- Decision: keep custom-path entry points and an explicit `auto` reset option in the same quick-pick flow.
- Reason: contributors need a single command that covers detected, manual, and fallback runtime selection paths.

### Verification

- Checks run:
  - `npm run sync:syntax`
  - `npm run lint`
  - `npm run test`
  - `./scripts/dev-vscode.sh --smoke --no-open`
- Result: the picker now lists detected Sage/Python candidates with dedicated actions, and the repository-wide build,
  test, and helper flows remain green

### Follow-ups

- Next task: add extension-host smoke coverage around runtime selection and actual language-server startup
- Risks or blockers: local interpreter discovery still focuses on common filesystem layouts and does not yet enumerate
  every environment manager

## Entry 32

- Date: 2026-03-31
- Task ID: EXT-008
- Scope: extension
- Related milestone: Runtime hardening
- Commit: `5e4319e`

### Goal

Make real Sage library navigation work without requiring users to hand-configure `sage.analysis.sourceRoots` before
hover, definition, and docs can see Sage's own sources.

### Decisions

- Decision: extend workspace discovery with interpreter-derived Sage roots when explicit source roots are absent.
- Reason: the selected runtime is the strongest local signal for where Sage's importable sources actually live.
- Decision: combine two discovery strategies: filesystem heuristics first, then a short runtime import probe.
- Reason: local source checkouts should resolve instantly from path layout, while packaged Sage installs still need a
  reliable fallback.
- Decision: keep explicit `sage.analysis.sourceRoots` authoritative.
- Reason: manual configuration must remain the escape hatch for unusual layouts and reproducible debugging.

### Verification

- Checks run:
  - `npm run lint`
  - `npm run test`
  - manual probe with `buildWorkspaceInitializationData(...)` against `/workspace/sage/sage`
- Result: the extension test suite stays green, and the local Sage runtime now contributes
  `/workspace/sage/src` automatically to the language-server source roots

### Follow-ups

- Next task: add runtime doc/source fallback for symbols that still cannot be resolved statically
- Risks or blockers: packaged Sage distributions with unusual layouts may still require manual source-root overrides
