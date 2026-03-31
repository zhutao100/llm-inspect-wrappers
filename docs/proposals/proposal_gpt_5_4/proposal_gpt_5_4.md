Short version: build **one small wrapper executable** with three symlink entrypoints: `fd-x`, `rg-x`, `sed-x`.

The important design choice is: **don’t parse human-oriented text if the underlying tool has a safer machine channel**.

- For `fd-x`, internally force `fd -0` and parse NUL-delimited paths. `fd`’s default output is newline-delimited, so raw line parsing is not filename-safe; `-0/--print0` exists specifically for that. Also, `fd` may prepend `./` to relative paths when `-0` is used, so strip that back out when re-rendering if you want canonical-looking output.
- For `rg-x`, use `rg --json` for normal match-mode. ripgrep’s JSON Lines stream gives you `begin`, `end`, `match`, `context`, and `summary` messages, and it explicitly models file/text payloads as either UTF-8 `text` or base64 `bytes`. But `--json` is not available with `--files`, `-l/--files-with-matches`, `--files-without-match`, or count modes, so for those modes use `-0/--null` instead.

## 1. Overall recommendation

I would make these wrappers **LLM-first inspection tools**, not general-purpose shell-compatible supersets.

That means:

1. **Intercept only the safe/common cases.**
2. **Render a terse, token-efficient text format.**
3. **Add deterministic summary/capping behavior for large outputs.**
4. **Fall back to the real tool unchanged for everything else.**

That gets you the best tradeoff between:
- low implementation complexity,
- correctness on weird paths,
- much better LLM context efficiency,
- and “never fail / passthrough fallback”.

## 2. One executable, three entrypoints

Use one script/binary, dispatched by `argv[0]`:

- `fd-x`
- `rg-x`
- `sed-x`

That keeps install overhead tiny:

```bash
ln -s /usr/local/bin/llm-inspect /usr/local/bin/fd-x
ln -s /usr/local/bin/llm-inspect /usr/local/bin/rg-x
ln -s /usr/local/bin/llm-inspect /usr/local/bin/sed-x
```

### Language choice

For a first version, I’d use **Python 3**.

Why:
- NUL-safe path handling is easy.
- JSONL parsing for `rg --json` is trivial.
- You can preserve odd filenames via `surrogateescape`.
- Single-file deploy is easy.

If you later want zero-runtime-dependency packaging, rewrite the same design in Rust.

## 3. Common contract

All three wrappers should follow the same policy.

### 3.1 Supported behavior
- Best-effort enhancement for the known/simple modes.
- Same stdout/stderr exit behavior as the real tool where feasible.
- Stable, terse text output for LLM consumption.

### 3.2 Fallback behavior
If any of these happens:
- unsupported flags,
- parser uncertainty,
- subprocess failure,
- post-processing exception,

then:
- run the canonical tool,
- pass through raw stdout/stderr,
- preserve exit code.

### 3.3 “Never fail” implementation detail
Do **not** stream transformed output directly.

Instead:
1. run inner tool,
2. capture stdout into a spooled temp buffer,
3. parse and transform,
4. if transform fails, replay raw captured stdout.

That avoids mixed partial output.

## 4. Output format

Keep it compact and regular.

### 4.1 File metadata atoms
Use only:
- `B` = bytes
- `L` = lines
- `H` = hits

### 4.2 `fd-x` line format

```text
path<TAB>1234B<TAB>56L
```

For non-regular files:
- directory: `path/<TAB>-<TAB>-`
- symlink: `path@<TAB>-<TAB>-`

### 4.3 `rg-x` grouped format

Render once per file:

```text
path<TAB>1234B<TAB>56L<TAB>3H
12:matching line text
48:another hit
# +1 more in path
```

This is much more token-efficient than repeating `path:` on every match line.

### 4.4 Footer summary

For both `fd-x` and `rg-x`, append a tiny footer:

```text
# summary 31F 4D ext:rs=18,py=4,toml=2 roots:src=21,tests=5,scripts=3
```

or

```text
# summary 8F 14H ext:rs=7,py=1 roots:src=6,scripts=1,tests=1
```

This solves the polyglot failure mode without dumb fields like `language: Rust`.

## 5. `fd-x`

## 5.1 What to intercept

Intercept only **plain listing modes**.

Good candidates:
- ordinary `fd PATTERN [PATH...]`
- type/depth/ignore filters
- hidden/follow/no-ignore variants

Pass through unchanged for:
- `-x`, `-X`, `--exec*`
- `--format`
- anything that changes output into something other than a path stream
- quiet/probe-like invocations where output isn’t expected

