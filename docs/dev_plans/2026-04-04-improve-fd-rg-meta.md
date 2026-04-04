# Problem Statement

## `rg-x`

`rg-x` outputs a verbose meta line as quoted below on zero match; this looks dumb and token inefficient.
```
files=0    printed_files=0    omitted_files=0    match_lines=0    printed_match_lines=0    omitted_match_lines=0
```

## `fd-x`

Using `~/workspace/agentic-tools/codex-chat` as a playground example; its sub-dir ` apps/CodexChatApp/Sources/` has >100 files.

When running `fd-x -t f -d 6 --max-results 10 . apps/CodexChatApp/Sources/*`

it returns 10 files plus a meta line as
```
@meta	tool=fd-x	total=10	printed=10	omitted=0
```

- The meta line didn't provide useful information of the **total** number of files (as if running without "--max-results")
- The "total=10	printed=10" is a bit misleading, can be mis-intercepted as "there are 10 files in total"

# Improvement Plan

## Improve `rg-x` zero-match output + fix `fd-x` `--max-results` totals

### Summary
Implement two targeted UX/token-efficiency improvements across **bash**, **python**, and **rust** wrappers, then update docs + tests to lock the new output contract:

1) `rg-x`: on **zero matches** (canonical `rg` exit code `1`), emit **no stdout at all** (no `@meta` line).
2) `fd-x`: when user passes `--max-results N`, compute and report a **true uncapped total** (as if `--max-results` were not present), so the meta line cannot be misread as “there are only N files”.

You selected:
- `rg-x` zero-match behavior: **No output**.
- `fd-x` `--max-results` behavior: **Compute true total**.

## Option Set

### `rg-x` zero matches
- **Option A (Chosen): Empty stdout**
  - Behavior: if there are 0 matches, print nothing; rely on exit code `1`.
  - Pros: most token efficient; matches canonical `rg` UX; removes “dumb” meta-only output.
  - Cons: callers that previously parsed `@meta` even on zero matches must instead handle empty stdout.

- Option B: minimal meta only
  - e.g. `@meta tool=rg-x mode=match match_lines=0`
  - Pros: machine-readable signal without verbosity.
  - Cons: still wastes tokens and still looks like “output” on “no matches”.

### `fd-x` with `--max-results N`
- **Option A (Chosen): compute true uncapped total via a second pass**
  - Pros: accurate; directly addresses “total=10 printed=10” confusion.
  - Cons: can be slower when the search space is huge.

- Option B: “peek +1” (N+1) to detect truncation only
  - Pros: cheap; tells you if there’s at least one more.
  - Cons: still no true total.

## Output Contract Changes (Decision-Complete)

### 1) `rg-x`: suppress meta on zero results (match mode + filelist mode)
For both:
- match mode (`rg-x PATTERN ...`)
- filelist mode (`rg-x -l PATTERN ...`, `rg-x --files ...`, etc.)

**New rule:** if there are **0 result rows** to print (no `@file` groups and no match lines / no file-table rows), then:
- stdout MUST be **empty** (no trailing newline, no `@meta`)
- exit code MUST remain whatever canonical `rg` returned (typically `1` for “no matches”)

**Implementation detail per language:**
- Determine “no results” from already-computed aggregates:
  - match mode: `total_files == 0` (or equivalent)
  - filelist mode: `total_paths == 0`

### 2) `fd-x`: correct totals when `--max-results` is present
Add **max-results awareness** and (when needed) compute a **second-pass uncapped count**.

#### Argument parsing
Detect `--max-results` in both forms:
- `--max-results N`
- `--max-results=N`

Produce:
- `max_results: int | None`
- `args_uncapped`: argv with the `--max-results` flag removed (preserving all other args and `--` semantics)

#### Two-pass counting rule (performance guard)
Only do the second pass **when it might matter**:
- Run pass #1 with original args (includes `--max-results`) to get `returned = number_of_paths_returned`.
- If `max_results` is present AND `returned == max_results`, then run pass #2 (uncapped) to compute `total`.
- Else, set `total = returned` and skip pass #2.

Pass #2 must be **count-only**:
- do not collect paths in memory
- do not compute file metadata
- just count NUL-separated entries from `fd --color=never -0 ...`

#### Meta line fields (new, consistent across implementations)
Keep existing keys and add a small, explicit set when `--max-results` is detected:

Always:
- `@meta\\ttool=fd-x\\ttotal=T\\tprinted=P\\tomitted=O`

Additionally, when `--max-results N` was present and successfully parsed:
- `\\tmax_results=N\\treturned=R\\tunseen=U`

