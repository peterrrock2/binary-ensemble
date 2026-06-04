from typing import Any, Optional, Tuple

__all__ = [
    "reorder",
    "reorder_multi_level_cluster",
    "reorder_reverse_cuthill_mckee",
    "reorder_by_key",
]

# Each helper returns (reordered_graph, node_permutation_map): the graph is a live
# NetworkX graph, the map is the parsed node_permutation_map.json dict.
def reorder(
    graph: Any, sort: str = "mlc", key: Optional[str] = None
) -> Tuple[Any, Any]: ...
def reorder_multi_level_cluster(graph: Any) -> Tuple[Any, Any]: ...
def reorder_reverse_cuthill_mckee(graph: Any) -> Tuple[Any, Any]: ...
def reorder_by_key(graph: Any, key: str) -> Tuple[Any, Any]: ...
