# Runtime review — 2026-11-07

Reviewed activation and configuration, per-project routing, command execution,
REPL sessions, language-client startup/recovery, documentation requests/rendering,
index cache maintenance, packaging and artifact cleanup.

## Confirmed issues addressed

- Interpreter selection reset every project's REPL. Reset only the selected scope.
- Standalone tasks used the script directory while project REPLs used the project root.
  Use the project root for both; standalone file runs retain a directory fallback.
- Source-root and extra-path changes left old REPL imports alive. Mark affected sessions
  stale, block sending more code, and explicitly restart through Start REPL. Other
  projects retain their sessions. Starting a REPL also initializes configured import paths.
- Helper cleanup deleted pre-existing files and raced overlapping tasks. Track ownership
  across managed task launches, preserve pre-existing or shared helpers and failed launches,
  and never delete disk helpers from REPL load commands.
- Startup cache pruning could delete another project's or another window's database.
  Startup now performs a dry run; explicit cache deletion requires offline maintenance.

- The configuration CLI treated unreadable/invalid JSON settings as empty, risking
  replacement of user settings. Parse JSONC, preserve comments and unrelated settings,
  and fail without writing on invalid input or duplicate top-level keys.

## Validation and boundaries

Regression coverage includes helper ownership and overlapping launches, project working
folders, and preserving another project's REPL when restarting a stale session. Run the
release gate and Extension Host gate for this change. These gates cover Rust analysis,
protocol requests, Unicode rename, highlighting, package/install integrity, performance,
and first-run/single-project/multi-project extension-host scenarios.

Static inspection and automated gates cannot prove the absence of all problems. In
particular, external Sage processes are not tracked by the extension's task registry;
users should avoid concurrently running the same file in external processes when helper
cleanup is enabled (disabled by default). Arbitrary dynamic Python/Sage receiver types
remain conservative navigation candidates. Cross-platform release validation remains
outside the current macOS package scope.

## Observed validation outcome

- Full `test:release` gate passed, including Rust, Clippy, lint, extension units,
  Python tests, LSP protocol/navigation/Unicode/highlighting, VSIX package/install,
  performance and real Sage source checks. Extension units: 213 passed; Python: 96 passed.
- `npm ci --dry-run --offline --ignore-scripts` passed.
- The full Extension Host gate did NOT pass. Isolated VS Code GUI processes repeatedly
  stayed alive after their extension hosts exited, or terminated with SIGTERM. One
  stalled test process was stopped explicitly; these attempts are not counted as passes.
- A focused single-project host run revealed an assertion failure in
  `verifyExternalSageSourceFollowUpWithPythonDisabled`: expected incoming call hierarchy
  items to contain ExternalSmokeCaller with a sage-source URI. Root cause remains
  unconfirmed; investigate the raw LSP incoming-call response and URI conversion next.
- The new multi-project host assertions have not been confirmed passing. The user
  requested wrap-up before further investigation. No new VSIX was installed into the
  user's normal VS Code environment during this audit.
