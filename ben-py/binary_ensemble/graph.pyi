import networkx as nx

from binary_ensemble.types import GraphInput, NodePermutationMap, SortMethod

__all__ = [
    "reorder",
    "reorder_multi_level_cluster",
    "reorder_reverse_cuthill_mckee",
    "reorder_by_key",
]

# Each helper returns (reordered_graph, node_permutation_map): the graph is a live NetworkX
# graph, the map is the parsed node_permutation_map.json dict.
def reorder(
    graph: GraphInput, sort: SortMethod = "mlc", key: str | None = None
) -> tuple[nx.Graph, NodePermutationMap]: ...
def reorder_multi_level_cluster(
    graph: GraphInput,
) -> tuple[nx.Graph, NodePermutationMap]: ...
def reorder_reverse_cuthill_mckee(
    graph: GraphInput,
) -> tuple[nx.Graph, NodePermutationMap]: ...
def reorder_by_key(graph: GraphInput, key: str) -> tuple[nx.Graph, NodePermutationMap]: ...
