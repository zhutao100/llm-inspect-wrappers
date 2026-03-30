#!/usr/bin/env python3
"""
llm_inspect.py

Multicall, best-effort wrappers for LLM-friendly repo inspection:

- fd-x: file discovery + file metadata (bytes + line count)
- rg-x: search + grouped output + long-line gating + file metadata
- sed-x: ranged reads (`sed -n 'a,bp' file`) + long-line gating + file metadata

Install (one option):
  ln -sf llm_inspect.py fd-x
  ln -sf llm_inspect.py rg-x
  ln -sf llm_inspect.py sed-x

Or invoke directly:
  ./llm_inspect.py fd-x ...
  ./llm_inspect.py rg-x ...
  ./llm_inspect.py sed-x ...
"""

from __future__ import annotations

import base64
import dataclasses
import json
import os
import re
import stat
import subprocess
import sys
from typing import Iterable


@dataclasses.dataclass(frozen=True)
class Config:
    max_fd_rows: int = int(os.getenv("LLM_X_MAX_FD_ROWS", "200"))
    max_rg_files: int = int(os.getenv("LLM_X_MAX_RG_FILES", "80"))
    max_rg_match_lines_per_file: int = int(os.getenv("LLM_X_MAX_RG_MATCH_LINES_PER_FILE", "4"))

    soft_line_chars: int = int(os.getenv("LLM_X_SOFT_LINE_CHARS", "400"))
    hard_line_chars: int = int(os.getenv("LLM_X_HARD_LINE_CHARS", "2000"))
    preview_head_chars: int = int(os.getenv("LLM_X_HEAD_CHARS", "160"))
    preview_tail_chars: int = int(os.getenv("LLM_X_TAIL_CHARS", "80"))

    wc_batch_arg_budget: int = int(os.getenv("LLM_X_WC_ARG_BUDGET", "60000"))


CFG = Config()


REAL_TOOL: dict[str, str] = {
    "fd-x": "fd",
    "rg-x": "rg",
    "sed-x": "sed",
}


@dataclasses.dataclass
class FileMeta:
    kind: str  # file | dir | symlink | other | missing
    bytes: int | None
    lines: int | None


@dataclasses.dataclass
class RgFileGroup:
    path: str
    hits: int = 0
    shown_lines: list[str] = dataclasses.field(default_factory=list)
    omitted_lines: int = 0


@dataclasses.dataclass(frozen=True)
class SedRangeSpec:
    start: int
    end: int
    path: str


RANGE_RE = re.compile(r"^\s*(\d+)\s*,\s*(\d+)\s*p\s*$")
JSON_KEY_RE = re.compile(r'"([^"\\]{1,48})"\s*:')
BASE64ISH_RE = re.compile(rb"^[A-Za-z0-9+/=_-]+$")


def env_c() -> dict[str, str]:
    env = dict(os.environ)
    env["LC_ALL"] = "C"
    env["LANG"] = "C"
    return env


def sdecode(b: bytes) -> str:
    return b.decode("utf-8", "surrogateescape")


def run_capture(argv: list[str]) -> subprocess.CompletedProcess[bytes]:
    return subprocess.run(
        argv,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=False,
        env=env_c(),
    )


def replay_raw(cp: subprocess.CompletedProcess[bytes]) -> int:
    sys.stdout.buffer.write(cp.stdout)
    sys.stderr.buffer.write(cp.stderr)
    return int(cp.returncode)


def passthrough(argv: list[str]) -> int:
    cp = subprocess.run(argv)
    return int(cp.returncode)


def escape_field(s: str) -> str:
    return s.replace("\\", "\\\\").replace("\t", "\\t").replace("\n", "\\n").replace("\r", "\\r")


def strip_dot_slash(path: str) -> str:
    return path[2:] if path.startswith("./") else path


def parse_nul_paths(buf: bytes) -> list[str]:
    if not buf:
        return []
    parts = buf.split(b"\0")
    if parts and parts[-1] == b"":
        parts = parts[:-1]
    return [strip_dot_slash(sdecode(p)) for p in parts if p]


def python_count_newlines(path: str) -> int:
    n = 0
    with open(path, "rb") as f:
        while True:
            chunk = f.read(1 << 20)
            if not chunk:
                break
            n += chunk.count(b"\n")
    return n


