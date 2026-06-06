# binary-ensemble

[![PyPI](https://img.shields.io/pypi/v/binary-ensemble.svg)](https://pypi.org/project/binary-ensemble/)
[![Python versions](https://img.shields.io/pypi/pyversions/binary-ensemble.svg)](https://pypi.org/project/binary-ensemble/)
[![Documentation](https://img.shields.io/readthedocs/binary-ensemble.svg)](https://binary-ensemble.readthedocs.io/)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/peterrrock2/binary-ensemble/blob/main/LICENSE)

**Compress, store, and stream massive ensembles of districting plans.**

Redistricting samplers like [GerryChain](https://gerrychain.readthedocs.io)'s ReCom,
ForestReCom, and Sequential Monte Carlo emit millions of plans. Stored as JSONL, a single
ensemble can run to *tens of gigabytes* — most of it redundant. **BEN** (Binary-Ensemble) is
a compression format and toolkit built for exactly this data: it turns those JSONL mountains
into compact binary files you can store, share, and stream sample-by-sample without unpacking
the whole thing.

`binary-ensemble` is the Python interface to the
[binary-ensemble](https://crates.io/crates/binary-ensemble) Rust library.

> A real 100k-plan ensemble on Colorado's ~140k census blocks is **27 GB** as JSONL.
> Reordered by `GEOID20` it compresses to a **~550 MB** BEN stream, and then to a **~6 MB**
> XBEN file — over a **4500× reduction**, fully lossless.

## Install

```bash
pip install binary-ensemble
```

Requires Python 3.11+. Pre-built wheels are available for Linux, macOS, and Windows.

## Quick example

Write an ensemble into one self-describing `.bendl` bundle, then read it back:

```python
from binary_ensemble import BendlEncoder, BendlDecoder

plans = [[1, 1, 2, 2], [1, 2, 2, 2], [1, 1, 1, 2]]

# The stream context finalizes the bundle when it closes.
encoder = BendlEncoder("ensemble.bendl", overwrite=True)
encoder.add_metadata({"sampler": "demo", "seed": 1234})
with encoder.stream("ben") as stream:
    for assignment in plans:
        stream.write(assignment)

# Iterate the assignments straight back out, one at a time.
for assignment in BendlDecoder("ensemble.bendl"):
    print(assignment)
```

Already have JSONL files? Convert whole files in one call:

```python
from binary_ensemble import encode_jsonl_to_ben, encode_ben_to_xben

encode_jsonl_to_ben("plans.jsonl", "plans.ben")   # fast working format
encode_ben_to_xben("plans.ben", "plans.xben")     # smallest, for storage
```

## Documentation

Full docs are at **[binary-ensemble.readthedocs.io](https://binary-ensemble.readthedocs.io/)**:

- [Quickstart](https://binary-ensemble.readthedocs.io/getting-started/quickstart/) — your first ensemble in a few lines.
- [Concepts](https://binary-ensemble.readthedocs.io/concepts/overview/) — dual graphs, the BEN/XBEN/BENDL formats, encoding variants, and the compression levers.
- [How-to guides](https://binary-ensemble.readthedocs.io/how-to/) — compress a GerryChain run, subsample, convert formats, shrink a bundle for sharing.
- [API reference](https://binary-ensemble.readthedocs.io/api/) — every public class and function.

## Command-line tools

The same engine ships as the `ben`, `reben`, `bendl`, and `pcben` CLI tools via Cargo:

```bash
cargo install binary-ensemble
```

## License

MIT — see [LICENSE](https://github.com/peterrrock2/binary-ensemble/blob/main/LICENSE).
