"""Graph reordering utilities used by ``ben sort-graph`` and bundle relabeling.

Reordering a dual graph before building a chain (or a bundle) can dramatically improve BEN/XBEN
compression. Each function takes a graph (a live ``networkx.Graph``, or adjacency-format JSON
as a ``dict``/``list``, raw ``bytes``, a file-like with ``.read()``, or a path) and returns
``(reordered_graph, node_permutation_map)``:

- ``reordered_graph`` is a live NetworkX graph in its new node ordering (the same shape
  :meth:`binary_ensemble.bundle.BendlEncoder.add_graph` and
  :meth:`binary_ensemble.bundle.BendlDecoder.read_graph` return).
- ``node_permutation_map`` is the parsed ``node_permutation_map.json`` payload, an object with a
  ``node_permutation_old_to_new`` field mapping original zero-based node positions to their new
  positions.

To reorder *and* embed the result in a bundle in one step, pass ``sort`` / ``key`` to
:meth:`binary_ensemble.bundle.BendlEncoder.add_graph`.
"""

from __future__ import annotations

from typing import TYPE_CHECKING

from binary_ensemble._core import graph_reorder
from binary_ensemble.types import GraphInput, NodePermutationMap, SortMethod

if TYPE_CHECKING:
    import networkx as nx

__all__ = [
    "reorder",
    "reorder_multi_level_cluster",
    "reorder_reverse_cuthill_mckee",
    "reorder_by_key",
]


def reorder(
    graph: GraphInput, sort: SortMethod = "mlc", key: str | None = None
) -> "tuple[nx.Graph, NodePermutationMap]":
    """Reorder ``graph`` and return ``(reordered_graph, node_permutation_map)``.

    Args:
        graph (GraphInput): The dual graph (:data:`~binary_ensemble.types.GraphInput`): a live
            ``networkx.Graph`` (subclasses such as ``gerrychain.Graph`` count), or
            adjacency-format JSON as a parsed ``dict`` or ``list``, raw ``bytes``, a file-like
            object with ``.read()``, or a ``str`` / ``os.PathLike`` path to a JSON file. A plain
            ``str`` is a *path* here.
        sort (SortMethod, optional): The ordering (:data:`~binary_ensemble.types.SortMethod`):
            ``"mlc"`` (multi-level clustering), ``"rcm"`` (reverse Cuthill-McKee), or ``"key"``
            (sort by the node attribute named in ``key``). Default is ``"mlc"``.
        key (str | None, optional): Node attribute to sort by, e.g. ``key="GEOID"``;
            ``key="id"`` sorts by the NetworkX node id. Required with (and only valid with)
            ``sort="key"``. Default is ``None``.

    Returns:
        tuple[networkx.Graph, NodePermutationMap]: The reordered graph (a live NetworkX graph,
        the shape :meth:`BendlEncoder.add_graph <binary_ensemble.bundle.BendlEncoder.add_graph>`
        and :meth:`BendlDecoder.read_graph <binary_ensemble._core.BendlDecoder.read_graph>`
        return) and the parsed permutation map
        (:class:`~binary_ensemble.types.NodePermutationMap`), whose
        ``node_permutation_old_to_new`` field maps original zero-based node positions to their
        new positions.

    Raises:
        ValueError: If ``sort`` / ``key`` is invalid.
    """
    return graph_reorder(graph, sort, key)


def reorder_multi_level_cluster(
    graph: GraphInput,
) -> "tuple[nx.Graph, NodePermutationMap]":
    """Reorder ``graph`` using recursive multi-level clustering.

    Equivalent to :func:`reorder` with ``sort="mlc"``.

    Args:
        graph (GraphInput): The dual graph (:data:`~binary_ensemble.types.GraphInput`): a live
            ``networkx.Graph``, or adjacency-format JSON as a parsed ``dict`` or ``list``, raw
            ``bytes``, a file-like object with ``.read()``, or a ``str`` / ``os.PathLike`` path
            to a JSON file.

    Returns:
        tuple[networkx.Graph, NodePermutationMap]: The reordered graph and the parsed permutation
        map (see :func:`reorder`).
    """
    return graph_reorder(graph, "mlc")


def reorder_reverse_cuthill_mckee(
    graph: GraphInput,
) -> "tuple[nx.Graph, NodePermutationMap]":
    """Reorder ``graph`` using Reverse Cuthill-McKee.

    Equivalent to :func:`reorder` with ``sort="rcm"``.

    Args:
        graph (GraphInput): The dual graph (:data:`~binary_ensemble.types.GraphInput`): a live
            ``networkx.Graph``, or adjacency-format JSON as a parsed ``dict`` or ``list``, raw
            ``bytes``, a file-like object with ``.read()``, or a ``str`` / ``os.PathLike`` path
            to a JSON file.

    Returns:
        tuple[networkx.Graph, NodePermutationMap]: The reordered graph and the parsed permutation
        map (see :func:`reorder`).
    """
    return graph_reorder(graph, "rcm")


def reorder_by_key(graph: GraphInput, key: str) -> "tuple[nx.Graph, NodePermutationMap]":
    """Reorder ``graph`` by sorting on a node attribute.

    Equivalent to :func:`reorder` with ``sort="key"``.

    Args:
        graph (GraphInput): The dual graph (:data:`~binary_ensemble.types.GraphInput`): a live
            ``networkx.Graph``, or adjacency-format JSON as a parsed ``dict`` or ``list``, raw
            ``bytes``, a file-like object with ``.read()``, or a ``str`` / ``os.PathLike`` path
            to a JSON file.
        key (str): Node attribute to sort by, e.g. ``key="GEOID"``; the special ``key="id"``
            sorts by the NetworkX node id.

    Returns:
        tuple[networkx.Graph, NodePermutationMap]: The reordered graph and the parsed permutation
        map (see :func:`reorder`).
    """
    return graph_reorder(graph, "key", key)
