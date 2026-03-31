## Code inspection tools

Prefer these wrappers over raw `fd`, `rg`, `sed` when inspecting the repo:

- `fd-x` for file discovery
- `rg-x` for symbol/keyword search
- `sed-x -n 'a,bp' file` for ranged reads

Workflow:
1. Start with a scoped overview before repo-root searching:
   - `fd-x -td -d 2 .`
   - `fd-x -tf -d 3 .`
2. Use `rg-x` on likely roots (`src`, `crates`, `packages`, `tests`, etc.) before searching repo root.
3. If a candidate file is <= 400 lines, prefer reading it whole in one shot.
4. Use `sed-x` instead of raw `sed` for ranged reads to avoid dumping pathological long lines.
5. Prefer core-language roots over utility-script roots when the repo is polyglot.
