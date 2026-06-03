"""Graph reordering utilities (the reben orderings).

Reordering a dual graph before building a chain (or a bundle) can dramatically
improve BEN/XBEN compression. Each function takes a NetworkX adjacency-format
graph (a ``dict``/``list``, raw JSON ``bytes``, a file-like with ``.read()``, or
a path) and returns ``(reordered_graph, node_permutation_map)``:

- ``reordered_graph`` is a live NetworkX graph in its new node ordering (the same
  shape :meth:`binary_ensemble.bundle.BendlEncoder.add_graph` and
  :meth:`binary_ensemble.bundle.BendlDecoder.read_graph` return).
- ``node_permutation_map`` is the parsed ``node_permutation_map.json`` payload —
  an object with a ``node_permutation_old_to_new`` field mapping original
  zero-based node positions to their new positions.

To reorder *and* embed the result in a bundle in one step, pass
``preprocess_method`` to :meth:`binary_ensemble.bundle.BendlEncoder.add_graph`.
"""

from __future__ import annotations

from typing import Any, Tuple

from binary_ensemble._core import graph_reorder

__all__ = [
    "reorder",
    "reorder_multi_level_cluster",
    "reorder_reverse_cuthill_mckee",
    "reorder_by_key",
]


def reorder(graph: Any, method: str) -> Tuple[Any, Any]:
    """Reorder ``graph`` by ``method`` and return ``(reordered_graph, node_permutation_map)``.

    ``method`` is one of ``"multi-level-cluster"`` / ``"mlc"``,
    ``"reverse-cuthill-mckee"`` / ``"rcm"``, or a node-attribute key (e.g.
    ``"geoid"``, or the special ``"id"`` for the NetworkX node id).
    """
    return graph_reorder(graph, method)


def reorder_multi_level_cluster(graph: Any) -> Tuple[Any, Any]:
    """Reorder ``graph`` using recursive multi-level clustering."""
    return graph_reorder(graph, "multi-level-cluster")


def reorder_reverse_cuthill_mckee(graph: Any) -> Tuple[Any, Any]:
    """Reorder ``graph`` using Reverse Cuthill-McKee."""
    return graph_reorder(graph, "reverse-cuthill-mckee")


def reorder_by_key(graph: Any, key: str) -> Tuple[Any, Any]:
    """Reorder ``graph`` by sorting on a node-attribute ``key`` (use ``"id"`` for node id)."""
    return graph_reorder(graph, key)
