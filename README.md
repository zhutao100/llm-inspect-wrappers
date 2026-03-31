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
- `sed-x` — ranged reads (`sed -n 'a,bp' file`) + long-line gating + file metadata

For deterministic wrapper output, wrapped `rg` invocations ignore `RIPGREP_CONFIG_PATH` (passthrough modes keep the original environment) and run captured commands with `stdin=/dev/null` so `rg-x PATTERN` still searches the filesystem in CI/piped environments.

## Tests

- Per-implementation tests live under each subdirectory.
- Root `tests/` cross-validates the three implementations against the same fixtures.

Run cross-validation:

```bash
python3 -m unittest discover -s tests -q
```

## Build & release

Build + package all external-facing artifacts into `dist/`:

```bash
scripts/release_local.sh
```