## 5.2 Internal execution plan

Internally run:

```bash
fd -0 ...
```

Parse NUL-delimited paths, then lookup metadata.

That is the right answer to your note: **yes, `fd` path parsing matters**; raw newline parsing is not robust, so the wrapper should switch the inner call to `-0`.

## 5.3 Metadata collection

Use batched metadata lookup for regular files:
- `wc -lc` in batches for “normal” names
- fallback path for pathological names containing newlines

Practical detail:
- keep original result order
- batch by argv byte budget, not by file count
- set `LC_ALL=C`

### Newline-in-filename corner case
This is the one place where `wc` becomes annoying, because its own output is line-oriented.

So I’d do:
- fast path: batch `wc -lc` for files without `\n`
- rare fallback: one-file-at-a-time or Python-native counting for files with `\n`

That keeps correctness without slowing the normal case.

## 5.4 Size-aware output control

To avoid flooding context:

- if results `<= 200`: print all
- else:
  - print summary first
  - print first 120 paths
  - append `# omitted N more`

That is an intentional semantic deviation, but it directly addresses the “repo-root grep/fd flood” failure mode.

## 6. `rg-x`

## 6.1 Two modes

### Mode A: file-list modes
Use when args contain:
- `--files`
- `-l` / `--files-with-matches`
- `-L` / `--files-without-match`

Internally use:

```bash
rg -0 ...
```

Then parse NUL-delimited file paths and render like `fd-x`.

### Mode B: match mode
Use `rg --json` and parse the event stream.

This is the key win: **yes, `rg` has a structured event stream, and you should absolutely use it**. It gives you exact file boundaries, match lines, context lines, and final summary stats.

## 6.2 What to pass through

Pass through unchanged for modes where JSON is incompatible or where preserving exact rg semantics is more important than prettifying:

- `--json` already explicitly requested
- `-c`, `--count`, `--count-matches`
- `-o`, `--only-matching`
- `-r`, `--replace`
- `--passthru`
- `--vimgrep` if you want exact editor-compatible format

## 6.3 Match-mode rendering

Accumulate by file:

- metadata from path -> `B/L`
- hits count from parsed match events
- match lines
- context lines if present

Suggested rendering:

```text
src/pipeline/mod.rs	21432B	612L	4H
18:use crate::pipeline::Builder;
112:fn build_pipeline(cfg: &Config) -> Result<Pipeline> {
271:let builder = PipelineBuilder::new();
# +1 more in src/pipeline/mod.rs

scripts/debug.py	3120B	92L	1H
44:def build_pipeline(cfg):
# summary 2F 5H ext:rs=1,py=1 roots:src=1,scripts=1
```

### Why grouped output is better
This preserves rg’s core semantics while avoiding the expensive repetition of the path on every line.

## 6.4 Large result handling

For huge searches:
- cap displayed files, e.g. `80`
- cap match lines per file, e.g. `4`
- still report full counts in footer

Example:

```text
# summary 482F 31240H ext:rs=410,py=38,ts=22
# showing first 80 files, first 4 matches per file
...
```

This is the biggest practical improvement over raw `rg` for LLM agents.

## 6.5 Optional extra: reuse line gating here too

I would also apply the `sed-x` suspicious-line truncator to `rg-x` match/context lines.

Because otherwise a single minified JSON line from a log or fixture can still wreck the context window.

## 7. `sed-x`

## 7.1 Scope
Only intercept:

```bash
sed -n 'START,ENDp' FILE
```

and a tiny set of equivalent spellings.

Everything else: passthrough.

That keeps BSD/GNU compatibility simple.

## 7.2 Implementation choice
I would **not** shell out to `sed` for this mode.

For plain numeric ranges, read the file directly and emit lines `START..END`. It’s simpler than trying to parse `sed` quoting edge cases after the fact, and numeric range semantics are trivial.

## 7.3 Suspicious-line gating

For each selected line, if any of these holds:
- line length > soft threshold, e.g. `400`
- line length > hard threshold, e.g. `2000`
- JSON-looking and long
- base64/blob-looking
- punctuation density very high / whitespace very low
- non-UTF8 bytes

replace with a compact descriptor:

```text
[sed-x truncated line=14 len=18291 kind=json keys=ts,level,msg,request_id head='{"ts":"2026-03-30T12:01:00Z","level":"info",' tail='"status":200}']
```

Otherwise emit the original line unchanged.

## 7.4 Footer

Always append:

```text
# meta server.log 183902B 20L range=1..20 truncated=1
```