def safe_for_wc(path: str) -> bool:
    base = os.path.basename(path)
    if "\n" in path:
        return False
    if base.startswith("-"):
        return False
    return True


def chunked_paths(paths: list[str], budget: int) -> Iterable[list[str]]:
    batch: list[str] = []
    used = 0
    for p in paths:
        cost = len(p.encode("utf-8", "surrogateescape")) + 1
        if batch and used + cost > budget:
            yield batch
            batch = []
            used = 0
        batch.append(p)
        used += cost
    if batch:
        yield batch


def wc_line_counts(paths: list[str]) -> dict[str, int]:
    out: dict[str, int] = {}
    wc_safe = [p for p in paths if safe_for_wc(p)]
    wc_unsafe = [p for p in paths if not safe_for_wc(p)]

    for batch in chunked_paths(wc_safe, CFG.wc_batch_arg_budget):
        cp = run_capture(["wc", "-l", *batch])
        if cp.returncode != 0:
            raise RuntimeError(f"wc failed rc={cp.returncode}")

        rows = [line for line in sdecode(cp.stdout).splitlines() if line.strip()]
        if len(batch) > 1 and rows and rows[-1].strip().endswith(" total"):
            rows = rows[:-1]

        if len(rows) != len(batch):
            raise RuntimeError("wc output row count mismatch")

        for p, row in zip(batch, rows):
            m = re.match(r"^\s*(\d+)\s+", row)
            if not m:
                raise RuntimeError(f"cannot parse wc row: {row!r}")
            out[p] = int(m.group(1))

    for p in wc_unsafe:
        out[p] = python_count_newlines(p)

    return out


def collect_meta(paths: list[str]) -> dict[str, FileMeta]:
    metas: dict[str, FileMeta] = {}
    regulars: list[str] = []

    for p in paths:
        try:
            st = os.lstat(p)
        except OSError:
            metas[p] = FileMeta(kind="missing", bytes=None, lines=None)
            continue

        if stat.S_ISREG(st.st_mode):
            metas[p] = FileMeta(kind="file", bytes=st.st_size, lines=None)
            regulars.append(p)
        elif stat.S_ISDIR(st.st_mode):
            metas[p] = FileMeta(kind="dir", bytes=None, lines=None)
        elif stat.S_ISLNK(st.st_mode):
            metas[p] = FileMeta(kind="symlink", bytes=None, lines=None)
        else:
            metas[p] = FileMeta(kind="other", bytes=None, lines=None)

    if regulars:
        try:
            line_counts = wc_line_counts(regulars)
        except Exception:
            line_counts = {p: python_count_newlines(p) for p in regulars}
        for p in regulars:
            metas[p].lines = line_counts.get(p)

    return metas


def render_file_table(tool: str, paths: list[str], metas: dict[str, FileMeta], max_rows: int) -> str:
    total = len(paths)
    shown = sorted(paths)[:max_rows]
    omitted = total - len(shown)

    out: list[str] = []
    for p in shown:
        meta = metas.get(p, FileMeta(kind="missing", bytes=None, lines=None))
        b = str(meta.bytes) if meta.bytes is not None else "-"
        l = str(meta.lines) if meta.lines is not None else "-"
        out.append(f"{escape_field(p)}\tkind={meta.kind}\tbytes={b}\tlines={l}")

    out.append(f"@meta\ttool={tool}\ttotal={total}\tprinted={len(shown)}\tomitted={omitted}")
    return "\n".join(out) + "\n"


def safe_preview_text(s: str) -> str:
    return s.replace("\\", "\\\\").replace("\r", "\\r").replace("\n", "\\n").replace("\t", "\\t")


def safe_preview_bytes(raw: bytes, max_chars: int) -> str:
    try:
        s = raw[:max_chars].decode("utf-8")
    except UnicodeDecodeError:
        s = raw[:max_chars].decode("utf-8", "replace")
    return safe_preview_text(s)


