## Key facts to anchor the wrapper design

* **`fd` output is inherently ambiguous if you parse by newline** (paths *can* contain newlines), so a robust wrapper should prefer **NUL-delimited** output (e.g., `fd --print0`) whenever it intends to parse paths. ([Arch Manual Pages][1])
* **`rg` has a structured JSON Lines event stream** via `rg --json` (records include `type` + `data`, and `data.submatches[]` includes `start`/`end` offsets). ([ripgrepy.readthedocs.io][2])
* **`rg --json` cannot be combined with “file-list” output modes** like `--files-with-matches` / `-l` (and similar flags); therefore `rg-x` needs at least two operational modes. ([ripgrepy.readthedocs.io][2])
* `rg --json` emits a **final stats/summary record** implicitly, which your parser must accept. ([GitHub][3])

These constraints largely determine how to implement “thin post-process wrappers” without brittle stream parsing.

---

## Packaging recommendation: one multicall executable with symlinks

To meet “drop-in / low overhead” requirements, ship **one** binary (Rust/Go are the obvious choices) that behaves like BusyBox:

* Install a single executable, e.g. `tool-x`
* Create symlinks: `fd-x`, `rg-x`, `sed-x` → `tool-x`
* Dispatch behavior by `argv[0]` (`fd-x` mode, etc.)

This yields:

* **One artifact** to distribute
* Consistent flags (`--raw`, `--limits`, `--format`) across wrappers
* Easy “passthrough fallback” (exec underlying tool when wrapper can’t/shouldn’t post-process)

---

## Common building block: fast deterministic “file facts”

All three wrappers benefit from the same metadata function:

### `file_facts(path)` (deterministic, bounded work)

Return:

* `bytes`: file size from `stat`
* `loc`: newline count, **capped scan** (e.g., stop counting after N MB unless explicitly requested)
* `max_line_bytes`: maximum line length observed during scan (bounded)
* `is_binary`: true if NUL byte appears in sampled windows
* `sample_kind`: `{text, jsonl_like, unknown}` via lightweight heuristics

**Important:** this avoids the “`wc -l` says small but lines are huge” failure mode by directly tracking **max line length** and/or “columns” (byte length) during scan.

Suggested defaults (tunable):

* Scan budget: `min(bytes, 2 MiB head + 2 MiB tail)` for “max line” heuristics on large files
* LOC budget: count newlines in the same scan window unless `--exact-loc` is set

---

## Output format: path-first, low-footprint, machine-friendly

To “mimic” canonical tools while still being parseable and compact:

* Keep the **canonical prefix** first (path or `path:line:col:`)
* Append metadata as **TSV “facts”** (tab is rarer than spaces in normal output)
* Emit wrapper-only meta lines with a reserved prefix (e.g., `@meta\t...`) so downstream tooling (or an LLM) can reliably ignore them.

Example conventions:

* **Record line:** `<canonical>\tbytes=<n>\tloc=<n>\tmax=<n>`
* **Wrapper meta line:** `@meta\tkey=value\tkey=value`

This stays minimal, avoids verbose JSON by default, but remains structured enough for agents.

(Optionally support `--format jsonl` for callers that want strict encoding for weird filenames.)

---

# `fd-x` spec

### Purpose

* Provide a **layout + file size/LoC/max-line** overlay to prevent:

  * blind root searching without preflight
  * multi-call churn on small files
  * polyglot misclassification

### Behavior (best-effort, passthrough fallback)

1. **Detect passthrough conditions**:

   * If args include `--exec` / `--exec-batch` (or other modes where stdout is not a simple path list), **passthrough** to `fd` verbatim. ([GitHub][4])
2. Otherwise:

   * Invoke underlying `fd` with user args, but ensure parseable output:

     * If user did *not* specify `--print0`/`-0`, `fd-x` may internally add it for parsing and then re-emit path text safely (or switch to `--format jsonl` automatically on unprintable paths).
   * For each found entry, compute `file_facts(path)` and emit:

**Example output (default text/TSV):**

```
./src/lib.rs	bytes=13240	loc=412	max=121
./scripts/util.py	bytes=4210	loc=88	max=102
@meta	files=30	dirs=6	total_bytes=...	total_loc≈...
```

### Helpful optional flags

* `--overview[=DEPTH]`: additionally emit top-level directory rollups + extension histogram, e.g.:

  * `@meta\text=.rs:120 .py:8 .md:14 ...`
  * `@meta\tdir=src files=... bytes=... loc≈...`
* `--limit N`: cap entries printed; still print totals (avoid flooding)
* `--raw`: emit underlying `fd` output only

---

# `rg-x` spec

### Purpose

* Prevent “grep floods the context window”
* Add **file facts** to results
* Provide structured matches via `--json` where possible

### Mode selection (must be explicit in the implementation)

Because `rg --json` is incompatible with file-list-only modes, `rg-x` should do:

#### A) “file list mode” (passthrough-ish)

If the user requested any of:

* `-l` / `--files-with-matches`
* `--files-without-match`
* `--files`
* other modes that output only file paths

Then:

* Execute `rg` as requested (no `--json`)
* Parse output as a path stream (prefer `--null` if available; otherwise best-effort newline parsing)
* Append file facts per path

This respects the user’s intent and the `--json` constraint. ([ripgrepy.readthedocs.io][2])

#### B) “match mode” (structured)

Otherwise:

