from __future__ import annotations

import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from dataclasses import dataclass
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]
BASH = "/bin/bash" if Path("/bin/bash").exists() else "bash"


def have_tools(*names: str) -> bool:
    return all(shutil.which(n) for n in names)


def parse_kv_fields(fields: list[str]) -> dict[str, str]:
    out: dict[str, str] = {}
    for f in fields:
        if "=" not in f:
            continue
        k, v = f.split("=", 1)
        out[k] = v
    return out


def parse_file_table(stdout: str) -> tuple[dict[str, dict[str, str]], dict[str, str]]:
    rows: dict[str, dict[str, str]] = {}
    meta: dict[str, str] = {}

    for line in [ln for ln in stdout.splitlines() if ln.strip()]:
        if line.startswith("@meta\t"):
            meta = parse_kv_fields(line.split("\t")[1:])
            continue
        cols = line.split("\t")
        if not cols:
            continue
        path = cols[0]
        rows[path] = parse_kv_fields(cols[1:])

    return rows, meta


@dataclass(frozen=True)
class RgMatch:
    line: int
    col: int
    body: str


def parse_rg(stdout: str) -> tuple[dict[str, dict[str, str]], dict[str, list[RgMatch]], dict[str, str]]:
    headers: dict[str, dict[str, str]] = {}
    matches: dict[str, list[RgMatch]] = {}
    meta: dict[str, str] = {}

    current_path: str | None = None
    for line in [ln for ln in stdout.splitlines() if ln.strip()]:
        if line.startswith("@file\t"):
            fields = parse_kv_fields(line.split("\t")[1:])
            current_path = fields.get("path")
            if current_path is None:
                continue
            headers[current_path] = fields
            matches.setdefault(current_path, [])
            continue
        if line.startswith("@meta\t"):
            meta = parse_kv_fields(line.split("\t")[1:])
            continue

        if current_path is None:
            continue
        try:
            line_s, col_s, body = line.split(":", 2)
            matches[current_path].append(RgMatch(line=int(line_s), col=int(col_s), body=body))
        except ValueError:
            continue

    return headers, matches, meta


@dataclass(frozen=True)
class Impl:
    name: str
    argv_prefix: list[str]

    def run(
        self,
        *args: str,
        cwd: Path,
        env: dict[str, str] | None = None,
        stdin: str = "",
    ) -> subprocess.CompletedProcess[str]:
        merged_env = None
        if env is not None:
            merged_env = dict(os.environ)
            merged_env.update(env)
        return subprocess.run(
            [*self.argv_prefix, *args],
            cwd=str(cwd),
            env=merged_env,
            input=stdin,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )


