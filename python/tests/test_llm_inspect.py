from __future__ import annotations

import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
SCRIPT = REPO_ROOT / "python" / "llm_inspect.py"


def have_tools(*names: str) -> bool:
    return all(shutil.which(n) for n in names)


def run_wrapped(
    *args: str,
    cwd: Path,
    env: dict[str, str] | None = None,
    stdin: str = "",
) -> subprocess.CompletedProcess[str]:
    merged_env = None
    if env is not None:
        merged_env = dict(os.environ)
        merged_env.update(env)
    cp = subprocess.run(
        [sys.executable, str(SCRIPT), *args],
        cwd=str(cwd),
        env=merged_env,
        input=stdin,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    return cp


def parse_kv_fields(fields: list[str]) -> dict[str, str]:
    out: dict[str, str] = {}
    for f in fields:
        if "=" not in f:
            continue
        k, v = f.split("=", 1)
        out[k] = v
    return out


class TestLlmInspectPython(unittest.TestCase):
    @unittest.skipUnless(have_tools("fd", "rg"), "requires fd + rg on PATH")
    def test_fd_x_emits_file_table(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            (root / "dir").mkdir()
            (root / "a.txt").write_text("a\nb\nc\n", encoding="utf-8")
            (root / "dir" / "b.txt").write_text("hello\n", encoding="utf-8")

            cp = run_wrapped("fd-x", "-tf", cwd=root)
            self.assertEqual(cp.returncode, 0, cp.stderr)

            lines = [ln for ln in cp.stdout.splitlines() if ln.strip()]
            self.assertTrue(any(ln.startswith("@meta\ttool=fd-x\t") for ln in lines))

            rows = [ln for ln in lines if not ln.startswith("@meta\t")]
            self.assertGreaterEqual(len(rows), 2)

            by_path: dict[str, dict[str, str]] = {}
            for row in rows:
                cols = row.split("\t")
                by_path[cols[0]] = parse_kv_fields(cols[1:])

            for rel in ("a.txt", "dir/b.txt"):
                self.assertIn(rel, by_path)
                meta = by_path[rel]
                self.assertEqual(meta.get("kind", "file"), "file")
                st = os.stat(root / rel)
                self.assertEqual(int(meta["bytes"]), st.st_size)
                self.assertEqual(int(meta["lines"]), (root / rel).read_text(encoding="utf-8").count("\n"))

    @unittest.skipUnless(have_tools("rg"), "requires rg on PATH")
    def test_rg_x_groups_and_gates(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            (root / "short.txt").write_text("needle here\n", encoding="utf-8")

            long_line = '{"k":"' + ("x" * 3000) + '","needle":"y"}\n'
            (root / "long.json").write_text(long_line, encoding="utf-8")

            # Ensure wrapper behavior is deterministic even if the environment enables a ripgrep rc
            # via `RIPGREP_CONFIG_PATH` (e.g. one containing `--quiet`).
            rg_conf = root / "ripgrep.conf"
            rg_conf.write_text("--quiet\n", encoding="utf-8")

            cp = run_wrapped("rg-x", "needle", ".", cwd=root, env={"RIPGREP_CONFIG_PATH": str(rg_conf)})
            self.assertIn(cp.returncode, (0, 1), cp.stderr)

            lines = [ln for ln in cp.stdout.splitlines() if ln.strip()]
            file_headers = [ln for ln in lines if ln.startswith("@file\t")]
            self.assertEqual({h.split("\t")[1] for h in file_headers}, {"path=long.json", "path=short.txt"})

            long_match = next((ln for ln in lines if ln.startswith("1:") and "rg-x truncated" in ln), None)
            self.assertIsNotNone(long_match)

            short_match = next((ln for ln in lines if ln.startswith("1:") and "needle here" in ln), None)
            self.assertIsNotNone(short_match)

            self.assertTrue(any(ln.startswith("@meta\ttool=rg-x\tmode=match\t") for ln in lines))

    @unittest.skipUnless(have_tools("rg"), "requires rg on PATH")
    def test_rg_x_no_match_emits_empty_stdout(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            (root / "hay.txt").write_text("hay here\n", encoding="utf-8")

            cp = run_wrapped("rg-x", "needle", ".", cwd=root)
            self.assertEqual(cp.returncode, 1, cp.stderr)
            self.assertEqual(cp.stdout, "")

    @unittest.skipUnless(have_tools("rg"), "requires rg on PATH")
    def test_rg_x_reads_stdin_when_piped(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            cp = run_wrapped("rg-x", "needle", cwd=root, stdin="hay\nneedle here\n")
            self.assertEqual(cp.returncode, 0, cp.stderr)
            self.assertIn("@file\tpath=<stdin>", cp.stdout)
            self.assertIn("2:1:needle here", cp.stdout)

    def test_sed_x_range_gates_long_line(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            (root / "big.txt").write_text(("x" * 3000) + "\n", encoding="utf-8")

            cp = run_wrapped("sed-x", "-n", "1,1p", "big.txt", cwd=root)
            self.assertEqual(cp.returncode, 0, cp.stderr)

            self.assertIn("sed-x truncated line=1", cp.stdout)
            self.assertIn("@meta\ttool=sed-x\tpath=big.txt", cp.stdout)
