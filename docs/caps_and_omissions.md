# Caps, omissions, and long-line gating

These wrappers intentionally **cap what they print** to avoid flooding an LLM context window, while still reporting **totals** via a trailing `@meta` line.

Key idea:
- **Printed output is bounded.**
- **Totals remain informative** (`total`, `files`, `match_lines`, etc.).
- **Omitted output is not an error**; it is a deliberate truncation with explicit counts.

All wrappers are **best-effort**: if an invocation is unsupported or parsing fails, they **passthrough** to the canonical tool output (which may be unbounded and will not include wrapper `@meta` lines).

---

## Environment variables (defaults)

### Output caps (omit behavior)
- `LLM_X_MAX_FD_ROWS=200`
  - Caps **file-table rows** printed by:
    - `fd-x ...`
    - `rg-x -l ...` / `rg-x --files-with-matches ...` / other `rg` file-list modes
- `LLM_X_MAX_RG_FILES=80`
  - Caps **file groups** printed by `rg-x` **match mode**.
- `LLM_X_MAX_RG_MATCH_LINES_PER_FILE=20`
  - Caps **match lines printed per file** in `rg-x` **match mode** (when output is capped).
- `LLM_X_MAX_RG_NO_OMIT_MATCH_LINES=200`
  - If total match lines across all files is **≤ this value**, `rg-x` prints **all** match lines (no omissions) and disables the file/per-file caps for that run.

### Stdin total scanning caps (`sed-x`)
When `sed-x` reads from **stdin**, it may continue reading after the requested range to compute totals (`lines`, `bytes`). That scan is bounded by:
- `LLM_X_SEDX_STDIN_MAX_LINES=200000`
- `LLM_X_SEDX_STDIN_MAX_BYTES=10000000`

### Long-line gating (token blowup prevention)
Long-line gating applies to:
- `rg-x` match line bodies (in match mode)
- `sed-x` printed lines (file or stdin)

Tuning:
- `LLM_X_SOFT_LINE_CHARS=400` (heuristic “suspiciously long” threshold)
- `LLM_X_HARD_LINE_CHARS=2000` (always-gate threshold)
- `LLM_X_HEAD_CHARS=160` (prefix preview length in a gated marker)
- `LLM_X_TAIL_CHARS=80` (suffix preview length in a gated marker)

---

## `fd-x`: file table cap + totals

`fd-x` only intercepts “simple path list” shapes. If you use flags that change the output shape (e.g. `--exec`, `--format`, `--list-details`, `--print0`, etc.), it will passthrough.

### What gets capped
- Printed rows are capped to `LLM_X_MAX_FD_ROWS`.
- Rows beyond that are **omitted** and only counted in `@meta`.

### Meta meaning
Trailer line:
```
@meta	tool=fd-x	total=T	printed=P	omitted=O	[ max_results=N	returned=R	unseen=U ]
```
- `total`:
  - default: total number of paths returned by canonical `fd`.
  - when user passes `--max-results N` and the wrapper can compute an uncapped count: total number of paths `fd` would return **as if `--max-results` were removed** (all other args identical).
- `printed`: number of rows actually printed (≤ `LLM_X_MAX_FD_ROWS`, and ≤ `returned` when present).
- `omitted = returned - printed` (or `total - printed` when `returned` is absent): wrapper-only truncation due to `LLM_X_MAX_FD_ROWS`.

When `--max-results` is present and successfully parsed, the meta line also includes:
- `max_results`: the user-provided `--max-results N`.
- `returned`: number of paths actually returned by `fd` under the user cap.
- `unseen = total - returned`: paths excluded only due to the user cap.

### Notes
- File metadata (`bytes=...`, `lines=...`) is computed **only for printed rows**.
- Do not rely on output order (implementations may preserve tool order or sort).

---

## `rg-x` file-list modes: file table cap + totals

When `rg-x` detects a file-list mode (e.g. `-l`, `--files-with-matches`, `--files`), it prints a file table just like `fd-x`.

### What gets capped
- Printed rows are capped to `LLM_X_MAX_FD_ROWS` (same as `fd-x`), even though the tool is `rg-x`.

