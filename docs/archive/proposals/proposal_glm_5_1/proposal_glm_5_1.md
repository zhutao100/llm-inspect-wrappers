Good. Now I have the key technical facts confirmed. Let me synthesize the full design and implementation.

---

# `xwrap` — Thin Post-Process Wrappers for LLM-Agentic File Inspection

## Preliminary: Key Technical Confirmations

**`fd` output parsing:** The `-0` option tells `fd` to separate search results by the NULL character (instead of newlines). This is critical — filenames can contain newlines (rare in practice, but must be handled). We'll use `fd ... -0` internally for safe parsing and strip it before forwarding args to the real `fd`.

**`rg` structured output:** When the `--json` flag is provided, ripgrep will emit a sequence of messages, each encoded as a JSON object, where there are five different message types: `begin` (a file is being searched and contains at least one match), `end` (a file is done being searched), `match` (a match was found, including text and offsets), `context` (a contextual line was found), and `summary` (final summary statistics across all files). However, the JSON Lines format is only supported for showing search results. It cannot be used with other flags that emit other types of output, such as `--files`, `--files-with-matches`, `--files-without-match`, `--count` or `--count-matches`. This is important: for `rg -l` style invocations, we must parse the standard line-based output, not JSON.

**Design decision:** Since `rg --json` cannot be combined with `-l`/`-c`/`--files`, and the default `rg` text output (`--no-heading` with `path:line:col:text`) is well-structured enough for post-processing, we should **not** force `--json` injection. Instead, parse the canonical text output, which preserves tool behavior conventions. For match-mode output, we parse the well-known `path:linenum:text` format (with `--no-heading` forced for pipe contexts). For `-l`/`-c` modes, output is already one-path-per-line.

---

## Architecture

```
┌──────────────────────────────────────────────────────────┐
│  Single Bash script: xwrap                               │
│  Symlinked as: fd-x, rg-x, sed-x                        │
│  Dispatches by $0 basename (argv[0])                     │
│                                                          │
│  fd-x:  fd $@ -> collect paths -> annotate size/LoC     │
│  rg-x:  rg $@ -> collect paths -> annotate size/LoC     │
│         + gate long match lines                          │
│  sed-x: detect ranged-read -> gate huge lines            │
│         + append file size/LoC trailer                   │
└──────────────────────────────────────────────────────────┘
```

**Single file, multi-personality via `$0`.**  You can use a shell alias or a wrapper script in order to pass desired flags to fd by default. Same principle applies here — drop the script as `xwrap`, symlink `fd-x -> xwrap`, `rg-x -> xwrap`, `sed-x -> xwrap`.

### Why Bash, not a compiled binary?

- macOS and Linux both ship `/bin/bash` (macOS: bash 3.2+, but we target POSIX-compatible subset + a few bashisms that work on 3.2).
- Zero dependency install. No Python startup overhead (~30-80ms), no compilation step.
- The wrappers are I/O-bound (running `fd`/`rg`/`sed` + `wc`), not compute-bound. Bash overhead is negligible relative to subprocess I/O.
- Trade-off: if the project scales to need JSON parsing (e.g. consuming `rg --json`), a Python rewrite would be warranted. For now, text parsing suffices.

---

## Implementation

See the bundled `xwrap.sh`

---

## Design Notes & Rationale

### `fd-x`

The wrapper runs `fd` with user-supplied arguments verbatim, then appends a `--- file info ---` trailer block with `[bytes, lines]` per file.

**Filename parsing safety:** In production projects, filenames with newlines are essentially nonexistent. One user encountered a special file containing `\n` in the filename and it makes it difficult to parse the output of `rg --files`. For the LLM-agent use case (inspecting code repos), newline-in-filename is a non-concern. We parse line-delimited output, which is what the agent sees anyway. The `-0` null-separator option is reserved for future hardening if needed.

**Why not use `fd -x wc -cl`?** The `-x/--exec` option runs an external command for each of the search results (in parallel). This would intermix `wc` output with filenames in an unpredictable order due to parallelism, making it harder to post-process. Running `wc` in our loop after collecting paths gives deterministic output.

### `rg-x`

Key subtlety: we force `--no-heading --line-number` to normalize output format for parsing. You can switch to the standard grep output format with `--no-heading`. This ensures every match line starts with `path:linenum:content`, which is trivially parseable and also the convention LLMs already understand.

**Why not use `rg --json`?** The JSON Lines format is only supported for showing search results. It cannot be used with other flags that emit other types of output, such as `--files`, `-l/--files-with-matches`, `--files-without-match`, `-c/--count` or `--count-matches`. ripgrep will report an error if any of the aforementioned flags are used in concert with `--json`. Since we need to be a universal passthrough wrapper, we can't inject `--json` without knowing which flags the agent already passed. The text format is sufficient for our annotation needs.

