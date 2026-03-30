## File inspection wrappers

This project has `fd-x`, `rg-x`, `sed-x` available on PATH.
They are drop-in replacements for `fd`, `rg`, `sed` that append file size and
line-count metadata, and truncate huge lines with content hints.

Preferences:
- Use `fd-x` instead of `fd` for file discovery. The trailing file-info block
  tells you each file's byte size and line count — use this to decide whether
  to read a file in one shot vs. ranged reads.
- Use `rg-x` instead of `rg` for content search. Matched files get size/LoC
  annotations so you know what you're about to open.
- Use `sed-x` instead of `sed` for ranged file reads. It will warn you if lines
  are truncated and always shows total file dimensions.
- For files under ~500 lines, prefer reading the whole file in one shot rather
  than multiple ranged reads.
- Before grepping the entire repo, first use `fd-x -e rs -e py -e ts` (etc.) to
  scope which file types exist, then narrow your rg search with `-t` or `-g`.
