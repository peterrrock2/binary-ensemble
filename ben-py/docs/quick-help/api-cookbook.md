# API cookbook

Short recipes for common jobs. These examples assume the sample files from
[How-to guides](index.md) exist in the working directory.

## Create a minimal bundle

```python
from binary_ensemble import BendlEncoder

plans = [[1, 1, 2, 2], [1, 2, 2, 2]]

encoder = BendlEncoder("cookbook-minimal.bendl", overwrite=True)
with encoder.ben_stream() as ensemble:
    for plan in plans:
        ensemble.write(plan)
```

## Create a bundle with metadata

```python
from binary_ensemble import BendlEncoder

encoder = BendlEncoder("cookbook-metadata.bendl", overwrite=True)
encoder.add_metadata({"sampler": "demo", "seed": 1234})

with encoder.ben_stream() as ensemble:
    ensemble.write([1, 1, 2, 2])
```

## Create a bundle with a graph

```python
import networkx as nx

from binary_ensemble import BendlEncoder

graph = nx.convert_node_labels_to_integers(nx.path_graph(4))

encoder = BendlEncoder("cookbook-graph.bendl", overwrite=True)
ordered_graph = encoder.add_graph(nx.adjacency_data(graph), sort="rcm")

with encoder.ben_stream() as ensemble:
    ensemble.write([1, 1, 2, 2])

assert ordered_graph.number_of_nodes() == 4
```

## Read assignments

```python
from binary_ensemble import BendlDecoder

decoder = BendlDecoder("ensemble.bendl")

for assignment in decoder:
    print(assignment[:4])
    break
```

## Read graph and metadata

```python
from binary_ensemble import BendlDecoder

decoder = BendlDecoder("ensemble.bendl")

graph = decoder.read_graph()
metadata = decoder.read_metadata()

print(graph.number_of_nodes())
print(metadata["sampler"])
```

## Inspect bundle assets

```python
from binary_ensemble import BendlDecoder

decoder = BendlDecoder("ensemble.bendl")

for asset in decoder.list_assets():
    print(asset["name"], asset["type"], asset["flags"])
```

## Add custom assets

```python
from binary_ensemble import BendlEncoder

encoder = BendlEncoder("cookbook-assets.bendl", overwrite=True)
encoder.add_asset("scores.json", '{"cut_edges": [10, 11]}', content_type="json")
encoder.add_asset("notes.txt", "Created for cookbook example.", content_type="text")

with encoder.ben_stream() as ensemble:
    ensemble.write([1, 1, 2, 2])
```

## Append an asset after finalization

```python
from binary_ensemble import BendlEncoder

encoder = BendlEncoder.append("ensemble.bendl")
encoder.add_asset("cookbook-note.txt", "Added after the run.", content_type="text")
encoder.close()
```

## Subsample every Nth plan

```python
from binary_ensemble import BendlDecoder

for assignment in BendlDecoder("ensemble.bendl").subsample_every(30):
    print(assignment[:4])
```

## Subsample by range

```python
from binary_ensemble import BendlDecoder

window = list(BendlDecoder("ensemble.bendl").subsample_range(10, 15))
assert len(window) == 5
```

## Convert JSONL to BEN and XBEN

```python
from binary_ensemble import encode_ben_to_xben, encode_jsonl_to_ben

encode_jsonl_to_ben("plans.jsonl", "cookbook-plans.ben", overwrite=True)
encode_ben_to_xben("cookbook-plans.ben", "cookbook-plans.xben", overwrite=True)
```

## Decode XBEN back to BEN

```python
from binary_ensemble import decode_xben_to_ben

decode_xben_to_ben("chain.xben", "cookbook-work.ben", overwrite=True)
```

## Extract the stream from a bundle

```python
from binary_ensemble import BendlDecoder

BendlDecoder("ensemble.bendl").extract_stream("cookbook-extracted.ben", overwrite=True)
```

## Relabel and recompress a bundle

```python
from binary_ensemble import compress_stream, relabel_bundle

relabel_bundle("ensemble.bendl", out_file="cookbook-sorted.bendl", sort="mlc")
compress_stream("cookbook-sorted.bendl", out_file="cookbook-archive.bendl")
```

## Reorder a graph directly

```python
import networkx as nx

from binary_ensemble import graph

dual_graph = nx.convert_node_labels_to_integers(nx.path_graph(4))
ordered_graph, permutation_map = graph.reorder(nx.adjacency_data(dual_graph), sort="rcm")

assert ordered_graph.number_of_nodes() == 4
assert "node_permutation_old_to_new" in permutation_map
```
