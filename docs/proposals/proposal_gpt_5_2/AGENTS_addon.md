1. **Preflight before repo-root search**

   * Run `fd-x --overview=2 -tf .` (or just `fd-x -tf -d 4 .` in small repos)
   * Use the extension histogram / dir rollups to pick a scope (`src/`, `crates/`, `tests/`, etc.)

2. **Prefer scoped search**

   * `rg-x <pattern> src tests` (avoid `.` unless needed)
   * If results are large, tighten with globs (`-g '*.rs'`) and limits (`--max-total-matches`)

3. **Never use raw `sed` for “peek ranges”**

   * Use `sed-x -n 'START,ENDp' FILE` so pathological long lines are truncated safely

4. **When you need full-file reads**

   * Use `fd-x` facts first; if `loc <= 400` and `max <= 400`, it is usually cheaper to read in one shot (your existing “size-gated reading” principle).
