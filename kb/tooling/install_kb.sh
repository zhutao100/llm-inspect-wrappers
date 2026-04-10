#!/usr/bin/env bash
set -euo pipefail

# Install `kb` from the latest GitHub release into `.kb-tool/bin/kb`.
#
# Optional env overrides:
# - KB_TOOL_REPO: GitHub repo in `owner/name` form (default: zhutao100/kb-tool)
# - KB_TOOL_TAG:  Release tag like `v0.1.0` (default: latest)
# - KB_TOOL_PLATFORM: `macos-arm64`, `macos-x86_64`, `linux-x86_64` (default: derived from uname)
# - KB_TOOL_BIN_DIR: install dir (default: .kb-tool/bin)

need_cmd() {
  local c="$1"
  if ! command -v "${c}" >/dev/null 2>&1; then
    echo "error: required command not found in PATH: ${c}" >&2
    exit 2
  fi
}

need_cmd curl
need_cmd python3
need_cmd tar

KB_TOOL_REPO="${KB_TOOL_REPO:-zhutao100/kb-tool}"
KB_TOOL_TAG="${KB_TOOL_TAG:-}"
KB_TOOL_PLATFORM="${KB_TOOL_PLATFORM:-}"
KB_TOOL_BIN_DIR="${KB_TOOL_BIN_DIR:-.kb-tool/bin}"

repo_root="$(git rev-parse --show-toplevel 2>/dev/null || true)"
if [[ -n "${repo_root}" ]]; then
  cd "${repo_root}"
fi

if [[ -z "${KB_TOOL_TAG}" ]]; then
  KB_TOOL_TAG="$(
    curl -fsSL "https://api.github.com/repos/${KB_TOOL_REPO}/releases/latest" |
      python3 -c 'import json,sys; print(json.load(sys.stdin).get("tag_name",""))'
  )"
fi

if [[ -z "${KB_TOOL_TAG}" ]]; then
  echo "error: failed to determine latest kb-tool release tag" >&2
  echo "hint: set KB_TOOL_TAG=vX.Y.Z and rerun" >&2
  exit 2
fi

if [[ -z "${KB_TOOL_PLATFORM}" ]]; then
  os="$(uname -s)"
  arch="$(uname -m)"

  case "${os}" in
    Darwin) os="macos" ;;
    Linux) os="linux" ;;
    *)
      echo "error: unsupported OS: ${os}" >&2
      exit 2
      ;;
  esac

  case "${arch}" in
    x86_64|amd64) arch="x86_64" ;;
    arm64|aarch64) arch="arm64" ;;
    *)
      echo "error: unsupported architecture: ${arch}" >&2
      exit 2
      ;;
  esac

  KB_TOOL_PLATFORM="${os}-${arch}"
fi

asset="kb-${KB_TOOL_PLATFORM}.tar.gz"
url="https://github.com/${KB_TOOL_REPO}/releases/download/${KB_TOOL_TAG}/${asset}"

tmp_dir="$(mktemp -d)"
cleanup() { rm -rf "${tmp_dir}"; }
trap cleanup EXIT

echo "kb-tool: downloading ${url}..." >&2
curl -fsSL "${url}" -o "${tmp_dir}/${asset}"
tar -xzf "${tmp_dir}/${asset}" -C "${tmp_dir}"

if [[ ! -f "${tmp_dir}/kb" ]]; then
  echo "error: expected release archive to contain a 'kb' binary" >&2
  exit 2
fi

mkdir -p "${KB_TOOL_BIN_DIR}"
cp "${tmp_dir}/kb" "${KB_TOOL_BIN_DIR}/kb"
chmod +x "${KB_TOOL_BIN_DIR}/kb"

echo "kb-tool: installed ${KB_TOOL_TAG} (${KB_TOOL_PLATFORM}) to ${KB_TOOL_BIN_DIR}/kb" >&2
