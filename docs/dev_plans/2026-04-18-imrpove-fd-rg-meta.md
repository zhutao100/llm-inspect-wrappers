## Title: Compact `@meta` v2 for `rg-x` (and file-table wrappers)

## Summary
Redesign wrapper metadata to be materially more token-efficient while staying semantically explicit (no 1–2 character abbreviations). The main win is removing redundant “printed/omitted” prose fields and emitting **only totals by default**, adding **`shown_*` fields only when the wrapper actually truncated output**. Apply this to:
- `rg-x` (match mode + filelist mode)
- file-table meta for `fd-x` and `rg-x` filelist output
Keep `sed-x` metadata keys unchanged (per your selection).

---

## Public output/interface changes (breaking)
### 1) `rg-x` match-mode trailer `@meta` (major change)
**Old (example):**
`@meta tool=rg-x mode=match files=F printed_files=PF omitted_files=OF match_lines=ML printed_match_lines=PML omitted_match_lines=OML`

**New (v2):**
`@meta	tool=rg-x	mode=match	files=F	match_lines=ML	[shown_files=PF]	[shown_match_lines=PML]`

**Rules**
- Always emit `files` and `match_lines` as totals.
- Emit `shown_files` **only if** `shown_files < files`.
- Emit `shown_match_lines` **only if** `shown_match_lines < match_lines`.
- When a `shown_*` field is absent, its value is defined as equal to the corresponding total.

### 2) `rg-x` per-file header `@file` when per-file match lines were capped
**Old:**
`@file … match_lines=M shown=S omitted=K`

**New (v2):**
`@file	…	match_lines=M	shown=S`

**Rules**
- Only include `match_lines`/`shown` when the per-file cap actually omitted lines (`shown < match_lines`).
- Do **not** print `omitted`; it is derivable as `match_lines - shown`.
- Keep existing header fields otherwise (`path=…`, `kind=…` only when non-file, `bytes=…`, `lines=…`).

### 3) File-table wrappers (`fd-x` and `rg-x` filelist) `@meta` (unify + compact)
Applies to:
- `fd-x …` outputs (file table)
- `rg-x -l …` / `rg-x --files-with-matches …` / other filelist modes that produce a file table

**Old:**
`@meta tool=… [mode=filelist] total=T printed=P omitted=O [max_results=N returned=R unseen=U]`

**New (v2):**
- `fd-x`:
  `@meta	tool=fd-x	rows=T	[shown_rows=P]	[max_results=N	returned_rows=R]`
- `rg-x` filelist:
  `@meta	tool=rg-x	mode=filelist	rows=T	[shown_rows=P]`

**Rules**
- `rows` is the total number of tool-returned rows for this invocation:
  - For `fd-x`, preserve existing “uncapped total” behavior when `--max-results` is present and the wrapper can compute the uncapped count.
- Emit `shown_rows` **only if** wrapper truncation occurred (i.e., it printed fewer table rows than it received from the tool under the user’s args).
- For `fd-x`, when `--max-results` is parsed, emit:
  - `max_results=N` (user-provided cap)
  - `returned_rows=R` (rows actually returned by canonical `fd` under the user cap)
- Drop `unseen` (derivable as `rows - returned_rows`).

### 4) `sed-x`
No changes (keep `path=` / `source=stdin`, `complete=`, `reason=`, `truncated_lines=` as-is).

---

## Implementation plan (decision-complete)
### A) Update documentation (spec first)
1. Update `README.md` “Output format” section:
   - Replace `total/printed/omitted` discussion for file tables with `rows` + optional `shown_rows`.
   - Replace `rg-x` match-mode trailer field list with the new v2 schema and rules.
2. Update `docs/caps_and_omissions.md`:
   - `rg-x` file-list mode meta: replace `total/printed/omitted` with `rows` + optional `shown_rows`.
   - `rg-x` match mode:
     - Update the “Trailer meta fields” section to new v2 fields and “absent implies equal” rule.
     - Update the “Per-file header fields” section to remove `omitted` and define derivation.

### B) Bash implementation (`bash/xwrap`)
1. **File-table meta emission**
   - In `fd_x` and `rg_x_filelist`, change meta line construction:
     - Always emit `rows=<total>`.
     - Emit `shown_rows=<printed>` only when `printed < returned`.
     - For `fd-x` with parsed `--max-results`: emit `max_results` and `returned_rows`; remove `unseen`.
