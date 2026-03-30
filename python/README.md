# Python implementation

Single multicall Python script intended to be symlinked as `fd-x`, `rg-x`, and `sed-x`.

## Install

From this directory:

```bash
chmod +x llm_inspect.py
ln -sf llm_inspect.py fd-x
ln -sf llm_inspect.py rg-x
ln -sf llm_inspect.py sed-x
```

Or invoke directly:

```bash
./llm_inspect.py fd-x ...
./llm_inspect.py rg-x ...
./llm_inspect.py sed-x ...
```

## Requirements

- `python3` (3.13+ recommended)
- `fd`, `rg`, `sed` available on `PATH`

## Tests

```bash
python3 -m unittest discover -s tests -q
```
