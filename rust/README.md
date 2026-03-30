# Rust implementation

Single multicall binary intended to be symlinked as `fd-x`, `rg-x`, and `sed-x`.

## Build

```bash
cargo build --release
```

The binary will be at `target/release/llm-inspect-wrappers`.

## Install

Example (from this directory):

```bash
install -m 755 target/release/llm-inspect-wrappers /usr/local/bin/llm-inspect-wrappers
ln -sf /usr/local/bin/llm-inspect-wrappers /usr/local/bin/fd-x
ln -sf /usr/local/bin/llm-inspect-wrappers /usr/local/bin/rg-x
ln -sf /usr/local/bin/llm-inspect-wrappers /usr/local/bin/sed-x
```

## Requirements

- Rust toolchain (for building)
- `fd`, `rg`, `sed` available on `PATH` (at runtime)

## Tests

```bash
cargo test
```
