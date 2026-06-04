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

To reorder *and* embed the result in a bundle in one step, pass ``sort`` / ``key``
to :meth:`binary_ensemble.bundle.BendlEncoder.add_graph`.
"""

from __future__ import annotations

from typing import Any, Optional, Tuple

from binary_ensemble._core import graph_reorder

__all__ = [
    "reorder",
    "reorder_multi_level_cluster",
    "reorder_reverse_cuthill_mckee",
    "reorder_by_key",
]


def reorder(
    graph: Any, sort: str = "mlc", key: Optional[str] = None
) -> Tuple[Any, Any]:
    """Reorder ``graph`` and return ``(reordered_graph, node_permutation_map)``.

    ``sort`` is ``"mlc"`` (multi-level clustering, the default), ``"rcm"``
    (reverse Cuthill-McKee), or ``"key"`` to sort by the node attribute named in
    ``key`` (e.g. ``sort="key", key="GEOID"``; ``key="id"`` sorts by the NetworkX
    node id). ``key`` is only valid with ``sort="key"``.
    """
    return graph_reorder(graph, sort, key)


def reorder_multi_level_cluster(graph: Any) -> Tuple[Any, Any]:
    """Reorder ``graph`` using recursive multi-level clustering."""
    return graph_reorder(graph, "mlc")


def reorder_reverse_cuthill_mckee(graph: Any) -> Tuple[Any, Any]:
    """Reorder ``graph`` using Reverse Cuthill-McKee."""
    return graph_reorder(graph, "rcm")


def reorder_by_key(graph: Any, key: str) -> Tuple[Any, Any]:
    """Reorder ``graph`` by sorting on a node-attribute ``key`` (use ``"id"`` for node id)."""
    return graph_reorder(graph, "key", key)