def classify_line(raw: bytes) -> tuple[str, str]:
    """
    Returns (kind, hint_suffix_without_leading_space).
    kind:
      plain | long | json | base64 | minified | binary
    """
    try:
        txt = raw.decode("utf-8")
    except UnicodeDecodeError:
        return ("binary", "")

    stripped = raw.strip()

    if stripped[:1] in (b"{", b"[") and len(raw) >= CFG.soft_line_chars:
        keys: list[str] = []
        head_txt = txt[:4096]
        for m in JSON_KEY_RE.finditer(head_txt):
            k = m.group(1)
            if k not in keys:
                keys.append(k)
            if len(keys) >= 6:
                break
        hint = f"keys={','.join(keys)}" if keys else ""
        return ("json", hint)

    if len(stripped) >= CFG.soft_line_chars and b" " not in stripped and BASE64ISH_RE.fullmatch(stripped[:4096]):
        return ("base64", "")

    if len(txt) >= CFG.soft_line_chars:
        ws = sum(ch.isspace() for ch in txt) / max(1, len(txt))
        punct = sum((not ch.isalnum()) and (not ch.isspace()) for ch in txt) / max(1, len(txt))
        if ws < 0.05 and punct > 0.25:
            return ("minified", "")

    return ("plain", "")


def should_gate_line(raw: bytes) -> tuple[bool, str, str]:
    kind, hint = classify_line(raw)

    if len(raw) > CFG.hard_line_chars:
        return (True, kind if kind != "plain" else "long", hint)

    if len(raw) > CFG.soft_line_chars and kind != "plain":
        return (True, kind, hint)

    return (False, kind, hint)


def truncated_marker(prefix: str, raw: bytes, kind: str, hint: str) -> str:
    head = safe_preview_bytes(raw, CFG.preview_head_chars)
    tail = (
        safe_preview_bytes(raw[-CFG.preview_tail_chars :], CFG.preview_tail_chars)
        if len(raw) > CFG.preview_head_chars
        else ""
    )
    extra = f" {hint}" if hint else ""
    return f"[{prefix} len={len(raw)} kind={kind}{extra} head='{head}' tail='{tail}']"


def rg_decode_obj_text(obj: dict) -> str:
    if "text" in obj:
        return obj["text"]
    if "bytes" in obj:
        raw = base64.b64decode(obj["bytes"])
        return raw.decode("utf-8", "replace")
    return ""


# ------------------------------------------------------------------------------
# fd-x
# ------------------------------------------------------------------------------

FD_UNSUPPORTED_EXACT = {
    "-0",
    "--print0",
    "-x",
    "-X",
    "--exec",
    "--exec-batch",
}
FD_UNSUPPORTED_PREFIX = (
    "--exec=",
    "--exec-batch=",
    "--format",
    "--format=",
)


def fd_x_supported(args: list[str]) -> bool:
    for a in args:
        if a in FD_UNSUPPORTED_EXACT:
            return False
        if any(a.startswith(p) for p in FD_UNSUPPORTED_PREFIX):
            return False
    return True


def main_fd(args: list[str]) -> int:
    real = "fd"

    if not fd_x_supported(args):
        return passthrough([real, *args])

    cp = run_capture([real, *args, "-0"])
    if cp.returncode != 0:
        return replay_raw(cp)

    try:
        paths = parse_nul_paths(cp.stdout)
        metas = collect_meta(paths)
        out = render_file_table("fd-x", paths, metas, max_rows=CFG.max_fd_rows)
        sys.stdout.write(out)
        sys.stderr.buffer.write(cp.stderr)
        return int(cp.returncode)
    except Exception:
        return replay_raw(cp)


# ------------------------------------------------------------------------------
# rg-x
# ------------------------------------------------------------------------------

RG_PASSTHROUGH_EXACT = {
    "--json",
    "--passthru",
    "--vimgrep",
    "--null",
    "-0",
    "-c",
    "--count",
    "--count-matches",
    "-o",
    "--only-matching",
    "-r",
    "--replace",
}
RG_PASSTHROUGH_PREFIX = ("--replace=",)

RG_FILELIST_EXACT = {
    "--files",
    "-l",
    "--files-with-matches",
    "-L",
    "--files-without-match",
}


def short_flag_bundle_contains(arg: str, chars: set[str]) -> bool:
    return arg.startswith("-") and not arg.startswith("--") and any(c in arg[1:] for c in chars)


