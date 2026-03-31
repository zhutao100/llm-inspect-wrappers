## Code inspection wrappers (`fd-x`, `rg-x`, `sed-x`)

These are thin, best-effort wrappers around `fd`, `rg`, and ranged `sed` reads that:
- append file metadata (`bytes`, `lines`) to help pick scope + decide full-read vs slices
- truncate pathological long lines (JSONL logs, minified files) with content hints
- passthrough to the canonical tool output on unsupported flags / parse errors (**never fail**)

Preferences:
- file search: `fd-x` > `fd` > `find` (in git repos, respects `.gitignore`)
- text search: `rg-x` > `rg` > `grep`
- ranged reads: `sed-x -n 'a,bp' file` > `sed -n 'a,bp' file`

Workflow (capped, scope-first):
1. Preflight overview: `fd-x -t d -d 2 --max-results 200 .` then `fd-x -t f -d 4 --max-results 200 .`
2. Scoped search: `rg-x --color never -m 50 <pattern> src tests` (avoid `.` unless needed)
3. Read efficiently:
   - if `fd-x` reports `lines <= ~400`, read in one shot: `sed-x -n '1,400p' path`
   - otherwise, use tighter slices: `sed-x -n 'START,ENDp' path`
4. When limiting output, prefer tool-native caps (`--max-results`, `-m`) over piping to `head`.
