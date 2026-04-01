#!/usr/bin/env bash
set -euo pipefail

# Allow `scripts/setup_agentic_autostage_shim.sh` to tune convergence by writing
# `.githooks/.prek-autostage-env`.
# shellcheck disable=SC1091
[[ -f "$(dirname "$0")/.prek-autostage-env" ]] && source "$(dirname "$0")/.prek-autostage-env"

# Self-healing pre-commit shim for prek:
# - Runs prek on the staged set
# - If hooks modified staged files in the working tree, auto-stage and re-run
# - Fails only when remaining failures are not auto-resolvable (lint/typecheck/tests, etc.)

MAX_ROUNDS="${PREK_AUTOFIX_MAX_ROUNDS:-5}"

if ! command -v prek >/dev/null 2>&1; then
  echo "error: prek not found in PATH" >&2
  exit 2
fi

repo_root="$(git rev-parse --show-toplevel 2>/dev/null || true)"
if [[ -z "${repo_root}" ]]; then
  echo "error: not in a git repository" >&2
  exit 2
fi
cd "${repo_root}"

# Capture the initial staged set (exclude deletions so git-add is safe).
files=()
while IFS= read -r -d '' f; do
  files+=("$f")
done < <(git diff --cached --name-only -z --diff-filter=ACMR 2>/dev/null || true)
if [[ "${#files[@]}" -eq 0 ]]; then
  exit 0
fi

git_dir="$(git rev-parse --git-dir)"
lock_dir="${git_dir}/.prek-autostage.lockdir"

acquire_lock() {
  local attempts=0
  local max_attempts=200
  while ! mkdir "${lock_dir}" 2>/dev/null; do
    attempts=$((attempts + 1))
    if [[ "${attempts}" -ge "${max_attempts}" ]]; then
      echo "error: could not acquire hook lock: ${lock_dir}" >&2
      return 1
    fi
    sleep 0.05
  done
  trap 'rmdir "${lock_dir}" 2>/dev/null || true' EXIT
}

git_add_with_retry() {
  local -a delays=(0.05 0.05 0.1 0.1 0.2 0.2 0.4 0.4 0.8 0.8 1 1)
  local out
  for d in "${delays[@]}"; do
    if out="$(git add -- "${files[@]}" 2>&1)"; then
      return 0
    fi
    if [[ "${out}" == *"index.lock"* ]]; then
      sleep "${d}"
      continue
    fi
    echo "${out}" >&2
    return 1
  done
  out="$(git add -- "${files[@]}" 2>&1)" || { echo "${out}" >&2; return 1; }
}

unstaged_changes_in_staged_set() {
  # returns 0 if there ARE unstaged changes for files[], 1 otherwise
  if ! git diff --quiet -- "${files[@]}" 2>/dev/null; then
    return 0
  fi
  return 1
}

acquire_lock

for round in $(seq 1 "${MAX_ROUNDS}"); do
  set +e
  prek run
  status=$?
  set -e

  if [[ "${status}" -eq 0 ]]; then
    exit 0
  fi

  if unstaged_changes_in_staged_set; then
    git_add_with_retry
    continue
  fi

  exit "${status}"
done

echo "error: prek did not converge after ${MAX_ROUNDS} rounds (possible formatter ping-pong)" >&2
exit 1