2. **`rg_x_match` per-file header**
   - When `current_omitted > 0`, print `match_lines=<current_hits>\tshown=<shown_count>`; remove `omitted=…`.
3. **`rg_x_match` trailer meta**
   - Always print: `tool`, `mode=match`, `files`, `match_lines`.
   - Conditionally append:
     - `shown_files=<files_printed>` if `files_printed < files_seen`
     - `shown_match_lines=<match_lines_printed>` if `match_lines_printed < match_lines_total`
   - Remove `printed_*` / `omitted_*` fields entirely.

### C) Python implementation (`python/llm_inspect.py`)
1. **`render_file_table`**
   - Replace `total/printed/omitted` emission with:
     - `rows=<total_override or len(paths)>`
     - `shown_rows=<len(shown)>` only when `len(shown) < len(paths)` (wrapper truncation)
   - Preserve `tool` and optional `mode`.
2. **`main_fd`**
   - Change meta extras to: `max_results` and `returned_rows` (rename from `returned`), drop `unseen`.
3. **`main_rg_filelist`**
   - Uses updated `render_file_table` output.
4. **`main_rg_json` (match mode)**
   - Change `@file` header when omitted lines exist: remove `omitted`, keep `match_lines` and `shown`.
   - Change trailer `@meta` to:
     - Always: `files`, `match_lines`
     - Conditionally: `shown_files`, `shown_match_lines` only when truncated

### D) Rust implementation (`rust/src/*.rs`)
1. **`rust/src/fdx.rs`**
   - Emit `rows=<total>` always.
   - Emit `shown_rows=<shown>` only when `shown < returned`.
   - When `max_results` present: emit `max_results` and `returned_rows`; drop `unseen`.
2. **`rust/src/rgx.rs`**
   - Filelist mode meta: emit `rows`, optional `shown_rows` (as above).
   - Match mode:
     - `@file` with omitted lines: remove `omitted`, keep `match_lines` and `shown`.
     - Trailer `@meta`: always `files`, `match_lines`, optional `shown_files`, optional `shown_match_lines`.

---

## Tests (update to match v2 “living spec”)
Update tests to assert the new contract across all 3 implementations:

1. `tests/test_cross_validate.py`
   - For match mode:
     - Always assert `meta["files"]` and `meta["match_lines"]` exist and are totals.
     - In “no omit” cases, assert `shown_files` and `shown_match_lines` are absent.
     - In capped cases:
       - Assert `shown_match_lines` exists and is less than `match_lines`.
       - Assert file header contains `match_lines` and `shown`, and does **not** contain `omitted`.
   - For filelist mode:
     - Assert `meta["rows"]` exists.
     - Assert `shown_rows` absent unless the wrapper row cap is forced in the test env (add a test env case if needed).
   - For `fd-x` with `--max-results` coverage (existing):
     - Update assertions to look for `returned_rows` and to derive unseen behavior if needed.
2. `python/tests/test_llm_inspect.py` and `bash/tests/test_xwrap.py`
   - Keep broad prefix checks, but update any direct key expectations (`returned`→`returned_rows`, etc.) if present.

---

## Acceptance criteria / scenarios to verify
- `rg-x` match mode with small results: trailer is short (no `shown_*`), no redundant counts.
- `rg-x` match mode with per-file truncation only: trailer includes `shown_match_lines` but not `shown_files`.
- `rg-x` match mode with file-group truncation: trailer includes `shown_files` (and likely `shown_match_lines`).
- `rg-x` filelist mode: meta uses `rows` (+ optional `shown_rows` only when wrapper truncates).
- `fd-x` with `--max-results`:
  - meta includes `rows`, `max_results`, `returned_rows`
  - `shown_rows` appears only when wrapper truncates printed table rows
- All three implementations produce identical shapes for the same scenarios (tests are the oracle).

---

## Commands to run for verification
- `scripts/test_all.sh`
- (Optional quick) `python3 -m unittest discover -s tests -q`

---

## Commit plan
After tests pass, create a Conventional Commit capturing the breaking meta-schema change, e.g.:
- `feat(meta): compact rg-x and file-table @meta schema`

Body should mention:
- removal of redundant `printed_*`/`omitted_*`
- conditional `shown_*` fields
- `returned`→`returned_rows` rename for `fd-x`
- docs + tests updated as spec

---

## Assumptions (locked)
- Goal is LLM token efficiency over backwards compatibility.
- No changes to `sed-x` meta keys.
- No changes to passthrough behavior (help/version and unsupported rg/fd/sed modes remain canonical output without wrapper meta).
