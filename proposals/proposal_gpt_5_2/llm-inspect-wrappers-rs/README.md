# tool-x (reference skeleton)

A **reference Rust repo skeleton** for implementing thin, best-effort wrappers around:

- `fd` → `fd-x`
- `rg` → `rg-x`
- `sed` (ranged reads) → `sed-x`

The intent is to keep the wrappers **deterministic**, **fast**, and **non-fragile** for LLM agentic tooling:

- Prefer structured streams (`fd --print0`, `rg --json`) when parsing is required.
- Add low-footprint file facts (`bytes`, `loc≈`, `max_line≈`) to reduce inspection churn.
- **Never fail**: on unsupported argument patterns or errors, **passthrough** to the underlying tool.

## What this repo is

- A runnable structure with concrete modules, interfaces, and test scaffolding.
- A starting point for implementing the behavior described in your design notes.

## What this repo is not

- A complete production implementation.
- A replacement for careful CLI/UX review (limits, truncation policies, exit-code semantics).

## Build & install (local)

Prereqs: Rust toolchain (stable), and `fd` + `rg` available on `PATH`.

```bash
cargo build --release

# Install single multicall binary
install -m 0755 target/release/tool-x ~/.local/bin/tool-x

# Symlinks determine mode
ln -sf ~/.local/bin/tool-x ~/.local/bin/fd-x
ln -sf ~/.local/bin/tool-x ~/.local/bin/rg-x
ln -sf ~/.local/bin/tool-x ~/.local/bin/sed-x
```

Or:

```bash
./scripts/install.sh ~/.local/bin
```

## Quick sanity checks (local)

```bash
cargo test

# Show help
fd-x --help
rg-x --help
sed-x --help

# Preflight-ish listing with file facts
fd-x -tf -d 3 .

# Structured search (match mode uses rg --json internally)
rg-x "TODO" src

# Safe ranged read (wrapper reads file itself; avoids printing huge lines verbatim)
sed-x -n '1,120p' README.md
```

## Output format (default)

- One record per path / match line.
- Append wrapper facts as TSV-like `key=value` pairs.
- Wrapper meta lines start with `@meta\t...`.

Example:

```
./src/main.rs\tbytes=1234\tloc=210\tmax_line≈140
@meta\tfiles=32\ttotal_bytes=...
```

## Environment overrides

You may pin underlying tools for deterministic behavior:

- `FD_X_FD=/path/to/fd`
- `RG_X_RG=/path/to/rg`
- `SED_X_SED=/usr/bin/sed`

## AGENTS.md integration snippet

```markdown
### Repo inspection wrappers (prefer over raw fd/rg/sed)
- Use `fd-x` for layout + file facts, then scope searches.
- Use `rg-x` for match search; it uses `rg --json` when safe.
- Use `sed-x` for `sed -n 'a,bp' file` to truncate pathological lines.
```
