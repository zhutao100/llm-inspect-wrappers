from __future__ import annotations

import re
import sys
from dataclasses import dataclass
from pathlib import Path

LOCAL_ONLY_CONFIG = Path("config/local-resources.yaml")
PYENV_VERSION_FILE = Path(".python-version")

_USERNAME_SEGMENT = r"[A-Za-z0-9._-]+"
_PYTHON_VERSION_PATTERN = re.compile(r"^3\.\d+(?:\.\d+)?$")

ABSOLUTE_HOME_PATTERNS: list[re.Pattern[str]] = [
    # macOS
    re.compile(rf"/Users/{_USERNAME_SEGMENT}(?:/|\b)"),
    # Linux
    re.compile(rf"/home/{_USERNAME_SEGMENT}(?:/|\b)"),
    # Windows (forward slashes)
    re.compile(rf"[A-Za-z]:/Users/{_USERNAME_SEGMENT}(?:/|\b)"),
    # Windows (backslashes; matches `C:\\Users\\...` and escaped variants like `C:\\\\Users\\\\...`)
    re.compile(rf"[A-Za-z]:\\+Users\\+{_USERNAME_SEGMENT}(?:\\+|\b)"),
]


@dataclass(frozen=True)
class Match:
    path: Path
    line_no: int
    line: str


def _iter_matches(path: Path, patterns: list[re.Pattern[str]]) -> list[Match]:
    try:
        text = path.read_text(encoding="utf-8")
    except UnicodeDecodeError:
        text = path.read_text(encoding="utf-8", errors="replace")

    matches: list[Match] = []
    for i, line in enumerate(text.splitlines(), start=1):
        if any(p.search(line) for p in patterns):
            matches.append(Match(path=path, line_no=i, line=line))
    return matches


def main(argv: list[str]) -> int:
    paths = [Path(p) for p in argv[1:]]

    blocked: list[str] = []
    invalid_pyenv_version_files: list[tuple[str, str]] = []
    offenders: list[Match] = []

    for path in paths:
        path_str = path.as_posix()
        if path_str.startswith("./"):
            path_str = path_str[2:]
        if path_str == LOCAL_ONLY_CONFIG.as_posix():
            blocked.append(str(path))
            continue

        if not path.is_file():
            continue

        if path_str == PYENV_VERSION_FILE.as_posix():
            version = path.read_text(encoding="utf-8", errors="replace").strip().splitlines()[:1]
            if version and not _PYTHON_VERSION_PATTERN.fullmatch(version[0]):
                invalid_pyenv_version_files.append((str(path), version[0]))
                continue

        offenders.extend(_iter_matches(path, ABSOLUTE_HOME_PATTERNS))

    if not blocked and not invalid_pyenv_version_files and not offenders:
        return 0

    print("Repo hygiene check failed:", file=sys.stderr)

    if blocked:
        print("", file=sys.stderr)
        print(
            f"- `{LOCAL_ONLY_CONFIG}` is local-only and must never be committed.",
            file=sys.stderr,
        )
        print("  Unstage/remove it and keep it in your working tree only.", file=sys.stderr)

    if invalid_pyenv_version_files:
        print("", file=sys.stderr)
        print(
            "- `.python-version` looks like a machine-local pyenv env name (commit only a Python version like `3.13.9`):",
            file=sys.stderr,
        )
        for path, value in invalid_pyenv_version_files:
            print(f"  - {path}: {value!r}", file=sys.stderr)
        print(
            "  Put the env name in `config/local-resources.yaml` instead (or keep `.python-version` uncommitted).",
            file=sys.stderr,
        )

    if offenders:
        print("", file=sys.stderr)
        print(
            "- Found machine-local home paths in tracked files (use `$HOME/...` or relative paths):",
            file=sys.stderr,
        )
        for match in offenders:
            snippet = match.line.strip()
            if len(snippet) > 240:
                snippet = f"{snippet[:240]}…"
            print(f"  - {match.path}:{match.line_no}: {snippet}", file=sys.stderr)

    print("", file=sys.stderr)
    print(
        "Tip: put machine-local paths in `config/local-resources.yaml` (gitignored) and keep docs/tests portable.",
        file=sys.stderr,
    )
    return 1


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
