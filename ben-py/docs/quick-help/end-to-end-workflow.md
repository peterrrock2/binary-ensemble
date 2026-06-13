# End-to-end workflow

This tutorial follows the recommended lifecycle:

1. prepare a graph,
2. write a `.bendl` file with a BEN stream while producing assignments,
3. inspect and analyze the bundle,
4. add provenance,
5. relabel and recompress for sharing.

The code uses a tiny NetworkX grid so it runs anywhere. The same structure applies to a
GerryChain run.

## Prepare the graph

```python
import networkx as nx

SIDE = 4
dual_graph = nx.convert_node_labels_to_integers(nx.grid_2d_graph(SIDE, SIDE))

for node in dual_graph.nodes:
    row, col = divmod(node, SIDE)
    dual_graph.nodes[node]["TOTPOP"] = 1
    dual_graph.nodes[node]["GEOID20"] = f"{row:02d}{col:02d}"

adjacency = nx.adjacency_data(dual_graph)
```

## Write the working bundle

`add_graph()` returns the graph in the order assignments should use. In this toy example the
assignment generator already uses integer node positions, so we only need the node count.

```python
from binary_ensemble import BendlEncoder

encoder = BendlEncoder("workflow.bendl", overwrite=True)
ordered_graph = encoder.add_graph(adjacency, sort="key", key="GEOID20")
encoder.add_metadata({"sampler": "toy-grid", "seed": 2026, "node_order": "GEOID20"})

node_count = ordered_graph.number_of_nodes()

with encoder.ben_stream(variant="twodelta") as ensemble:
    for step in range(20):
        assignment = [(node + step) % 4 + 1 for node in range(node_count)]
        ensemble.write(assignment)
```

## Inspect the result

```python
from binary_ensemble import BendlDecoder

decoder = BendlDecoder("workflow.bendl")

print(decoder.count_samples())
print(decoder.assignment_format())
print(decoder.asset_names())

assert decoder.read_graph().number_of_nodes() == node_count
assert decoder.read_metadata()["sampler"] == "toy-grid"
```

## Analyze a subset

```python
from binary_ensemble import BendlDecoder

decoder = BendlDecoder("workflow.bendl")

district_one_sizes = []
for assignment in decoder.subsample_every(5):
    district_one_sizes.append(sum(1 for district in assignment if district == 1))

print(district_one_sizes)
```

## Attach post-run provenance

```python
from binary_ensemble import BendlEncoder

encoder = BendlEncoder.append("workflow.bendl")
encoder.add_asset("analysis-notes.txt", "Checked with the end-to-end tutorial.", content_type="text")
encoder.close()
```

## Produce a shareable archive

Relabel/reorder first, then recompress the embedded stream to XBEN.

```python
from binary_ensemble import compress_stream, relabel_bundle

relabel_bundle("workflow.bendl", out_file="workflow-sorted.bendl", sort="mlc")
compress_stream("workflow-sorted.bendl", out_file="workflow-archive.bendl")
```

## Adapting this to GerryChain

The only GerryChain-specific step is extracting assignments in the same node order as the
graph stored in the bundle.

<!-- docs-test: skip requires GerryChain and a real Partition object -->
```python
write_order = list(ordered_graph.nodes)

with encoder.ben_stream(variant="twodelta") as ensemble:
    for partition in chain:
        series = partition.assignment.to_series()
        ensemble.write(series.loc[write_order].astype(int).tolist())
```

The invariant is the same for every sampler: the list you pass to `ensemble.write()` must be in
the embedded graph's node order.