def rg_should_passthrough(args: list[str]) -> bool:
    for a in args:
        if a in RG_PASSTHROUGH_EXACT:
            return True
        if any(a.startswith(p) for p in RG_PASSTHROUGH_PREFIX):
            return True
        if short_flag_bundle_contains(a, {"0", "c", "o"}):
            return True
    return False


def rg_is_filelist_mode(args: list[str]) -> bool:
    for a in args:
        if a in RG_FILELIST_EXACT:
            return True
        if short_flag_bundle_contains(a, {"l", "L"}):
            return True
    return False


def render_rg_match_line(line_no: int | None, line_text: str) -> str:
    raw = line_text.encode("utf-8", "replace")
    gate, kind, hint = should_gate_line(raw)
    body = truncated_marker("rg-x truncated", raw, kind, hint) if gate else safe_preview_text(line_text.rstrip("\n"))
    prefix = f"{line_no}:" if line_no is not None else "?:"
    return prefix + body


def main_rg_filelist(args: list[str]) -> int:
    real = "rg"
    cp = run_capture([real, *args, "-0"])
    if cp.returncode == 2:
        return replay_raw(cp)

    try:
        paths = parse_nul_paths(cp.stdout)
        metas = collect_meta(paths)
        out = render_file_table("rg-x", paths, metas, max_rows=CFG.max_fd_rows)
        sys.stdout.write(out)
        sys.stderr.buffer.write(cp.stderr)
        return int(cp.returncode)
    except Exception:
        return replay_raw(cp)


def main_rg_json(args: list[str]) -> int:
    real = "rg"
    cp = run_capture([real, *args, "--json"])
    if cp.returncode == 2:
        return replay_raw(cp)

    groups_by_path: dict[str, RgFileGroup] = {}
    try:
        for raw_line in cp.stdout.splitlines():
            if not raw_line.strip():
                continue
            evt = json.loads(sdecode(raw_line))
            if evt.get("type") != "match":
                continue

            data = evt.get("data", {})
            path_obj = data.get("path")
            if not path_obj:
                continue

            path = rg_decode_obj_text(path_obj)
            path = strip_dot_slash(path)
            line_text = rg_decode_obj_text(data.get("lines", {}))
            line_no = data.get("line_number")
            submatches = data.get("submatches", [])
            hit_incr = max(1, len(submatches))

            grp = groups_by_path.setdefault(path, RgFileGroup(path=path))
            grp.hits += hit_incr
            if len(grp.shown_lines) < CFG.max_rg_match_lines_per_file:
                grp.shown_lines.append(render_rg_match_line(line_no, line_text))
            else:
                grp.omitted_lines += 1

        all_paths = sorted(groups_by_path.keys())
        shown_paths = all_paths[: CFG.max_rg_files]
        omitted_files = len(all_paths) - len(shown_paths)

        metas = collect_meta(shown_paths)

        out_lines: list[str] = []
        printed_match_lines = 0
        total_match_lines = 0

        for p in shown_paths:
            g = groups_by_path[p]
            meta = metas.get(p, FileMeta(kind="missing", bytes=None, lines=None))
            b = str(meta.bytes) if meta.bytes is not None else "-"
            l = str(meta.lines) if meta.lines is not None else "-"
            out_lines.append(
                f"@file\tpath={escape_field(p)}\tkind={meta.kind}\tbytes={b}\tlines={l}"
                f"\thits={g.hits}\tshown={len(g.shown_lines)}\tomitted={g.omitted_lines}"
            )
            out_lines.extend(g.shown_lines)
            printed_match_lines += len(g.shown_lines)
            total_match_lines += len(g.shown_lines) + g.omitted_lines

        for p in all_paths[len(shown_paths) :]:
            g = groups_by_path[p]
            total_match_lines += len(g.shown_lines) + g.omitted_lines

        omitted_match_lines = total_match_lines - printed_match_lines
        total_hits = sum(groups_by_path[p].hits for p in all_paths)

        out_lines.append(
            f"@meta\ttool=rg-x\tmode=match\tfiles={len(all_paths)}\tprinted_files={len(shown_paths)}"
            f"\tomitted_files={omitted_files}\tmatch_lines={total_match_lines}\tprinted_match_lines={printed_match_lines}"
            f"\tomitted_match_lines={omitted_match_lines}\thits={total_hits}"
        )
        sys.stdout.write("\n".join(out_lines) + "\n")
        sys.stderr.buffer.write(cp.stderr)
        return int(cp.returncode)
    except Exception:
        return replay_raw(cp)


