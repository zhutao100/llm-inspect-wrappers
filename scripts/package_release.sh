#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage:
  scripts/package_release.sh [--only all|scripts|rust] [--version <vX.Y.Z>] [--out-dir <dir>]

Builds and packs release artifacts into <out-dir>:
  - bash wrappers tarball (portable)
  - python wrappers tarball (portable)
  - rust binary tarball (platform-specific, uses `cargo build --release`)

Examples:
  scripts/package_release.sh
  scripts/package_release.sh --version v0.1.0
  scripts/package_release.sh --only rust --version v0.1.0
USAGE
}

repo_root="$(
  cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." >/dev/null 2>&1
  pwd -P
)"
cd "$repo_root"

only="all"
out_dir="$repo_root/dist"
version="${LLM_INSPECT_VERSION:-}"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --only)
      only="${2-}"
      shift 2
      ;;
    --version)
      version="${2-}"
      shift 2
      ;;
    --out-dir)
      out_dir="${2-}"
      shift 2
      ;;
    -h | --help)
      usage
      exit 0
      ;;
    *)
      echo "error: unknown arg: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

case "$only" in
  all | scripts | rust) ;;
  *)
    echo "error: --only must be one of: all|scripts|rust" >&2
    exit 2
    ;;
esac

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "error: missing required command: $1" >&2
    exit 127
  }
}

detect_version() {
  if [[ -n "$version" ]]; then
    echo "$version"
    return 0
  fi

  if command -v git >/dev/null 2>&1 && git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
    git describe --tags --dirty --always 2>/dev/null || true
  fi
}

sanitize_for_filename() {
  local s="${1-}"
  s="${s//\//_}"
  s="${s// /_}"
  echo "$s"
}

sha256_file() {
  local p="$1"
  python3 - "$p" <<'PY'
import hashlib
import sys
from pathlib import Path

p = Path(sys.argv[1])
h = hashlib.sha256()
with p.open("rb") as f:
    for chunk in iter(lambda: f.read(1024 * 1024), b""):
        h.update(chunk)
print(h.hexdigest())
PY
}

make_tarball() {
  local src_dir="$1"
  local tar_path="$2"
  local base
  base="$(basename "$src_dir")"
  tar -czf "$tar_path" -C "$(dirname "$src_dir")" "$base"
}

tmp_root="$(mktemp -d "${TMPDIR:-/tmp}/llm-inspect-wrappers.dist.XXXXXX")"
cleanup() {
  rm -rf "$tmp_root"
}
trap cleanup EXIT

ver="$(detect_version)"
ver="${ver:-dev}"
ver="$(sanitize_for_filename "$ver")"

rm -rf "$out_dir"
mkdir -p "$out_dir"

package_bash() {
  local pkg="llm-inspect-wrappers-bash-${ver}"
  local stage="$tmp_root/$pkg"
  mkdir -p "$stage/bin"
  cp -p bash/xwrap "$stage/bin/xwrap"
  ln -s xwrap "$stage/bin/fd-x"
  ln -s xwrap "$stage/bin/rg-x"
  ln -s xwrap "$stage/bin/sed-x"
  cp -p README.md LICENSE "$stage/"
  make_tarball "$stage" "$out_dir/${pkg}.tar.gz"
}

package_python() {
  local pkg="llm-inspect-wrappers-python-${ver}"
  local stage="$tmp_root/$pkg"
  mkdir -p "$stage/bin"
  cp -p python/llm_inspect.py "$stage/bin/llm_inspect.py"
  chmod 0755 "$stage/bin/llm_inspect.py"
  ln -s llm_inspect.py "$stage/bin/fd-x"
  ln -s llm_inspect.py "$stage/bin/rg-x"
  ln -s llm_inspect.py "$stage/bin/sed-x"
  cp -p README.md LICENSE "$stage/"
  make_tarball "$stage" "$out_dir/${pkg}.tar.gz"
}

rust_host_triple() {
  rustc -vV | sed -n 's/^host: //p'
}

package_rust() {
  need_cmd cargo
  local host
  host="$(rust_host_triple)"
  host="$(sanitize_for_filename "$host")"

  (cd rust && cargo build --release -q)

  local pkg="llm-inspect-wrappers-rust-${host}-${ver}"
  local stage="$tmp_root/$pkg"
  mkdir -p "$stage/bin"
  cp -p rust/target/release/llm-inspect-wrappers "$stage/bin/llm-inspect-wrappers"
  ln -s llm-inspect-wrappers "$stage/bin/fd-x"
  ln -s llm-inspect-wrappers "$stage/bin/rg-x"
  ln -s llm-inspect-wrappers "$stage/bin/sed-x"
  cp -p README.md LICENSE "$stage/"
  make_tarball "$stage" "$out_dir/${pkg}.tar.gz"
}

need_cmd tar
need_cmd python3

if [[ "$only" == "all" || "$only" == "scripts" ]]; then
  package_bash
  package_python
fi

if [[ "$only" == "all" || "$only" == "rust" ]]; then
  need_cmd rustc
  package_rust
fi

(
  cd "$out_dir"
  : >SHA256SUMS
  for f in *.tar.gz; do
    [[ -f "$f" ]] || continue
    printf '%s  %s\n' "$(sha256_file "$out_dir/$f")" "$f" >>SHA256SUMS
  done
)

ls -1 "$out_dir" >/dev/null
