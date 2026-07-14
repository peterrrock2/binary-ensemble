# binary-ensemble

[![PyPI](https://img.shields.io/pypi/v/binary-ensemble.svg)](https://pypi.org/project/binary-ensemble/)
[![Python versions](https://img.shields.io/pypi/pyversions/binary-ensemble.svg)](https://pypi.org/project/binary-ensemble/)
[![Documentation](https://img.shields.io/readthedocs/binary-ensemble.svg)](https://binary-ensemble.readthedocs.io/)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/peterrrock2/binary-ensemble/blob/main/LICENSE.md)

**Compress, store, and stream massive ensembles of districting plans.**

Redistricting samplers like [GerryChain](https://gerrychain.readthedocs.io)'s ReCom,
ForestReCom, and Sequential Monte Carlo emit millions of plans. Stored as JSONL, a single
ensemble can run to _tens of gigabytes_ — most of it redundant. **BEN** (Binary-Ensemble) is
a compression format and toolkit built for exactly this data: it turns those JSONL mountains
into compact binary files you can store, share, and stream sample-by-sample without unpacking
the whole thing.

`binary-ensemble` is the Python interface to the
[binary-ensemble](https://crates.io/crates/binary-ensemble) Rust library.

> A real 50k-plan ensemble on Colorado's ~140k census blocks is **13.5 GB** as JSONL.
> Reordered by `GEOID20` it compresses to a **~280 MB** BEN stream, and then to a **5.6 MB**
> XBEN file — over a **2,400× reduction**, fully lossless.

## Install

```bash
pip install binary-ensemble
```

Requires Python 3.11+. Pre-built wheels are available for Linux, macOS, and Windows. The
only runtime dependency is NetworkX, and the API is fully type-annotated (`py.typed`).

## Quick example

Write an ensemble into one self-describing `.bendl` file, then read it back:

```python
from binary_ensemble import BendlEncoder, BendlDecoder

plans = [[1, 1, 2, 2], [1, 2, 2, 2], [1, 1, 1, 2]]

# The stream context finalizes the bundle when it closes.
encoder = BendlEncoder("ensemble.bendl", overwrite=True)
encoder.add_metadata({"sampler": "demo", "seed": 1234})
with encoder.ben_stream() as ensemble:
    for assignment in plans:
        ensemble.write(assignment)

# Iterate the assignments straight back out, one at a time.
for assignment in BendlDecoder("ensemble.bendl"):
    print(assignment)
```

## The graph travels with the data

An assignment is just integers — it only means something in a dual graph's node order. A
`.bendl` file embeds the graph, so a collaborator can open one file and reconstruct plans
with no risk of pairing the wrong graph:

```python
import networkx as nx
from binary_ensemble import BendlEncoder, BendlDecoder

dual_graph = nx.convert_node_labels_to_integers(nx.grid_2d_graph(4, 4))

encoder = BendlEncoder("run.bendl", overwrite=True)
ordered = encoder.add_graph(nx.adjacency_data(dual_graph))  # preserves the graph's node order
with encoder.ben_stream() as ensemble:
    for step in range(1000):
        ensemble.write([(node + step) % 4 + 1 for node in range(16)])

decoder = BendlDecoder("run.bendl")
graph = decoder.read_graph()        # back as a live networkx.Graph, in assignment order
print(len(decoder))                 # 1000 — read from the header, no scan

for assignment in decoder.subsample_every(100):
    ...                             # every 100th plan, without decoding the rest
```

## More than the basics

- **Whole-file converters** for existing JSONL ensembles:

  ```python
  from binary_ensemble import encode_jsonl_to_ben, encode_ben_to_xben

  encode_jsonl_to_ben("plans.jsonl", "plans.ben")   # fast working format
  encode_ben_to_xben("plans.ben", "plans.xben")     # smallest, for storage
  ```

- **Shrink for sharing** — reorder a finished file and recompress its stream to XBEN,
  keeping every asset:

  ```python
  from binary_ensemble import relabel_bundle, compress_stream

  relabel_bundle("run.bendl", out_file="run-sorted.bendl", sort="mlc")
  compress_stream("run-sorted.bendl", out_file="run-archive.bendl")
  ```

- **Subsampling** by stride, range, or explicit indices on both bundles and plain
  `.ben`/`.xben` streams — skipped samples are never materialized.
- **Custom assets**: attach scores, notes, run manifests, or arbitrary binary blobs (a
  zipped shapefile, a GeoPackage) alongside the stream. Every asset is checksummed
  (CRC32C), large payloads are xz-compressed transparently, and
  `BendlDecoder.verify()` validates a whole file in one call.
- **Sampler-agnostic**: encoders take plain `list[int]` assignments, so the same API works
  for GerryChain, ForestReCom, SMC, or your own code.

## Documentation

Full docs are at **[binary-ensemble.readthedocs.io](https://binary-ensemble.readthedocs.io/)**:

- [Quickstart](https://binary-ensemble.readthedocs.io/getting-started/quickstart/) — your first ensemble in a few lines.
- [Concepts](https://binary-ensemble.readthedocs.io/concepts/overview/) — dual graphs, the BEN/XBEN/BENDL formats, encoding variants, and the compression levers.
- [Quick Help](https://binary-ensemble.readthedocs.io/quick-help/) — compress a GerryChain run, analyze with NumPy/pandas, subsample, convert formats, shrink a file for sharing, recover a crashed run.
- [API reference](https://binary-ensemble.readthedocs.io/api/) — every public class and function.
- [Tutorial notebooks](https://binary-ensemble.readthedocs.io/user/using_bendl/) — executed end to end in CI against the live API, as is every code snippet in the docs.

## Command-line tools

The same engine ships as the `ben` and `bendl` CLI tools via Cargo:

```bash
cargo install binary-ensemble
```

## License

MIT — see [LICENSE](https://github.com/peterrrock2/binary-ensemble/blob/main/LICENSE.md).
