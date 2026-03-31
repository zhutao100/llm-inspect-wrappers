#!/usr/bin/env bash
set -euo pipefail

repo_root="$(
  cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." >/dev/null 2>&1
  pwd -P
)"

cd "$repo_root"

python3 -m unittest discover -s python/tests -q
python3 -m unittest discover -s bash/tests -q
tmp_log="$(mktemp "${TMPDIR:-/tmp}/llm-inspect-wrappers.cargo-test.XXXXXX")"
trap 'rm -f "$tmp_log"' EXIT
if ! (cd rust && cargo test -q) >"$tmp_log" 2>&1; then
  cat "$tmp_log" >&2
  exit 1
fi
python3 -m unittest discover -s tests -q