That gives the agent the crucial “can I one-shot read this file?” signal.

## 8. Heuristics I’d actually use

Defaults:

```text
MAX_FD_ROWS=200
MAX_RG_FILES=80
MAX_RG_MATCHES_PER_FILE=4
SOFT_LINE_CHARS=400
HARD_LINE_CHARS=2000
HEAD_CHARS=160
TAIL_CHARS=80
TOP_EXT=6
TOP_ROOTS=6
```

And make them env-tunable:
- `LLM_X_MAX_FD_ROWS`
- `LLM_X_MAX_RG_FILES`
- `LLM_X_MAX_MATCHES_PER_FILE`
- `LLM_X_SOFT_LINE_CHARS`
- etc.

## 9. Why this addresses your listed failure modes

### 9.1 Blind repo-root grep flood
`rg-x` and `fd-x` both produce:
- compact summaries,
- ext/root histograms,
- deterministic caps.

So a broad search becomes “overview first” instead of raw spam.

### 9.2 Repeated small-file partial reads
`fd-x`/`rg-x` surface `L` on the candidate file immediately.

That enables the agent to decide:
- `<= ~400L`: read whole file once
- else: use `sed-x` range reads

### 9.3 Wrong-language jump in polyglot repos
The footer’s `ext:` and `roots:` distribution gives a cheap structural prior:
- `rs=120 under src/, py=8 under scripts/`
- therefore likely core path is Rust, Python is auxiliary

### 9.4 Accidental huge dump from short-LoC file
`sed-x` line gating handles exactly this case:
- short file by LoC
- giant per-line payload
- output becomes summarized instead of dumped

## 10. Integration recipe for `AGENTS.md`

This is the right low-overhead integration point for Codex-style agents. OpenAI explicitly recommends maintaining `AGENTS.md` for persistent repo context, says Codex can be guided by it, and their published system message scopes `AGENTS.md` files to directory subtrees with deeper files taking precedence.

See the bundled `AGENTS_addon.md` for an example instruction section.

## 11. Bottom line

If I had to make this real, I would ship:

- **one Python executable**
- **argv[0]-based dispatch**
- **safe interception only**
- **NUL parsing for `fd` and file-list `rg`**
- **JSONL parsing for match-mode `rg`**
- **direct numeric-range reader for `sed-x`**
- **deterministic capping + terse summaries**
- **hard passthrough fallback**

That’s thin enough to be deployable today, but opinionated enough to materially improve agent inspection behavior.

---

The bundled `llm_inspect.py` is a **single-file reference Python skeleton** for the design discussed: one executable, dispatched by `argv[0]`, with symlink entrypoints `fd-x`, `rg-x`, and `sed-x`.

It is intentionally:
- **conservative** in what it intercepts,
- **best-effort** in post-processing,
- **passthrough** on unsupported/uncertain cases,
- and structured to be easy to harden.

## What this skeleton already does

- **`fd-x`**
  - intercepts simple list-style `fd` usage,
  - internally forces `fd -0`,
  - parses NUL-delimited paths,
  - appends `bytes` + `line count`,
  - caps oversized result sets,
  - adds compact `ext:` / `roots:` summary.

- **`rg-x`**
  - **file-list modes**: uses `-0`, parses file paths, renders like `fd-x`.
  - **match mode**: uses `--json`, groups by file, counts hits, shows a few lines per file, adds metadata and summary.
  - gates suspicious long/minified/json/base64-ish lines.

- **`sed-x`**
  - only intercepts `sed -n 'start,endp' file`-style range reads,
  - reads directly in Python,
  - replaces suspicious giant lines with a compact marker,
  - always appends `# meta ... bytes lines range truncated`.

- **fallback policy**
  - unsupported or uncertain cases go to the real tool unchanged.

## First hardening steps I’d recommend

1. **Tighten arg parsing**
   - especially bundled short flags and more `sed` spellings.

2. **Add integration tests**
   - paths with spaces,
   - paths beginning with `-`,
   - no-match `rg`,
   - polyglot repo summaries,
   - huge single-line logs.

3. **Promote some conservative cases from passthrough to supported**
   - e.g. richer `sed -e` combinations.

4. **Switch capture to spooled temp files**
   - if you want stronger “never fail” behavior on huge raw outputs.

5. **Optionally add JSON output mode**
   - useful for future MCP or structured consumers, while defaulting to terse text.

If you want, I can next generate:
1. a **small test suite** for this skeleton, or
2. a **production-hardened v2** with stricter arg parsing and better `wc` batching.