**Huge line gating in rg output:** This catches the case where `rg` matches inside a minified JS file, a giant JSON log line, etc. The match-line content is gated while preserving the `path:linenum:` prefix so the LLM still knows *where* the match is.

### `sed-x`

We only intercept the `sed -n 'START,ENDp' file` pattern (the ranged-read idiom agents use). Everything else passes through via `exec sed "$@"` (zero overhead — replaces the wrapper process entirely).

**Gating rationale:** The `_gate_line` function provides a best-effort content hint: for JSON blobs, it extracts top-level key names; for XML/HTML, it notes the type; for base64, it flags it. The head+tail peek gives the LLM enough to decide whether to request the full line via a more targeted extraction.

**File info trailer:** Always appended so the agent knows the total file dimensions — this directly addresses the "300-line file read in 3 chunks" failure mode.

### Gating heuristic (`_gate_line`)

```
Original:  {"user_id":"abc","payload":"{...10KB of nested JSON...}","ts":1234}
Gated:     {"user_id":"abc","payload":"  ...⟪TRUNCATED 10234 chars, json object, keys: [user_id,payload,ts]⟫...  ,"ts":1234}
```

The `⟪TRUNCATED⟫` marker is deliberately non-ASCII to be unambiguous in LLM context. The char-count gives the LLM a signal about line magnitude. The key-list hint lets the LLM decide if the line is worth full extraction.

---

## Installation

```bash
# Drop the script
curl -o /usr/local/bin/xwrap https://raw.githubusercontent.com/...  # or copy manually
chmod +x /usr/local/bin/xwrap

# Create symlinks
ln -sf /usr/local/bin/xwrap /usr/local/bin/fd-x
ln -sf /usr/local/bin/xwrap /usr/local/bin/rg-x
ln -sf /usr/local/bin/xwrap /usr/local/bin/sed-x
```

Verify: `fd-x --version` should print `fd`'s version (passthrough).

---

## AGENTS.md Integration Recipe

Add the instruction section in the bundled `AGENTS_addon.md` into `~/.codex/AGENTS.md`:

This is the minimal, low-token prompt injection. The key insight is that the metadata trailer changes the LLM's *information landscape* before it plans the next tool call — it now has file dimensions in context without needing a separate `wc` call.

---

## Edge Cases & Failure Modes

| Scenario | Behavior |
|---|---|
| `fd` or `rg` not installed | The `fd` / `rg` subprocess fails; wrapper captures error and prints it through. Agent sees the error, same as without wrapper. |
| Binary file in results | `wc -c -l` still works on binary files (LoC will be low/zero, byte count correct). Annotation is still useful. |
| `rg` with `--json` flag passed by agent | `--no-heading` is a no-op when `--json` is active. The wrapper's line parsing will see JSON lines, gating will apply per-line if huge. Not ideal but non-destructive. |
| Very large result set (10K+ matches) | `MAX_ANNOTATE` cap (default 500) limits the annotation trailer. The gated match lines add minimal overhead per line. |
| `sed-x` called with non-range invocation | `exec sed "$@"` — zero overhead passthrough. |
| Any exception in bash | `set -o pipefail` ensures we surface real errors. If the annotation logic itself fails, the original tool output was already printed. |

---

## Performance Characteristics

**`fd-x`:** Cost = `fd` runtime + one `wc -cl < file` per result file. `wc` on a local file is a single `fstat` + sequential read, typically < 1ms per file. For 500 files, annotation adds ~0.5s (mostly process fork overhead in bash; could be reduced to ~50ms with `xargs -P` batching — easy follow-up optimization).

**`rg-x`:** Cost = `rg` runtime + line-length checks (pure bash string length, O(1)) + `wc` per unique matched file. Since `rg` typically matches a subset of project files, annotation cost is small.

**`sed-x`:** Cost = `sed` runtime + one `wc` call per target file + per-line length check. Negligible.

### Follow-up Optimization: Batch `wc`

For `fd-x` with many files, replace the per-file `wc` loop with:

```bash
printf '%s\0' "${file_paths[@]}" | xargs -0 wc -cl 2>/dev/null
```

This runs a single `wc` process for all files. The output format is `lines bytes filename` per line, easily parsed. This reduces fork overhead from O(n) to O(1). Not included in v1 for clarity, but trivial to add.

---

## Possible Extensions (Not in Scope for v1)

1. **`tree-x`**: A dedicated project-overview command that runs `fd -t f | head -N` and annotates everything. Could be triggered as the *first* command in an agent session. Directly solves the "preflight overview" failure mode.

2. **Content-type sniffing in `sed-x`**: Use `file --mime-type` to detect binary/encoded files before `sed` reads them. Low-cost guard against dumping binary blobs.

3. **Token-budget estimation**: Given a known model's tokenizer, estimate the token count of the output and warn if it would consume >X% of a context window. Requires shipping a tokenizer lookup table — probably overkill for a bash tool.
