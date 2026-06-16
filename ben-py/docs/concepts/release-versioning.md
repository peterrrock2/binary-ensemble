# Release and versioning

This page describes the promises users should rely on at release boundaries.

## Python package versions

The Python package version is the normal package version installed by `pip`, exposed as
`binary_ensemble.__version__`. Use it to record which Python bindings wrote or read a
bundle.

```python
import binary_ensemble

print(binary_ensemble.__version__)
```

For reproducible runs, store that value in bundle metadata alongside sampler settings, graph
source, and random seed.

## Public API compatibility

The supported Python API is:

- `binary_ensemble.bundle`
- `binary_ensemble.stream`
- `binary_ensemble.codec`
- `binary_ensemble.graph`
- the same symbols re-exported from top-level `binary_ensemble`

`binary_ensemble._core` is an implementation detail. It may change to support the public
wrappers.

## File format compatibility

BEN, XBEN, and BENDL format stability is governed by the repository-level
[format stability document](https://github.com/peterrrock2/binary-ensemble/blob/1.0.0/docs/format-stability.md).
At the Python level, readers expose the bundle version and assignment stream format:

```python
from binary_ensemble import BendlDecoder

decoder = BendlDecoder("ensemble.bendl")
print(decoder.version())
print(decoder.assignment_format())
```

## How to report a compatibility problem

When a file does not open, report:

- the package version,
- the file type (`.ben`, `.xben`, or `.bendl`),
- the bundle version from `BendlDecoder.version()` if it opens far enough,
- whether `BendlDecoder.is_complete()` is true,
- the exact exception text,
- whether the file was produced by Python API or CLI.

If the file can be shared, include the smallest reproducing file. If it cannot be shared,
include the output of:

```python
from binary_ensemble import BendlDecoder

decoder = BendlDecoder("ensemble.bendl")
print(decoder.version())
print(decoder.assignment_format())
print(decoder.is_complete())
print(decoder.list_assets())
```

Do not include confidential geography or plans in public issues unless they are already
public data.
