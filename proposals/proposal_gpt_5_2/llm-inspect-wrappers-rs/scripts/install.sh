#!/usr/bin/env bash
set -euo pipefail

DEST="${1:-$HOME/.local/bin}"

mkdir -p "$DEST"

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

(cd "$ROOT" && cargo build --release)

install -m 0755 "$ROOT/target/release/tool-x" "$DEST/tool-x"
ln -sf "$DEST/tool-x" "$DEST/fd-x"
ln -sf "$DEST/tool-x" "$DEST/rg-x"
ln -sf "$DEST/tool-x" "$DEST/sed-x"

echo "Installed: $DEST/tool-x (symlinks: fd-x, rg-x, sed-x)"
