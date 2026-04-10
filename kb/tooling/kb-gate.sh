#!/usr/bin/env bash
set -euo pipefail

# Canonical kb gate sequence for commits and CI.
#
# Usage:
#   kb/tooling/kb-gate.sh staged
#   kb/tooling/kb-gate.sh worktree
#   kb/tooling/kb-gate.sh commit:<sha>

DIFF_SOURCE="${1:-}"
if [[ -z "${DIFF_SOURCE}" ]]; then
  echo "usage: $(basename "$0") <diff-source>" >&2
  echo "  diff-source: staged | worktree | commit:<sha>" >&2
  exit 2
fi

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

"${kb_bin}" index check --diff-source "${DIFF_SOURCE}" --format text >/dev/null
"${kb_bin}" lint all --format text >/dev/null
"${kb_bin}" obligations check --diff-source "${DIFF_SOURCE}" --format text >/dev/null
