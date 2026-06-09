# Examples gallery

Small standalone patterns you can paste into scripts. For longer explanations, follow the
links from each example.

## Minimal bundle

```python
from binary_ensemble import BendlEncoder, BendlDecoder

encoder = BendlEncoder("gallery-minimal.bendl", overwrite=True)
with encoder.stream("ben") as stream:
    stream.write([1, 1, 2, 2])
    stream.write([1, 2, 2, 2])

assert len(BendlDecoder("gallery-minimal.bendl")) == 2
```

See [Quickstart](../getting-started/quickstart.md).

## Bundle with graph, metadata, and notes

```python
import networkx as nx

from binary_ensemble import BendlDecoder, BendlEncoder

graph = nx.convert_node_labels_to_integers(nx.path_graph(4))

encoder = BendlEncoder("gallery-rich.bendl", overwrite=True)
encoder.add_graph(nx.adjacency_data(graph), sort=None)
encoder.add_metadata({"seed": 2026, "sampler": "demo"})
encoder.add_asset("notes.txt", "Toy gallery bundle.", content_type="text")

with encoder.stream("ben") as stream:
    stream.write([1, 1, 2, 2])

decoder = BendlDecoder("gallery-rich.bendl")
assert decoder.read_graph().number_of_nodes() == 4
assert decoder.read_metadata()["seed"] == 2026
```

See [Custom assets and appending](custom-assets-and-append.md).

## Plain stream conversion

```python
from binary_ensemble import decode_xben_to_ben, encode_ben_to_xben

encode_ben_to_xben("chain.ben", "gallery-chain.xben", overwrite=True)
decode_xben_to_ben("gallery-chain.xben", "gallery-chain.ben", overwrite=True)
```

See [Convert between formats](convert-formats.md).

## Subsample for diagnostics

```python
from binary_ensemble import BendlDecoder

diagnostic_plans = list(BendlDecoder("ensemble.bendl").subsample_every(40))
assert len(diagnostic_plans) > 0
```

See [Subsample a large ensemble](subsample.md).

## Archive a final bundle

```python
from binary_ensemble import compress_stream, relabel_bundle

relabel_bundle("ensemble.bendl", out_file="gallery-sorted.bendl", sort="mlc")
compress_stream("gallery-sorted.bendl", out_file="gallery-archive.bendl")
```

See [Shrink a bundle for sharing](shrink-for-sharing.md).