def main_rg(args: list[str]) -> int:
    real = "rg"
    if rg_should_passthrough(args):
        return passthrough([real, *args])
    if rg_is_filelist_mode(args):
        return main_rg_filelist(args)
    return main_rg_json(args)


# ------------------------------------------------------------------------------
# sed-x
# ------------------------------------------------------------------------------


def parse_sed_range_spec(args: list[str]) -> SedRangeSpec | None:
    """
    Conservative support:
      sed -n '10,20p' file
      sed -n -e '10,20p' file
      sed -n -e10,20p file
    """
    quiet = False
    script: str | None = None
    files: list[str] = []

    i = 0
    while i < len(args):
        a = args[i]
        if a == "-n":
            quiet = True
        elif a == "-e":
            i += 1
            if i >= len(args):
                return None
            if script is not None:
                return None
            script = args[i]
        elif a.startswith("-e") and len(a) > 2:
            if script is not None:
                return None
            script = a[2:]
        elif a.startswith("-"):
            return None
        else:
            if script is None:
                script = a
            else:
                files.append(a)
        i += 1

    if not quiet or script is None or len(files) != 1 or files[0] == "-":
        return None

    m = RANGE_RE.fullmatch(script)
    if not m:
        return None

    start = int(m.group(1))
    end = int(m.group(2))
    if start < 1 or end < start:
        return None

    return SedRangeSpec(start=start, end=end, path=files[0])


def render_sed_line(lineno: int, raw: bytes) -> tuple[bytes, bool]:
    gate, kind, hint = should_gate_line(raw)

    if not gate:
        try:
            raw.decode("utf-8")
        except UnicodeDecodeError:
            gate = True
            kind = "binary"
            hint = ""

    if gate:
        marker = truncated_marker(f"sed-x truncated line={lineno}", raw, kind, hint)
        return (marker.encode("utf-8") + b"\n", True)

    return (raw, False)


def main_sed(args: list[str]) -> int:
    real = "sed"
    spec = parse_sed_range_spec(args)
    if spec is None:
        return passthrough([real, *args])

    try:
        metas = collect_meta([spec.path])
        meta = metas.get(spec.path, FileMeta(kind="missing", bytes=None, lines=None))
        b = str(meta.bytes) if meta.bytes is not None else "-"
        l = str(meta.lines) if meta.lines is not None else "-"

        truncated = 0
        with open(spec.path, "rb") as f:
            for lineno, raw in enumerate(f, start=1):
                if lineno < spec.start:
                    continue
                if lineno > spec.end:
                    break
                rendered, was_truncated = render_sed_line(lineno, raw)
                sys.stdout.buffer.write(rendered)
                if was_truncated:
                    truncated += 1

        sys.stdout.write(
            f"@meta\ttool=sed-x\tpath={escape_field(spec.path)}\tkind={meta.kind}\tbytes={b}\tlines={l}"
            f"\trange={spec.start}..{spec.end}\ttruncated_lines={truncated}\n"
        )
        return 0
    except Exception:
        return passthrough([real, *args])


# ------------------------------------------------------------------------------
# Main dispatch
# ------------------------------------------------------------------------------


def main(argv: list[str]) -> int:
    prog = os.path.basename(argv[0])

    # Convenience mode:
    #   llm_inspect.py fd-x ...
    if prog not in REAL_TOOL and len(argv) >= 2 and argv[1] in REAL_TOOL:
        prog = argv[1]
        args = argv[2:]
    else:
        args = argv[1:]

    if prog == "fd-x":
        return main_fd(args)
    if prog == "rg-x":
        return main_rg(args)
    if prog == "sed-x":
        return main_sed(args)

    sys.stderr.write("usage:\n  fd-x ...\n  rg-x ...\n  sed-x ...\nor:\n  llm_inspect.py fd-x ...\n")
    return 2


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
