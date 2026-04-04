## Evaluation scope (criteria)

I evaluated each proposal against the task’s explicit requirements: **drop-in wrappers** for `fd`, `rg`, and ranged `sed`, that are **best-effort**, **passthrough on unsupported/exceptional cases**, add **file size + LoC metadata**, and **gate pathological long lines** for ranged reads (and ideally for `rg` match output as well).

---

## What’s in the box: the artifacts you get with each proposal

You effectively have three distinct implementation directions:

1. **GLM 5.1**: `docs/archive/proposals/proposal_glm_5_1/`

   * `xwrap.sh` (single Bash multicall script; symlinked as `fd-x`, `rg-x`, `sed-x`)
   * `AGENTS_addon.md` + design markdown

2. **GPT-5.4**: `docs/archive/proposals/proposal_gpt_5_4/`

   * `llm_inspect.py` (single Python multicall script; symlinked as `fd-x`, `rg-x`, `sed-x`)
   * `AGENTS_addon.md` + design markdown

3. **GPT-5.2**: `docs/archive/proposals/proposal_gpt_5_2/`

   * `llm-inspect-wrappers-rs/` (Rust multicall binary; symlinked as `fd-x`, `rg-x`, `sed-x`)
   * `AGENTS_addon.md` + design markdown

---

## Comparative scorecard (practical, not aspirational)

| Dimension                                           |                                 Opus (Bash) |       GPT-5.4 (Python) |                       GPT-5.2 (Rust) |
| --------------------------------------------------- | ------------------------------------------: | ---------------------: | -----------------------------------: |
| **Drop-in friendliness** (minimal ceremony)         |                                        High |            Medium–High |                               Medium |
| **Filename-safety** (NUL/newline/odd paths)         |                                         Low |                   High | High (fd/sed), Medium (rg file-list) |
| **Preserves stdout/stderr semantics**               | Medium–Low (merges via `2>&1` in key paths) |                   High |                                 High |
| **Token-efficiency** (caps/grouping/summaries)      |                                  Medium–Low |                   High |                               Medium |
| **“Never fail / passthrough fallback”** in practice |                                      Medium |                   High |                               Medium |
| **Doc ↔ artifact consistency**                      |                                         Low |                   High |                           Medium–Low |
| **Operational overhead** (deps/compile)             |                                         Low | Medium (needs python3) |                 High (build/install) |

---

## Proposal-by-proposal findings

### 1) GLM 5.1 (Bash `xwrap.sh`)

**What it does well**

* **Zero external runtime dependency** beyond the underlying tools; the multicall/symlink approach is correct and operationally simple.
* Implements **sed ranged-read gating** and appends a trailer with byte/line counts.
* Adds **some output bounding** (only for the “annotate” phase via `MAX_ANNOTATE`).

**Key issues / risks**

* **Doc/implementation mismatch on filename-safe parsing for `fd`:**

  * The proposal text emphasizes `fd -0`/NUL-splitting as critical; the shipped `xwrap.sh` **does not** actually force `-0` nor parse NUL-delimited output.
  * It captures `fd` output into a shell variable and splits by newline, which is inherently not filename-safe.
* **Doc/implementation mismatch on macOS-default Bash compatibility:**

  * The proposal text claims a “bash 3.2+” compatible subset, but the shipped `xwrap.sh` uses features that require newer Bash (for example `local -A ...` associative arrays).
* **stdout/stderr conflation**:

  * `fd` and `rg` are executed with `2>&1` captured into `output=...`, so stderr is merged into stdout. In non-error cases, warnings/noise can be mis-parsed as paths or match lines.
* **Memory/latency behavior on large result sets**:

  * Capturing full tool output into a variable is fragile for big repos (both performance and correctness under very large outputs).
* **Output adds “human headers”** (`--- file info (...) ---`, warning banners), which is explicitly at odds with the “avoid prettifiers” guidance; it increases token burn and makes downstream parsing less deterministic.

**Net**: Good ergonomics, but as-shipped it is the least defensible on correctness (path parsing + stderr handling) and is the most divergent from the task’s “LLM-efficient formatting” intent.

---

### 2) GPT-5.4 (Python `llm_inspect.py`)

**What it does well**

* **Correctly uses machine channels**:

  * `fd-x`: forces `fd -0` and parses NUL paths.
  * `rg-x`: uses `rg --json` for match mode; switches to `rg -0` (NUL) for file-list modes.
* **Separates stdout/stderr correctly** (captures both and re-emits without conflation).
* **Token-efficiency is strong**:

  * `fd-x`: emits tabular rows + “omitted N more” + a compact summary.
  * `rg-x`: groups matches by file; prints one header per file with `B`, `L`, and `H` (hits), and caps per-file lines.
* **Line gating is implemented** not only for `sed-x` but also for `rg-x` match lines (this directly addresses the “giant JSON log line match” failure mode).
* **Fallback behavior is generally correct**: conservative interception; on parse/transform failure it replays raw captured output.

