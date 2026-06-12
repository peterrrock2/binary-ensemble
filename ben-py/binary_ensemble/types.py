"""Shared type aliases for the public API.

These names describe the payload shapes the API accepts and returns, so user code can annotate
against them::

    from binary_ensemble.types import GraphInput, NodePermutationMap

    def load(graph: GraphInput) -> NodePermutationMap: ...

Nothing here changes runtime behavior; the aliases exist so the signatures in
:mod:`binary_ensemble.bundle`, :mod:`binary_ensemble.graph`, and the ``_core`` stubs say what
they actually mean.
"""

from __future__ import annotations

import os
from typing import Any, Literal, Protocol, TypedDict

# Imported eagerly so GraphInput is one honest runtime definition. networkx is a hard
# dependency and its core import is light (no compiled deps).
import networkx as nx

__all__ = [
    "AssetContentType",
    "AssetEntry",
    "AssignmentFormat",
    "BinaryAssetPayload",
    "GraphInput",
    "JsonAssetPayload",
    "MetadataInput",
    "NodePermutationMap",
    "SortMethod",
    "StrPath",
    "SupportsRead",
    "TextAssetPayload",
    "Variant",
]

Variant = Literal["standard", "mkv_chain", "twodelta"]
"""BEN encoding variant (see the variants concept page for how to choose)."""

AssignmentFormat = Literal["ben", "xben"]
"""Wire format of an assignment stream."""

SortMethod = Literal["mlc", "rcm", "key"]
"""Graph reordering method: multi-level clustering, reverse Cuthill-McKee, or
sort-by-node-attribute (which also requires ``key=``)."""

AssetContentType = Literal["json", "text", "binary", "file"]
"""How :meth:`~binary_ensemble.bundle.BendlEncoder.add_asset` treats its payload."""

StrPath = str | os.PathLike[str]
"""A filesystem path."""


class SupportsRead(Protocol):
    """A file-like object whose ``.read()`` yields ``bytes`` or ``str``."""

    def read(self) -> bytes | str: ...


GraphInput = nx.Graph | dict[str, Any] | list[Any] | bytes | bytearray | SupportsRead | StrPath
"""Accepted forms for a dual graph: a live ``networkx.Graph`` (subclasses such as
``gerrychain.Graph`` count; its node iteration order is preserved), or adjacency-format JSON as a
parsed ``dict`` / ``list``, raw ``bytes``, a file-like with ``.read()``, or a path to a JSON
file. A plain ``str`` is a *path* here."""

MetadataInput = dict[str, Any] | list[Any] | bytes | bytearray | SupportsRead | StrPath
"""Accepted forms for ``metadata.json`` payloads: a parsed ``dict`` / ``list``, raw JSON
``bytes``, a file-like with ``.read()``, or a path to a JSON file (a plain ``str`` is a *path*,
never inline JSON)."""

BinaryAssetPayload = bytes | bytearray | memoryview | str | SupportsRead | os.PathLike[str]
"""``add_asset`` payloads for ``content_type="binary"``: bytes-like (stored verbatim), ``str``
(stored as its UTF-8 encoding — *content*, not a path), a file-like with ``.read()``, or an
``os.PathLike`` whose file is read. Note that a plain ``str`` is content; only ``os.PathLike``
objects are treated as paths."""

TextAssetPayload = BinaryAssetPayload
"""``add_asset`` payloads for ``content_type="text"`` — the same shapes as
:data:`BinaryAssetPayload`, but the resulting bytes must be valid UTF-8."""

JsonAssetPayload = dict[str, Any] | list[Any] | BinaryAssetPayload
"""``add_asset`` payloads for ``content_type="json"``: additionally accepts a ``dict`` /
``list``, which is serialized via ``json.dumps``. The resulting bytes must be valid UTF-8
JSON."""


class NodePermutationMap(TypedDict):
    """The parsed ``node_permutation_map.json`` payload.

    ``node_permutation_old_to_new`` maps original zero-based node positions (as JSON string keys)
    to their new positions. Exactly one of ``ordering_method`` / ``key`` records how the ordering
    was produced.
    """

    node_permutation_old_to_new: dict[str, int]
    ordering_method: str | None
    key: str | None


class AssetEntry(TypedDict):
    """One bundle-directory entry, as returned by
    :meth:`~binary_ensemble.bundle.BendlDecoder.list_assets`."""

    name: str
    type: int
    offset: int
    len: int
    flags: list[str]
