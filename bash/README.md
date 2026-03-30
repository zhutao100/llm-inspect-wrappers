# Bash implementation

Single multicall script intended to be symlinked as `fd-x`, `rg-x`, and `sed-x`.

## Install

From this directory:

```bash
chmod +x xwrap
ln -sf xwrap fd-x
ln -sf xwrap rg-x
ln -sf xwrap sed-x
```

Or invoke directly:

```bash
./xwrap fd-x ...
./xwrap rg-x ...
./xwrap sed-x ...
```

## Requirements

- `bash`
- `fd`, `rg`, `sed` available on `PATH`

## Tests

There is no required Bash-native test framework. The test suite is written in Python:

```bash
python3 -m unittest discover -s tests -q
```