### Meta meaning
Trailer line:
```
@meta	tool=rg-x	mode=filelist	total=T	printed=P	omitted=O
```
If there are **0 rows** (no file paths), `rg-x` prints **nothing** (no `@meta`) and returns the canonical `rg` exit code (typically `1` for “no matches”).

---

## `rg-x` match mode: file cap + per-file match-line cap

If `rg-x` is not in passthrough mode and not in file-list mode, it runs `rg` in a structured mode and groups output by file.

If there are **0 matches**, `rg-x` prints **nothing** (no `@meta`) and returns the canonical `rg` exit code (`1`).

### No-omit for small results
If total match lines across all matching files is **≤ `LLM_X_MAX_RG_NO_OMIT_MATCH_LINES`**, `rg-x` prints:
- all matching file groups
- all match lines in those files

(This still applies long-line gating to avoid pathological token blowups.)

### What gets capped / omitted
If the total match lines exceed `LLM_X_MAX_RG_NO_OMIT_MATCH_LINES`, `rg-x` switches to capped output:

1) **Per-file match line cap**
- For each file group, only the first `LLM_X_MAX_RG_MATCH_LINES_PER_FILE` match lines are printed.
- Additional match lines in the same file are **omitted** and counted.

2) **File-group cap**
- Only the first `LLM_X_MAX_RG_FILES` matching files are printed as groups.
- Entire additional file groups are **omitted** (no `@file` header, no match lines), but still counted in the final totals.

### Per-file header fields
Each printed file group starts with an `@file` line:
```
@file	path=...	bytes=...	lines=...
```
If per-file match lines were omitted due to `LLM_X_MAX_RG_MATCH_LINES_PER_FILE`, the header also includes:
```
match_lines=M	shown=S	omitted=K
```
- `shown`: printed match lines for that file (≤ `LLM_X_MAX_RG_MATCH_LINES_PER_FILE`)
- `omitted`: match lines in that file not printed due to the per-file cap
- `match_lines = shown + omitted`

### Trailer meta fields
All match-mode output ends with:
```
@meta	tool=rg-x	mode=match	files=F	printed_files=PF	omitted_files=OF	match_lines=ML	printed_match_lines=PML	omitted_match_lines=OML
```
Interpretation:
- `files`: number of files with at least one match.
- `printed_files`: number of file groups printed (≤ `LLM_X_MAX_RG_FILES`).
- `omitted_files = files - printed_files`.
- `match_lines`: total match lines across **all** matching files.
- `printed_match_lines`: match lines actually printed (bounded by both caps).
- `omitted_match_lines = match_lines - printed_match_lines`.

### Important implications
- Even when file groups are omitted due to `LLM_X_MAX_RG_FILES`, their matches still contribute to:
  - `files`
  - `match_lines`
  - `omitted_files`
  - `omitted_match_lines`
- File byte/LoC metadata is collected **only for printed file groups**.

---

## `sed-x`: range reads + gating + stdin totals (bounded)

`sed-x` only intercepts the narrow ranged-read idiom:
```
sed -n 'a,bp' [file|-]
```
Other `sed` invocations passthrough.

### File input
Prints the selected range, then:
```
@meta	tool=sed-x	path=...	bytes=...	lines=...	range=a..b	[ truncated_lines=T ]
```

### Stdin input
Prints the selected range, then:
```
@meta	tool=sed-x	source=stdin	range=a..b	bytes=...	lines=...	complete=0|1	[ reason=cap|tty ]	[ truncated_lines=T ]
```
- `complete=1`: stdin was fully consumed to EOF; `lines/bytes` are total stdin.
- `complete=0 reason=cap`: stdin was scanned only up to a safety cap; `lines/bytes` reflect what was scanned.
- `complete=0 reason=tty`: stdin was a terminal; `sed-x` does not attempt to drain it.

### Why stdin totals may require extra reads
If you use `sed-x` in a pipeline like:
```
nl -ba file | sed-x -n '1,30p'
```
`sed-x` must keep reading stdin after line 30 to know the total number of stdin lines. This is bounded by `LLM_X_SEDX_STDIN_MAX_LINES` / `LLM_X_SEDX_STDIN_MAX_BYTES`.
