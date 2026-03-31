# llm-inspect-wrappers — agent notes

## Goal

Thin, best-effort wrappers around `fd`, `rg`, and ranged `sed` reads that emit LLM-efficient output.

Key rule: **never fail** — for unsupported flags / parse errors / tool failures, passthrough to the canonical tool output.

## Layout

- `docs/context_and_task.md` — problem statement + requirements
- `bash/xwrap` — bash multicall implementation (`fd-x`, `rg-x`, `sed-x`)
- `python/llm_inspect.py` — python multicall implementation
- `rust/` — rust crate + multicall binary
- `tests/` — cross-validates all 3 implementations

## Fast commands

- Run all tests: `scripts/test_all.sh`
- Build + package release artifacts: `scripts/release_local.sh --version vX.Y.Z`

## Invariants (treat as contract)

- Output formats must stay consistent across implementations; `tests/` is the living spec.
- Bash must remain compatible with macOS `/bin/bash` 3.2 (no Bash 4+ features like associative arrays, no `lastpipe`).
- Preserve canonical semantics: if an `fd`/`rg` flag changes the output shape (context, replacements, custom formats, etc.), passthrough.
- Determinism: wrapped `rg` calls ignore `RIPGREP_CONFIG_PATH` (passthrough keeps the original environment).
- Avoid committing machine-local paths; pre-commit runs `internal/check_repo_hygiene.py`.

## Repo-inspection hygiene (for agentic sessions)

- Prefer `git ls-files` for deterministic inventories before wide `rg` searches.
- Use narrow `rg` scopes (e.g. `bash/`, `python/`, `rust/`, `tests/`) and cap outputs.
- Use bounded reads with line numbers; wrappers exist to avoid token blowups from huge lines/logs.

## Commits

Use Conventional Commits and keep changes incremental (per implementation / per concern).
