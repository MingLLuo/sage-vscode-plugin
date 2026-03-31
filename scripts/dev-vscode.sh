#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WORKSPACE_MODE="repo"
BOOTSTRAP=0
SKIP_BUILD=0
SKIP_OPEN=0
DRY_RUN=0
CODE_BIN="${CODE_BIN:-code}"
PYTHON_BIN="${PYTHON_BIN:-python}"

usage() {
  cat <<'EOF'
Usage: ./scripts/dev-vscode.sh [options]

Prepare the repository for VS Code development, then open the repository so the
local F5 launch configurations can start the extension host.

Options:
  --smoke         Print smoke-workspace follow-up steps after opening VS Code.
  --bootstrap     Run npm install and editable-install the Python language server package first.
  --python BIN    Python executable used for the optional editable install. Default: python
  --code BIN      VS Code CLI executable to launch. Default: code
  --skip-build    Skip syntax sync and build steps.
  --no-open       Prepare the repo but do not launch VS Code.
  --dry-run       Print commands without executing them.
  --help          Show this message.

Examples:
  ./scripts/dev-vscode.sh
  ./scripts/dev-vscode.sh --smoke
  ./scripts/dev-vscode.sh --bootstrap --python /Users/example/miniforge3/bin/python
EOF
}

log() {
  printf '[dev-vscode] %s\n' "$*"
}

run_cmd() {
  printf '+'
  for arg in "$@"; do
    printf ' %q' "$arg"
  done
  printf '\n'
  if [[ "$DRY_RUN" -eq 0 ]]; then
    "$@"
  fi
}

require_command() {
  if ! command -v "$1" >/dev/null 2>&1; then
    log "missing required command: $1"
    exit 1
  fi
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --smoke)
      WORKSPACE_MODE="smoke"
      ;;
    --bootstrap)
      BOOTSTRAP=1
      ;;
    --python)
      shift
      [[ $# -gt 0 ]] || { log "--python requires a value"; exit 1; }
      PYTHON_BIN="$1"
      ;;
    --code)
      shift
      [[ $# -gt 0 ]] || { log "--code requires a value"; exit 1; }
      CODE_BIN="$1"
      ;;
    --skip-build)
      SKIP_BUILD=1
      ;;
    --no-open)
      SKIP_OPEN=1
      ;;
    --dry-run)
      DRY_RUN=1
      ;;
    --help)
      usage
      exit 0
      ;;
    *)
      log "unknown option: $1"
      usage
      exit 1
      ;;
  esac
  shift
done

require_command npm

if [[ "$BOOTSTRAP" -eq 1 ]]; then
  require_command "$PYTHON_BIN"
fi

if [[ "$SKIP_OPEN" -eq 0 ]]; then
  require_command "$CODE_BIN"
fi

cd "$ROOT_DIR"

if [[ "$BOOTSTRAP" -eq 1 ]]; then
  log "bootstrapping Node and Python dependencies"
  run_cmd npm install
  run_cmd "$PYTHON_BIN" -m pip install -e "./packages/sage-lsp[dev]"
fi

if [[ "$SKIP_BUILD" -eq 0 ]]; then
  log "syncing syntax assets and building the repository"
  run_cmd npm run sync:syntax
  run_cmd npm run build
fi

if [[ "$SKIP_OPEN" -eq 0 ]]; then
  log "opening the repository in VS Code"
  run_cmd "$CODE_BIN" "$ROOT_DIR"
fi

if [[ "$WORKSPACE_MODE" == "smoke" ]]; then
  log "next steps: press F5 and choose 'Sage Plugin: Smoke Workspace'"
else
  log "next steps: press F5 and choose 'Sage Plugin: Extension Host'"
fi

log "then run 'Sage: Select Interpreter' inside the extension host window"
