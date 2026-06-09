# Performance guide

`binary-ensemble` has two very different performance profiles:

- BEN is fast to write and read. Use it while sampling, inspecting, and subsampling.
- XBEN is much smaller but slower to create. Use it for archival and transfer.

## Format trade-offs

| Format | Write speed | Read speed | Size | Best use |
|---|---|---|---|---|
| JSONL | slowest and largest | simple but bulky | largest | Interchange and debugging |
| BEN | fast | fast | small | Day-to-day work |
| XBEN | slow to create | fast after startup | smallest | Archive and sharing |
| BENDL + BEN | fast | fast | small plus assets | Recommended working file |
| BENDL + XBEN | slow to create | fast after startup | smallest plus assets | Recommended archive file |

## The biggest lever: node order

Run-length encoding rewards long stretches of the same district id. A better graph node order
creates longer runs in every assignment.

```python
import networkx as nx

from binary_ensemble import graph

dual_graph = nx.convert_node_labels_to_integers(nx.grid_2d_graph(8, 8))
for node in dual_graph.nodes:
    dual_graph.nodes[node]["GEOID20"] = f"{node:04d}"

ordered_graph, permutation_map = graph.reorder(
    nx.adjacency_data(dual_graph),
    sort="key",
    key="GEOID20",
)

assert ordered_graph.number_of_nodes() == 64
assert "node_permutation_old_to_new" in permutation_map
```

For real Census block graphs, a geographic key such as `GEOID20` is often the best first
try. Without a meaningful key, use `sort="mlc"` or `sort="rcm"`.

## The second lever: XBEN recompression

XBEN runs LZMA2 over an XZ representation of the BEN stream. That exploits repetition across
plans and can reduce large block-level ensembles by orders of magnitude.

```python
from binary_ensemble import compress_stream

compress_stream("ensemble.bendl", out_file="performance-archive.bendl")
```

Expect XBEN compression to be asymmetric: slow to create, much faster to read. On large
ensembles, create the XBEN file once and keep a BEN working copy if you will iterate often.

## Tuning XBEN

The plain-stream XBEN encoders expose tuning options:

```python
from binary_ensemble import encode_ben_to_xben

encode_ben_to_xben(
    "chain.ben",
    "performance.xben",
    overwrite=True,
    n_threads=4,
    compression_level=6,
    xz_block_size=None,
)
```

Guidance:

| Option | Effect | Practical default |
|---|---|---|
| `n_threads` | Parallelizes compression work | `None` to use the library default |
| `compression_level` | Higher is smaller but slower | `9` for final archive, lower for iteration |
| `xz_block_size` | Controls XZ block sizing | `None` unless benchmarking a specific workload |

`compress_stream()` for bundles uses the library's bundle recompression defaults. If you
need fine control over XBEN tuning, extract the stream, tune the plain-stream conversion, and
package deliberately.

## Subsampling

Subsampling is designed to avoid decoding unneeded samples.

```python
from binary_ensemble import BendlDecoder

for assignment in BendlDecoder("ensemble.bendl").subsample_every(25):
    print(assignment[:4])
```

BEN streams are cheapest to subsample. XBEN streams pay a decompression startup cost, then
can still skip through the decoded stream efficiently.

## Practical workflow

For serious runs:

1. Reorder the graph before or during bundle creation.
2. Write a BEN bundle while sampling.
3. Attach metadata, graph, and provenance assets.
4. Use BEN for quality checks and analysis.
5. Relabel/reorder the final bundle if needed.
6. Recompress to XBEN for sharing.