class TestCrossValidateImplementations(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        if not have_tools("fd", "rg", "sed"):
            raise unittest.SkipTest("requires fd + rg + sed on PATH")

        cargo = shutil.which("cargo")
        if cargo is None:
            raise unittest.SkipTest("requires cargo to build Rust implementation")

        subprocess.run([cargo, "build", "-q"], cwd=str(REPO_ROOT / "rust"), check=True)

        cls.impls = [
            Impl("bash", [BASH, str(REPO_ROOT / "bash" / "xwrap")]),
            Impl("python", [sys.executable, str(REPO_ROOT / "python" / "llm_inspect.py")]),
            Impl("rust", [str(REPO_ROOT / "rust" / "target" / "debug" / "llm-inspect-wrappers")]),
        ]

    def test_fd_x_file_table_consistent(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            (root / "dir").mkdir()
            (root / "short.txt").write_text("needle here\n", encoding="utf-8")
            (root / "long.json").write_text('{"k":"' + ("x" * 3000) + '","needle":"y"}\n', encoding="utf-8")
            (root / "dir" / "nested.txt").write_text("nested\n", encoding="utf-8")

            results: dict[str, dict[str, dict[str, str]]] = {}
            for impl in self.impls:
                cp = impl.run("fd-x", "-tf", cwd=root)
                self.assertEqual(cp.returncode, 0, f"{impl.name} stderr:\n{cp.stderr}")
                rows, meta = parse_file_table(cp.stdout)
                self.assertEqual(meta.get("tool"), "fd-x")
                results[impl.name] = rows

            expected_files = {"short.txt", "long.json", "dir/nested.txt"}
            for impl in self.impls:
                self.assertTrue(expected_files.issubset(results[impl.name].keys()))

            for rel in expected_files:
                st = os.stat(root / rel)
                expected_bytes = st.st_size
                expected_lines = (root / rel).read_text(encoding="utf-8").count("\n")
                for impl in self.impls:
                    row = results[impl.name][rel]
                    self.assertEqual(row.get("kind", "file"), "file")
                    self.assertEqual(int(row["bytes"]), expected_bytes)
                    self.assertEqual(int(row["lines"]), expected_lines)

            # Color flags should not introduce ANSI escapes or break path parsing.
            for impl in self.impls:
                cp = impl.run("fd-x", "--color=always", "short", cwd=root)
                self.assertEqual(cp.returncode, 0, f"{impl.name} stderr:\n{cp.stderr}")
                self.assertNotIn("\x1b", cp.stdout)
                rows, meta = parse_file_table(cp.stdout)
                self.assertEqual(meta.get("tool"), "fd-x")
                self.assertIn("short.txt", rows)

    def test_fd_x_dir_rows_are_path_only(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            (root / "d1").mkdir()
            (root / "d2").mkdir()
            (root / "d2" / "nested").mkdir()

            for impl in self.impls:
                cp = impl.run("fd-x", "-td", cwd=root)
                self.assertEqual(cp.returncode, 0, f"{impl.name} stderr:\n{cp.stderr}")
                rows, meta = parse_file_table(cp.stdout)
                self.assertEqual(meta.get("tool"), "fd-x")
                self.assertGreaterEqual(len(rows), 2)
                for fields in rows.values():
                    self.assertEqual(fields, {})

    def test_fd_x_help_is_passthrough(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            for impl in self.impls:
                cp = impl.run("fd-x", "--help", cwd=root)
                self.assertEqual(cp.returncode, 0, f"{impl.name} stderr:\n{cp.stderr}")
                self.assertGreaterEqual(len(cp.stdout.splitlines()), 10)
                self.assertNotIn("@meta\ttool=fd-x", cp.stdout)
                self.assertNotIn("\\n", cp.stdout)

    def test_rg_x_match_mode_consistent(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            (root / "short.txt").write_text("needle here\n", encoding="utf-8")
            (root / "long.json").write_text('{"k":"' + ("x" * 3000) + '","needle":"y"}\n', encoding="utf-8")

            rg_conf = root / "ripgrep.conf"
            rg_conf.write_text("--quiet\n", encoding="utf-8")
            env = {"RIPGREP_CONFIG_PATH": str(rg_conf)}

            parsed: dict[str, tuple[dict[str, dict[str, str]], dict[str, list[RgMatch]], dict[str, str]]] = {}
            for impl in self.impls:
                cp = impl.run("rg-x", "needle", cwd=root, env=env)
                self.assertEqual(cp.returncode, 0, f"{impl.name} stderr:\n{cp.stderr}")
                parsed[impl.name] = parse_rg(cp.stdout)

            expected_paths = {"long.json", "short.txt"}
            for impl in self.impls:
                headers, matches, meta = parsed[impl.name]
                self.assertEqual(meta.get("tool"), "rg-x")
                self.assertEqual(meta.get("mode"), "match")
                self.assertEqual(set(headers.keys()), expected_paths)
                self.assertEqual(set(matches.keys()), expected_paths)

                self.assertGreaterEqual(len(matches["short.txt"]), 1)
                self.assertGreaterEqual(len(matches["long.json"]), 1)

            # Cross-impl: line/col should agree for the single-line fixtures.
            short_cols = {parsed[impl.name][1]["short.txt"][0].col for impl in self.impls}
            long_cols = {parsed[impl.name][1]["long.json"][0].col for impl in self.impls}
            self.assertEqual(len(short_cols), 1)
            self.assertEqual(len(long_cols), 1)

            for impl in self.impls:
                _, matches, _ = parsed[impl.name]
                sm = matches["short.txt"][0]
                lm = matches["long.json"][0]

                self.assertEqual(sm.line, 1)
                self.assertIn("needle here", sm.body)
                self.assertNotIn("rg-x truncated", sm.body)

                self.assertEqual(lm.line, 1)
                self.assertIn("rg-x truncated", lm.body)

    def test_rg_x_filelist_mode_consistent(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            (root / "dir").mkdir()
            (root / "a.txt").write_text("needle\n", encoding="utf-8")
            (root / "b.txt").write_text("nope\n", encoding="utf-8")
            (root / "dir" / "c.txt").write_text("needle here\n", encoding="utf-8")

            rg_conf = root / "ripgrep.conf"
            rg_conf.write_text("--quiet\n", encoding="utf-8")
            env = {"RIPGREP_CONFIG_PATH": str(rg_conf)}

            expected = {"a.txt", "dir/c.txt"}
            for impl in self.impls:
                cp = impl.run("rg-x", "-l", "needle", cwd=root, env=env)
                self.assertEqual(cp.returncode, 0, f"{impl.name} stderr:\n{cp.stderr}")
                rows, meta = parse_file_table(cp.stdout)
                self.assertEqual(meta.get("tool"), "rg-x")
                self.assertEqual(meta.get("mode"), "filelist")
                self.assertTrue(expected.issubset(rows.keys()))

            # Color flags should not introduce ANSI escapes or break NUL parsing.
            for impl in self.impls:
                cp = impl.run("rg-x", "--color=always", "-l", "needle", cwd=root, env=env)
                self.assertEqual(cp.returncode, 0, f"{impl.name} stderr:\n{cp.stderr}")
                self.assertNotIn("\x1b", cp.stdout)
                rows, meta = parse_file_table(cp.stdout)
                self.assertEqual(meta.get("tool"), "rg-x")
                self.assertEqual(meta.get("mode"), "filelist")
                self.assertTrue(expected.issubset(rows.keys()))

            for rel in expected:
                st = os.stat(root / rel)
                expected_bytes = st.st_size
                expected_lines = (root / rel).read_text(encoding="utf-8").count("\n")
                for impl in self.impls:
                    cp = impl.run("rg-x", "-l", "needle", cwd=root, env=env)
                    rows, _ = parse_file_table(cp.stdout)
                    row = rows[rel]
                    self.assertEqual(row.get("kind", "file"), "file")
                    self.assertEqual(int(row["bytes"]), expected_bytes)
                    self.assertEqual(int(row["lines"]), expected_lines)

    def test_sed_x_range_read_consistent(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            (root / "long.json").write_text('{"k":"' + ("x" * 3000) + '","needle":"y"}\n', encoding="utf-8")

            for impl in self.impls:
                cp = impl.run("sed-x", "-n", "1,1p", "long.json", cwd=root)
                self.assertEqual(cp.returncode, 0, f"{impl.name} stderr:\n{cp.stderr}")
                self.assertIn("sed-x truncated line=1", cp.stdout)
                self.assertIn("@meta\ttool=sed-x\tpath=long.json", cp.stdout)

    def test_sed_x_stdin_range_read_consistent(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            long_line = ("x" * 3000) + "\n"

            for impl in self.impls:
                cp = impl.run("sed-x", "-n", "1,1p", cwd=root, stdin=long_line)
                self.assertEqual(cp.returncode, 0, f"{impl.name} stderr:\n{cp.stderr}")
                self.assertIn("sed-x truncated line=1", cp.stdout)
                self.assertIn("@meta\ttool=sed-x\tsource=stdin", cp.stdout)
                self.assertIn("printed_lines=1", cp.stdout)
                self.assertIn("truncated_lines=1", cp.stdout)
