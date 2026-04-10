# kb — agent recipe (repo inspection + in-session updates)

This repo is **kb-enabled** (it has a committed `kb/` root). Prefer `kb` over wide filesystem scans to minimize IO churn and keep updates commit-gated.

## 0) Preflight (fast)

- Verify generated artifacts are current:
  - `kb index check --diff-source worktree`
- If you are about to commit:
  - `kb plan diff --diff-source staged --policy default --format json`
  - `bash kb/tooling/kb-gate.sh staged`

## 1) Discovery (typed; no fuzzy search)

- List what exists:
  - `kb list modules --format text`
  - `kb list tags --format text`
- Read a module card:
  - `kb describe module --id <MODULE_ID> --format json`
- Facts (discovery → exact lookup):
  - `kb list facts --format text`
  - `kb describe fact --id <FACT_ID> --format json`

## 2) “Single-call context” for code review / debugging

- For a change-set (preferred):
  - `kb pack diff --diff-source {staged|worktree} --format json`
- For exact selectors:
  - `kb pack selectors --module <MODULE_ID> --format json`
  - `kb pack selectors --path <PATH> --format json`

Notes:

- `pack selectors --module <MODULE_ID>` expands the module card’s `entrypoints`/`edit_points` and includes `related_facts` automatically.
- `pack selectors --path <DIR/>` includes a bounded subtree under the directory prefix.

Then open only the specific files/line ranges you still need (prefer bounded `sed-x` slices rather than dumping files).

## 3) In-session updates (what to edit when gates fail)

- Module cards: `kb/atlas/modules/<MODULE_ID>.toml`
- Facts: `kb/facts/facts.jsonl`
- Sessions:
  - `kb session init --id <SESSION_ID> [--tag <TAG>]...`
  - Edit the capsule to record decisions/pitfalls/verification (no absolute paths).
  - `kb session finalize --id <SESSION_ID> --diff-source staged --verification tests --verification lint`
