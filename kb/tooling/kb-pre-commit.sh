#!/usr/bin/env bash
set -euo pipefail

# Pre-commit entrypoint:
# - mechanically regenerates `kb/gen/*` for the staged set (if stale),
# - auto-stages regenerated artifacts,
# - then runs the canonical kb gate.

repo_root="$(git rev-parse --show-toplevel 2>/dev/null)"
cd "${repo_root}"

kb_bin=""
if [[ -x "${repo_root}/.kb-tool/bin/kb" ]]; then
  kb_bin="${repo_root}/.kb-tool/bin/kb"
elif command -v kb >/dev/null 2>&1; then
  kb_bin="$(command -v kb)"
else
  echo "error: kb not found; expected ${repo_root}/.kb-tool/bin/kb or kb in PATH" >&2
  echo "hint: bash kb/tooling/install_kb.sh" >&2
  exit 2
fi

if ! "${kb_bin}" index check --diff-source staged --format text >/dev/null 2>&1; then
  if ! command -v ctags >/dev/null 2>&1; then
    echo "error: ctags not found in PATH (required for kb index regen)" >&2
    exit 2
  fi

  "${kb_bin}" index regen --scope all --diff-source staged --format text >/dev/null
  git add kb/gen
fi

"${repo_root}/kb/tooling/kb-gate.sh" staged
