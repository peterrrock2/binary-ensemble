# Graph ordering deep dive

Graph ordering controls the sequence in which assignment values are written. Because BEN uses
run-length encoding, this one choice can dominate the final file size.

## What ordering changes

An assignment is positional:

```python
assignment = [1, 1, 2, 2]
```

If the graph order is `[A, B, C, D]`, the assignment says `A -> 1`, `B -> 1`,
`C -> 2`, `D -> 2`. If the graph order changes, the assignment must change with it.

## The available orderings

| Ordering | Use when | Strength | Cost |
|---|---|---|---|
| `sort="key"` | Nodes have a meaningful geographic key such as `GEOID20` | Often strongest on Census data | Cheap sort |
| `sort="mlc"` | No reliable key, topology should drive order | Strong default | Graph algorithm |
| `sort="rcm"` | Want topology-based bandwidth reduction | Solid fallback | Graph algorithm |
| `sort=None` | You must preserve existing order exactly | No compression help | None |

## Try an ordering directly

```python
import networkx as nx

from binary_ensemble import graph

dual_graph = nx.convert_node_labels_to_integers(nx.grid_2d_graph(4, 4))
for node in dual_graph.nodes:
    dual_graph.nodes[node]["GEOID20"] = f"{node:04d}"

adjacency = nx.adjacency_data(dual_graph)
ordered_graph, permutation_map = graph.reorder(adjacency, sort="key", key="GEOID20")

print(list(ordered_graph.nodes)[:4])
old_to_new = permutation_map["node_permutation_old_to_new"]
print({node: old_to_new[node] for node in list(old_to_new)[:4]})
```

## Use an ordering while creating a bundle

`add_graph()` returns the reordered graph. That returned graph is the graph whose node order
your assignments should follow.

```python
import networkx as nx

from binary_ensemble import BendlEncoder

dual_graph = nx.convert_node_labels_to_integers(nx.path_graph(4))
adjacency = nx.adjacency_data(dual_graph)

encoder = BendlEncoder("ordering.bendl", overwrite=True)
ordered_graph = encoder.add_graph(adjacency, sort="rcm")
write_order = list(ordered_graph.nodes)

with encoder.stream("ben") as stream:
    stream.write([1, 1, 2, 2])

assert len(write_order) == 4
```

## Use an ordering after a bundle already exists

If you already have a BEN bundle with a graph, use `relabel_bundle()`. It reorders the graph,
rewrites every assignment into that new order, and stores a fresh permutation map.

```python
from binary_ensemble import relabel_bundle

relabel_bundle("ensemble.bendl", out_file="ordering-sorted.bendl", sort="mlc")
```

## Common mistakes

- Sorting a graph after writing assignments, without rewriting the assignments.
- Writing assignments in `gerrychain.Graph.nodes` order while storing a differently ordered
  graph.
- Sorting by a key that is missing or not unique enough for the intended order.
- Recompressing to XBEN before testing graph order, which makes later repair slower.

## Recommended decision

Start with `sort="key", key="GEOID20"` when a Census-like geographic key exists. Otherwise
use `sort="mlc"`. Use `sort=None` only when preserving an external node order is more
important than compression.
