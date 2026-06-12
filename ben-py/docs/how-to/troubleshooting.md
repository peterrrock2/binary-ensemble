# Troubleshooting

Most `binary-ensemble` failures come from one of three sources: the wrong container, an
unfinished bundle, or a mismatch between assignment order and graph order. This guide gives
the quickest checks before you dig into a large run.

## I opened a file with the wrong reader

Use the reader that matches the container:

| File | Reader |
|---|---|
| `.bendl` | `BendlDecoder` |
| `.ben` | `BenDecoder(path)` |
| `.xben` | `BenDecoder(path, mode="xben")` |

```python
from binary_ensemble import BendlDecoder, BenDecoder

bundle = BendlDecoder("ensemble.bendl")
plain_ben = BenDecoder("chain.ben")
plain_xben = BenDecoder("chain.xben", mode="xben")

assert bundle.assignment_format() in {"ben", "xben"}
assert plain_ben.assignment_format() == "ben"
assert plain_xben.assignment_format() == "xben"
```

If you want the raw stream from a bundle, extract it:

```python
from binary_ensemble import BendlDecoder

BendlDecoder("ensemble.bendl").extract_stream("extracted.ben", overwrite=True)
```

## My bundle is incomplete

A bundle is finalized when the stream context closes cleanly. If the process exits while
writing, the file may contain readable stream bytes but the header remains marked incomplete.

```python
from binary_ensemble import BendlDecoder

decoder = BendlDecoder("ensemble.bendl")
print(decoder.is_complete())
```

Use context managers around stream writes so finalization happens at the right time:

```python
from binary_ensemble import BendlEncoder

encoder = BendlEncoder("new.bendl", overwrite=True)
with encoder.stream("ben") as stream:
    stream.write([1, 1, 2, 2])
```

For an assets-only bundle, use the encoder itself as the context manager or call `close()`:

```python
from binary_ensemble import BendlEncoder

with BendlEncoder("assets-only.bendl", overwrite=True) as encoder:
    encoder.add_metadata({"kind": "asset index"})
```

### Recovering samples from a crashed run

An unfinalized bundle is not a write-off. The stream bytes that reached disk before the
crash are still there; only the last frame may be cut off mid-write, and the asset
directory (written at finalization) is lost. `extract_stream(allow_unfinalized=True)`
copies the partial stream out, and a salvage loop keeps every sample up to the truncated
tail:

```python
from binary_ensemble import BendlDecoder, BenDecoder, BendlEncoder

# allow_unfinalized=True permits extraction even though the stream checksum
# was never written. (On a finalized bundle the flag is harmless.)
BendlDecoder("ensemble.bendl").extract_stream(
    "recovered.ben", overwrite=True, allow_unfinalized=True
)

# Keep every intact sample; stop at the truncated tail frame, if any.
recovered = []
stream = iter(BenDecoder("recovered.ben"))
while True:
    try:
        recovered.append(next(stream))
    except StopIteration:
        break          # clean end of stream
    except Exception:
        break          # truncated tail frame from the crash

# Re-encode the salvaged samples into a fresh, finalized bundle.
encoder = BendlEncoder("recovered.bendl", overwrite=True)
with encoder.stream("ben") as out:
    for assignment in recovered:
        out.write(assignment)
```

Two things to know about what survives a crash:

- **Assets do not.** The bundle's directory is committed at finalization, so
  `asset_names()` on a crashed bundle is empty even if you called `add_graph()` or
  `add_metadata()` before streaming. Re-attach the graph and metadata to the recovered
  bundle from their original sources.
- **`len()` and iteration on the crashed bundle itself raise** (the truncated tail frame
  breaks the sample count), which is why the recipe extracts first and salvages from the
  plain stream. For ensembles too large to buffer in a list, open the output stream first
  and write each salvaged sample as it is decoded.

## The assignments decode, but the maps look wrong

This is almost always a node-order problem. Decoding can only recover the integer vectors
that were written; it cannot prove that those vectors line up with the intended geography.

Check the basics:

```python
from binary_ensemble import BendlDecoder

decoder = BendlDecoder("ensemble.bendl")
graph = decoder.read_graph()
assignment = next(iter(decoder))

assert graph is not None
assert len(assignment) == graph.number_of_nodes()
```

If the lengths match but the maps still look wrong, confirm that the sampler wrote
assignments in the same node order as `list(graph.nodes)` from the embedded graph. When in
doubt, rebuild a tiny known assignment, write it, and read it back before launching the full
run.

## `read_graph()` returns `None`

The bundle does not contain `graph.json`. Plain `.ben` and `.xben` streams never contain a
graph, and a `.bendl` bundle only contains one if the writer called `add_graph()`.

```python
from binary_ensemble import BendlDecoder

decoder = BendlDecoder("ensemble.bendl")
print(decoder.asset_names())
```

For future runs, attach the graph before or during bundle creation:

```python
import networkx as nx

from binary_ensemble import BendlEncoder

graph = nx.convert_node_labels_to_integers(nx.path_graph(4))

encoder = BendlEncoder("with-graph.bendl", overwrite=True)
encoder.add_graph(nx.adjacency_data(graph), sort=None)
with encoder.stream("ben") as stream:
    stream.write([1, 1, 2, 2])
```

## Recompression or relabeling refuses my arguments

`compress_stream()` and `relabel_bundle()` need exactly one output mode:

```python
from binary_ensemble import compress_stream, relabel_bundle

compress_stream("ensemble.bendl", out_file="ensemble-archive.bendl")
relabel_bundle("ensemble.bendl", out_file="ensemble-sorted.bendl", sort="mlc")
```

or:

```python
from binary_ensemble import compress_stream, relabel_bundle

relabel_bundle("ensemble.bendl", in_place=True, sort="mlc")
compress_stream("ensemble.bendl", in_place=True)
```

Passing both `out_file` and `in_place=True`, or passing neither, raises `ValueError`. Relabel
before recompressing to XBEN; relabeling needs a BEN stream and an embedded graph.

## XBEN compression is slow

That is expected. XBEN uses high-ratio LZMA2 compression and is meant for archival or
transfer. Work against BEN while sampling, reading, and subsampling; recompress to XBEN once
the bundle is ready to share.

```python
from binary_ensemble import BendlEncoder, compress_stream

encoder = BendlEncoder("to-archive.bendl", overwrite=True)
with encoder.stream("ben") as stream:
    stream.write([1, 1, 2, 2])

compress_stream("to-archive.bendl", out_file="archive-copy.bendl")
```

If you need to repeatedly subsample a plain `.xben` stream, decode it back to `.ben` once:

```python
from binary_ensemble import decode_xben_to_ben

decode_xben_to_ben("chain.xben", "chain.work.ben", overwrite=True)
```