Where:
- `returned = R`: number of paths returned by canonical `fd` under the user’s `--max-results` cap (pass #1)
- `total = T`: uncapped total count if pass #2 ran successfully; otherwise equals `returned`
- `printed = P`: rows actually printed (bounded by `LLM_X_MAX_FD_ROWS` and by `returned`)
- `omitted = O`: `returned - printed` (wrapper-only omission)
- `unseen = U`: `total - returned` (results excluded only because of user `--max-results`)

Fallback safety (“never fail”):
- If pass #2 fails for any reason, do not error; set `total=returned` and `unseen=0`.

## Code Changes (by implementation)

### Bash (`bash/xwrap`)
1) `rg-x`
- In `rg_x_match`: only print the `@meta ... mode=match ...` trailer if `files_seen > 0` (or `match_lines_total > 0`).
- In `rg_x_filelist`: only print `@meta ... mode=filelist ...` trailer if `total > 0`.

2) `fd-x`
- Add a small argv parser inside `fd_x()` to:
  - extract `max_results`
  - build `argv_uncapped` by removing the max-results flag
- After pass #1 finishes:
  - set `returned = total_pass1`
  - if `max_results` set and `returned == max_results`, run pass #2 with `argv_uncapped` and count all paths to `total_uncapped`
- Emit meta with the new fields described above.

### Python (`python/llm_inspect.py`)
1) `rg-x`
- In `main_rg_filelist`: if parsed `paths` is empty, write nothing to stdout and return `cp.returncode`.
- In `main_rg_json`: after parsing, if `groups_by_path` is empty, write nothing to stdout and return `cp.returncode`.

2) `fd-x`
- Parse `--max-results` and create `args_uncapped` in `main_fd`.
- Run pass #1 as today to get `paths` and `returned=len(paths)`.
- If `max_results` present and `returned == max_results`, run pass #2 using a **streaming count** helper (no full capture) to compute `total`.
- Extend `render_file_table(...)` to accept optional meta overrides/extra fields:
  - either pass in `(total, returned, max_results, unseen)` explicitly, or pass a `meta_extra: dict[str,str]` and `total_override`.

### Rust (`rust/src/fdx.rs`, `rust/src/rgx.rs`)
1) `rg-x`
- In the filelist rendering helper (`render_file_table` in `rust/src/rgx.rs`): if `total == 0`, return `status` immediately without printing.
- In match-mode rendering path: if `total_files == 0`, return the `rg` exit code without printing.

2) `fd-x`
- Parse `--max-results` from `args` (both `--max-results N` and `--max-results=N`) and build `args_uncapped`.
- Pass #1: keep current capture (bounded by user max-results).
- If `max_results` present and `returned == max_results`, run pass #2 via `Command::spawn` + streaming read of stdout to count NUL-separated paths (do not `cmd_capture` the whole uncapped output).
- Print meta with the new keys; fallback to `total=returned` if pass #2 fails.

## Docs Updates
Update `docs/caps_and_omissions.md`:
- `rg-x`: add explicit note that on zero matches the wrapper prints **nothing** (no `@meta`) to match canonical `rg` and reduce tokens.
- `fd-x`: update the “Meta meaning” section to document:
  - new fields: `max_results`, `returned`, `unseen`
  - `omitted` now strictly means wrapper omission (vs user cap), while `unseen` captures user-cap exclusion.

## Tests / Acceptance Criteria

### New cross-impl tests (`tests/test_cross_validate.py`)
1) `rg-x` no-match emits empty stdout
- Fixture: a directory with a file that does not contain the pattern.
- Run `rg-x needle` for each impl.
- Assert:
  - `returncode == 1`
  - `stdout == \"\"` (or at least: no non-whitespace; no `@meta\\ttool=rg-x`)
  - `stderr` unconstrained (should typically be empty)

2) `fd-x` uncapped totals with `--max-results`
- Fixture: create 5 files.
- Run `fd-x -tf --max-results 2 .` for each impl.
- Parse meta and assert:
  - `tool=fd-x`
  - `max_results=2`
  - `returned=2`
  - `total=5`
  - `unseen=3`
  - `printed=2`
  - `omitted=0`

### Per-impl unit tests
Add a small `rg-x` no-match test to:
- `bash/tests/test_xwrap.py`
- `python/tests/test_llm_inspect.py`
so regressions show up even if cross-validate is skipped.

### Run verification
- `scripts/test_all.sh` should pass (and will validate consistency across all implementations).

## Assumptions / Defaults
- Treat “zero results” as “print nothing” for `rg-x` in both match and filelist modes.
- For `fd-x`, “true total” means “count of paths that `fd` would return if `--max-results` were removed, keeping all other args identical”.
- Second pass for `fd-x` is triggered only when `returned == max_results` to avoid unnecessary full scans.
- If the second pass fails, fall back silently to `total=returned` (preserving the “never fail” invariant).
