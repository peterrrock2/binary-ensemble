# `binary_ensemble.graph`

The graph module exposes the same reordering algorithms used by
`BendlEncoder.add_graph()` and `relabel_bundle()`. Use it when you want to inspect or manage
the graph order yourself before writing assignments.

Each function returns `(reordered_graph, node_permutation_map)`.

| Function                               | Ordering                                        |
| -------------------------------------- | ----------------------------------------------- |
| `reorder(graph, sort="mlc")`           | Dispatch helper for all orderings               |
| `reorder_multi_level_cluster(graph)`   | Recursive topology-based clustering             |
| `reorder_reverse_cuthill_mckee(graph)` | Reverse Cuthill-McKee bandwidth reduction       |
| `reorder_by_key(graph, key)`           | Sort by a node attribute, or `"id"` for node id |

```{eval-rst}
.. automodule:: binary_ensemble.graph
```

## Reordering functions

```python
import networkx as nx

from binary_ensemble import graph

dual_graph = nx.convert_node_labels_to_integers(nx.grid_2d_graph(4, 4))
for node in dual_graph.nodes:
    dual_graph.nodes[node]["GEOID20"] = f"{node:04d}"

adjacency = nx.adjacency_data(dual_graph)
reordered, permutation_map = graph.reorder(adjacency, sort="key", key="GEOID20")

assert reordered.number_of_nodes() == dual_graph.number_of_nodes()
assert "node_permutation_old_to_new" in permutation_map
```

The returned `reordered` graph is a NetworkX graph in the new node order. If you write an
assignment stream against this graph, emit assignment values in `list(reordered.nodes)`
order.

```{tip}
If you are creating a bundle, `BendlEncoder.add_graph(..., sort=...)` is usually simpler:
it reorders the graph, stores `graph.json`, stores `node_permutation_map.json`, and returns
the reordered graph in one call.
```

```{eval-rst}
.. autofunction:: binary_ensemble.graph.reorder

.. autofunction:: binary_ensemble.graph.reorder_multi_level_cluster

.. autofunction:: binary_ensemble.graph.reorder_reverse_cuthill_mckee

.. autofunction:: binary_ensemble.graph.reorder_by_key
```
