## Code inspection wrappers (`fd-x`, `rg-x`, `sed-x`)

Prefer these wrappers over raw `fd`, `rg`, `sed` when inspecting repos:

- `fd-x` for file discovery with size/line metadata
- `rg-x` for scoped search with match grouping and long-line gating
- `sed-x -n 'a,bp' file` for ranged reads with long-line gating

Workflow:
1. Preflight before repo-root search:
   - `fd-x -td -d 2 .`
   - `fd-x -tf -d 4 .`
2. Prefer scoped search:
   - `rg-x <pattern> src tests` (avoid `.` unless needed)
3. Prefer full reads for small files (≈400 lines or less).
4. Use `sed-x` for ranged reads to avoid dumping pathological long lines.
