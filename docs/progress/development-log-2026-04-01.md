# Development Log: 2026-04-01

## Entry 1

- Date: 2026-04-01
- Task ID: LSP-018
- Scope: lsp
- Related milestone: Runtime hardening
- Commit: `fa0f257`

### Goal

Generalize native Sage documentation so common library symbols resolve useful hover content and source locations beyond
the small set already covered by runtime fallback.

### Decisions

- Decision: merge runtime signatures back into static documentation instead of treating runtime and static analysis as
  mutually exclusive.
- Reason: local source paths from static indexing are valuable, but runtime fallback often carries richer callable
  signatures.
- Decision: inherit docstrings from factory classes and `.pyx` function bodies during static indexing.
- Reason: local Sage source checkouts do not always provide reliable runtime introspection, especially for constructor
  factories and native Cython helpers.

### Verification

- Checks run:
  - `python -m pytest packages/sage-lsp/tests/test_parser.py packages/sage-lsp/tests/test_index.py packages/sage-lsp/tests/test_server.py packages/sage-lsp/tests/test_runtime_introspection.py`
  - direct native-source probe for `graphs.PetersenGraph`, `PolynomialRing`, `EllipticCurve`, `matrix`, `NumberField`, and `Partitions`
- Result: parser, index, server, and runtime-introspection tests passed; native-source probe returned documentation and
  source paths for the targeted library symbols

### Follow-ups

- Next task: QA-004
- Risks or blockers: real VS Code extension-host smoke still requires an approval path that allows local app launch

## Entry 2

- Date: 2026-04-01
- Task ID: QA-004
- Scope: repo/test
- Related milestone: Runtime hardening
- Commit: `685345f`

### Goal

Add a repeatable local smoke command that validates native Sage library support without relying on a GUI launch.

### Decisions

- Decision: ship a repository-local smoke script that exercises the same documentation and definition pipeline against a
  real Sage source checkout.
- Reason: this keeps native-library verification available even when local VS Code app launches are blocked or
  undesirable.

### Verification

- Checks run:
  - `npm run test:native-smoke`
  - `npm run test`
- Result: native Sage smoke passed for `graphs.PetersenGraph`, `PolynomialRing`, `EllipticCurve`, `matrix`,
  `NumberField`, and `Partitions`; repository unit and Python tests also passed

### Follow-ups

- Next task: revisit extension-host native-library smoke once GUI launch approval is available
- Risks or blockers: the non-GUI smoke validates the analysis pipeline, not final command-palette or hover-popup UI
