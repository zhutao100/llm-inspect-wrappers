# llm-inspect-wrappers

Thin, best-effort wrappers around `fd`, `rg`, and ranged `sed` reads to help LLM agentic tools:

- get a fast repo/file overview (size + line counts) before deep reads
- avoid token blowups from pathological long lines (logs, minified files, huge JSON lines)

This repo contains three standalone implementations:

- `bash/` (single multicall shell script)
- `python/` (single multicall Python script)
- `rust/` (single multicall compiled binary)

Each implementation provides the same wrapper names:

- `fd-x` — file discovery + file metadata (bytes, lines)
- `rg-x` — search + file metadata + long-line gating (match mode)
- `sed-x` — ranged reads (`sed -n 'a,bp' [file|-]`) + long-line gating + file metadata (or stdin stats)

For deterministic wrapper output, wrapped `rg` invocations ignore `RIPGREP_CONFIG_PATH` (passthrough modes keep the original environment) and run captured commands with `stdin=/dev/null` so `rg-x PATTERN` still searches the filesystem in CI/piped environments.

## Output format

`fd-x` (and `rg-x` in filelist modes like `rg-x -l ...`) emit a file table followed by a single `@meta` line.

- Regular files: `path<TAB>bytes=N<TAB>lines=M`
- Non-regular (dir/symlink/other/missing): `path` (no extra fields)
- Meta line: `@meta<TAB>tool=...<TAB>total=...<TAB>printed=...<TAB>omitted=...` (plus `mode=...` when applicable)

Help/version flags are passed through without wrapper post-processing (canonical tool output, original newlines, no `@meta`):
`-h`, `--help`, `-V`, `--version`.

Examples:

```bash
fd-x -tf
# short.txt	bytes=11	lines=1
# @meta	tool=fd-x	total=1	printed=1	omitted=0

fd-x -td
# src/
# tests/
# @meta	tool=fd-x	total=2	printed=2	omitted=0
```

`sed-x` prints the selected range, then a single `@meta` line.

- File input: `@meta<TAB>tool=sed-x<TAB>path=...<TAB>bytes=...<TAB>lines=...<TAB>range=a..b` (plus `truncated_lines=...` when gating occurs)
- Stdin input: `@meta<TAB>tool=sed-x<TAB>source=stdin<TAB>range=a..b<TAB>bytes=...<TAB>lines=...<TAB>complete=0|1`
  - When `complete=0`, `reason=cap` (hit a scan cap) or `reason=tty` (stdin is a terminal and was not drained).

Stdin total scanning is bounded by:
- `LLM_X_SEDX_STDIN_MAX_LINES` (default: `200000`)
- `LLM_X_SEDX_STDIN_MAX_BYTES` (default: `10000000`)

## Caps & omissions

Wrappers intentionally cap printed output (and report totals via `@meta`) to avoid flooding an LLM context window.

Key knobs:
- `LLM_X_MAX_FD_ROWS` (file tables: `fd-x`, `rg-x -l`, etc.)
- `LLM_X_MAX_RG_FILES` (file groups in `rg-x` match mode)
- `LLM_X_MAX_RG_MATCH_LINES_PER_FILE` (match lines per file in `rg-x` match mode)
- `LLM_X_MAX_RG_NO_OMIT_MATCH_LINES` (print all match lines when results are small)

Details: `docs/caps_and_omissions.md`

## Tests

- Per-implementation tests live under each subdirectory.
- Root `tests/` cross-validates the three implementations against the same fixtures.

Run cross-validation:

```bash
python3 -m unittest discover -s tests -q
```

## Git hooks (prek)

This repo uses `prek` to run `.pre-commit-config.yaml`. The committed `.githooks/pre-commit` is a self-healing shim that re-runs `prek run` and auto-stages fixes until the staged set converges.

This repo uses prek **workspace mode**: additional configs live under `bash/.pre-commit-config.yaml` and `rust/.pre-commit-config.yaml` (see `prek list`).

One-time setup:

```bash
git config core.hooksPath .githooks
prek prepare-hooks
```

## Build & release

Build + package all external-facing artifacts into `dist/`:

```bash
scripts/release_local.sh
```