* Execute `rg --json ...` (inject `--json`)
* Parse JSON Lines events (`type=match`, plus the implicit final `type=summary/stats` record)
* Emit deterministic vimgrep-like lines (easy for agents), plus file facts:

**Example output:**

```
./src/lib.rs:120:9:	fn parse(...)	bytes=13240	loc=412	max=121
./src/lib.rs:201:3:	parse_error(...)	bytes=13240	loc=412	max=121
@meta	files_with_matches=1	total_matches=2
@meta	stats=...   # derived from rg’s final summary record
```

JSON events have stable `submatches[].start/end` offsets if you later want to add `match_span=` without re-parsing text. ([ripgrepy.readthedocs.io][2])

### Anti-flood defaults (agent-oriented, but overridable)

* `--max-files N` (default e.g. 200)
* `--max-matches-per-file K` (default e.g. 50)
* `--max-total-matches M` (default e.g. 500)
* When truncating, include:

  * `@meta\ttruncated=true\treason=...`
  * and per-file elisions like `@meta\tfile=... elided_matches=123`

### Passthrough fallback

If `rg` exits non-zero or `--json` parsing fails for any reason:

* Print a one-line `@meta` warning
* Re-run in raw mode (or directly passthrough) and return the underlying exit code

This satisfies “never fail” in practice.

---

# `sed-x` spec (ranged-read gating)

### Scope

Only intercept:

* `sed -n 'START,ENDp' FILE`
* (and optionally the equivalent with `-e`)

Everything else: **passthrough** to system `sed`.

### Why you should not “post-process sed stdout” for gating

If the line is huge, letting `sed` print it means the wrapper already lost—your context window is burned. So for the gated mode, implement the ranged read **in the wrapper** (streaming) and only then emit output.

### Ranged mode algorithm

1. Parse `START,ENDp` and `FILE`
2. Stream file line-by-line (no full-file read):

   * Skip until `START`
   * For each line within range:

     * If `len(line_bytes) <= MAX_LINE_BYTES`, print as-is
     * Else replace with:

       * `HEAD` + `…TRUNCATED(len=<n>)…` + `TAIL`
       * Optional lightweight hint: if it looks like JSON, extract a few keys from the **head window** only
3. Append file facts footer regardless of truncation:

**Example output:**

```
120: {"event":"...very long...   …TRUNCATED(len=84231)…   ..."}
121: ok
@meta	file=server.log	bytes=998221	loc=20	max≈84231	truncated_lines=1
```

### Suggested defaults

* `MAX_LINE_BYTES=2048`
* `HEAD_BYTES=512`, `TAIL_BYTES=256`
* Optional: `--max-output-bytes` to cap total emitted bytes

---

## Integration recipe for `AGENTS.md` (minimal + effective)

See the bundled `AGENTS_addon.md` for an example instruction section.

---

## Summary: how this avoids your listed failure modes

* **No preflight context** → `fd-x --overview` gives immediate repo shape, file counts, dominant extensions, and candidate roots.
* **Churn reading small files** → `fd-x` facts make “one-shot read vs chunked” an explicit decision.
* **Polyglot wrong-language jump** → extension histogram + dir rollups make “Rust core + Python utilities” obvious.
* **Accidental big-line dumps** → `sed-x` gates by *line byte length*, not `wc -l`, and never prints a pathological line unmodified.

[1]: https://man.archlinux.org/man/fd.1.en?utm_source=chatgpt.com "fd(1) - Arch manual pages"
[2]: https://ripgrepy.readthedocs.io/?utm_source=chatgpt.com "Welcome to ripgrepy's documentation! — ripgrepy ..."
[3]: https://github.com/BurntSushi/ripgrep/issues/2337?utm_source=chatgpt.com "the `--stats` flag should note that it is unconditionally ..."
[4]: https://github.com/sharkdp/fd/issues/1797?utm_source=chatgpt.com "[BUG] --print0 and --exec does not include null byte #1797"

---

Check out the bundled `llm-inspect-wrappers-rs`, a concrete **reference Rust repo skeleton** (`tool-x`) implementing the multicall wrapper shape (`fd-x`, `rg-x`, `sed-x`), including:

* Core modules:

  * `src/fdx.rs`: `fd-x` wrapper (prefers `fd --print0`, appends file facts)
  * `src/rgx.rs`: `rg-x` wrapper (file-list mode passthrough-ish; match mode uses `rg --json`)
  * `src/sedx.rs`: `sed-x` wrapper (intercepts `sed -n 'A,Bp' FILE`, performs gated ranged read in-wrapper)
  * `src/facts.rs`: bounded scan for `bytes`, `loc≈`, `max_line≈`, `bin`
  * `src/common.rs`: tool resolution + capture/passthrough helpers
* Unit tests embedded in `rgx.rs` and `sedx.rs`
* `scripts/install.sh` to build + install binary + symlinks
* Minimal README + MIT license

### Spot-verification performed (in this environment)

* Verified repo structure and module wiring (all `mod ...;` targets exist).
* Parsed `Cargo.toml` successfully (syntactically valid TOML; dependencies present).
* **Note:** this runtime environment does **not** have a Rust toolchain (`cargo`/`rustc` unavailable), so I could not execute `cargo test` / `cargo build` here. The skeleton is set up so you can run those checks immediately in your local dev environment.

### Local integrity check (recommended)

```bash
cd llm-inspect-wrappers-rs
cargo test
cargo build --release
./scripts/install.sh ~/.local/bin
```