**Weak spots / gaps**

* **Python availability**: if you truly want “system built-in only,” this is not guaranteed on all macOS installs without Xcode/brew tooling.
* **A small conservative choice reduces wrapper value**:

  * if the user already supplies `rg -0/--null`, the script chooses passthrough rather than using the wrapper’s own parsing + metadata path; this is a minor policy choice, but it means “already-safe” invocations can miss metadata augmentation.
* `sed-x` returns `0` on the wrapper-implemented ranged read path (not mirroring every nuanced `sed` exit behavior), though in practice this is usually acceptable for `sed -n 'a,bp' file`.

**Net**: This is the most internally consistent proposal (doc matches artifact), and it best satisfies the task’s intent: **safe parsing**, **deterministic compact output**, **caps**, and **best-effort fallback**.

---

### 3) GPT-5.2 (Rust `llm-inspect-wrappers-rs/`)

**What it does well**

* Strong **systems-level correctness posture**:

  * `fd-x` uses `--print0` and parses NUL output robustly.
  * It computes **bounded “file facts”** (bytes, loc≈, max_line≈, binary hint) via a scan budget, which is strictly better than naïvely trusting `wc -l` for the “20 lines but each is huge JSON” scenario.
* **Clean output discipline**:

  * Uses TSV “facts” appended to canonical prefixes; emits reserved `@meta\t...` lines.
* **Caps exist** for `rg-x` match printing, and `sed-x` truncation is byte-based and deterministic.
* Preserves stderr and generally cleanly maps exit codes.

**Key issues / risks**

* **Doc/AGENTS mismatch with the shipped code**:

  * The proposal text and `AGENTS_addon.md` mention features like `fd-x --overview[=DEPTH]` (dir rollups / extension histograms), but the Rust binary as provided does **not** implement `--overview`.
* **Drop-in flag conflicts in `fd-x` (wrapper flag vs underlying tool flag):**

  * The shipped Rust `fd-x` adds wrapper flags like `--format` (tsv/jsonl), which collides with `fd`’s own `--format` flag unless users remember to separate forwarded args with `--` (reducing “drop-in replacement” ergonomics).
* **`rg-x` file-list mode parsing is newline-based**:

  * It does not currently force `rg -0/--null` for file-list modes, so it is not filename-safe in that specific mode (rare, but it’s exactly the class of issue the task calls out for “filename-stream parsing” concerns).
* **No gating for huge `rg` match lines**:

  * It prints match text directly (trimmed), which reintroduces the “giant JSON log line match” token-waste failure mode unless users always switch to `sed-x` for inspection.

**Net**: Architecturally the most “productionable” long-term direction, but **as an artifact bundle** it is currently a **partial implementation** relative to what its own proposal text promises, and it misses two high-value protections: `rg` file-list NUL safety and `rg` match-line gating.

---

## Cross-validation: where the proposals agree (and what that implies)

### Strong consensus (you can treat as “design invariants”)

* **Multicall dispatch by `argv[0]` + symlinks** is the right installation story.
* **`fd` results must be parsed safely**; the existence of `fd -0/--print0` is recognized and (in two of three artifacts) implemented.
* **`rg --json` is the best match-mode channel**, but cannot be used for `-l/--files*`/count modes; those require separate handling.
* **`sed-x` should only intercept the narrow ranged-read idiom** and otherwise pass through.

### Key divergence (and the most defensible resolution)

* **Parsing “human text” vs using structured streams**:

  * Opus leans toward parsing canonical human output (and the artifact does this).
  * GPT-5.4 and GPT-5.2 lean toward **structured streams** (`fd -0`, `rg --json`, `rg -0`) where possible.
  * Given the explicit “filename-stream parsing” note in the task, the structured-stream approach is more defensible.

### Internal inconsistencies you should resolve before proceeding

* **Opus**: design text claims `fd -0` is critical, but the Bash artifact does not implement it.
* **GPT-5.2**: design + AGENTS mention `--overview`/rollups, but the Rust artifact does not implement it.

Those inconsistencies matter because they will mislead the consuming agent (Codex/Claude Code) when it tries to rely on documented wrapper behavior.

---

## Bottom line

* If your priority is **fastest path to a correct, usable drop-in** that meets the task’s objectives with minimal further work, **GPT-5.4 (Python)** is currently the strongest *as-delivered* bundle: safe parsing, compact grouped output, capping, line gating, and consistent docs.
* If your priority is **long-term robustness and a single compiled binary**, **GPT-5.2 (Rust)** is the strongest *direction*, but it needs targeted work to match its own spec (notably: `rg` file-list NUL mode, `rg` match-line gating, and either implement or remove “overview” promises).
* **Opus (Bash)** is attractive operationally, but the shipped artifact would need structural fixes (NUL-safe parsing, stdout/stderr separation, avoid full-output capture into variables, and reduce non-essential banners) before it reliably addresses the task’s failure modes.
