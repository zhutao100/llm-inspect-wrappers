#!/usr/bin/env bash
set -euo pipefail

repo_root="$(git rev-parse --show-toplevel 2>/dev/null)"
"${repo_root}/kb/tooling/kb-gate.sh" worktree
