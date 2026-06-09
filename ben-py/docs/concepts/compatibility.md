# Compatibility and stability

This page covers what is stable at the Python package boundary and what belongs to the
underlying binary format.

## Python package compatibility

`binary-ensemble` requires Python 3.11 or newer and depends on NetworkX at runtime.

```python
import binary_ensemble

assert "BendlEncoder" in binary_ensemble.__all__
```

Pre-built wheels are intended to make normal installation a one-command process:

```bash
pip install binary-ensemble
```

Building from source requires a Rust toolchain and `maturin`; see
[Installation](../getting-started/installation.md).

## Public Python surface

Import from these public modules, or from the top-level `binary_ensemble` namespace:

| Module | Stability expectation |
|---|---|
| `binary_ensemble.bundle` | Public bundle API |
| `binary_ensemble.stream` | Public plain-stream API |
| `binary_ensemble.codec` | Public whole-file conversion API |
| `binary_ensemble.graph` | Public graph-reordering API |

Do not import from `binary_ensemble._core` in application code. It is the compiled extension
implementation detail behind the public modules.

## File-format stability

The byte-level format stability policy lives in the repository-level
[format stability document](https://github.com/peterrrock2/binary-ensemble/blob/main/docs/format-stability.md).
At the Python level, the important rule is simpler: readers auto-detect stream variants, and
bundle readers expose the bundle version through `BendlDecoder.version()`.

```python
from binary_ensemble import BendlDecoder

decoder = BendlDecoder("ensemble.bendl")
print(decoder.version())
print(decoder.assignment_format())
```

## Reading older files

When opening an existing file, start with inspection:

```python
from binary_ensemble import BendlDecoder

decoder = BendlDecoder("ensemble.bendl")

print(decoder.is_complete())
print(decoder.version())
print(decoder.asset_names())
```

For plain streams, use `BenDecoder(path)` for `.ben` and `BenDecoder(path, mode="xben")` for
`.xben`.

## Reproducibility metadata

The binary format stores assignments losslessly. Reproducing an analysis also requires the
context around the stream. For serious runs, store at least:

- package versions,
- sampler name and parameters,
- random seed,
- graph source and hash,
- node-order choice,
- scoring definitions,
- creation date and operator notes.

```python
from binary_ensemble import BendlEncoder

encoder = BendlEncoder("compatibility.bendl", overwrite=True)
encoder.add_metadata(
    {
        "sampler": "ReCom",
        "seed": 1234,
        "node_order": "GEOID20",
        "binary_ensemble": "record the package version here",
    }
)
with encoder.stream("ben") as stream:
    stream.write([1, 1, 2, 2])
```

