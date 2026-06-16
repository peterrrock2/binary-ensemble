"""Shared test helpers: the example-graph fixture trio and corruption injection.

``tests/data/gerrymandria.json`` is load-bearing for both the unit tests and the docs-snippet
runner; every file that needs it goes through these helpers so the path and conventions live
in exactly one place.
"""

from __future__ import annotations

import json
from pathlib import Path

EXAMPLE_GRAPH = Path(__file__).resolve().parent / "data" / "gerrymandria.json"


def example_graph() -> dict:
    """The example dual graph, freshly parsed (callers may mutate their copy)."""
    return json.loads(EXAMPLE_GRAPH.read_text())


def example_node_count() -> int:
    return len(example_graph()["nodes"])


def flip_byte_at(path: Path, marker: bytes) -> None:
    """XOR the first byte of ``marker`` wherever it first occurs in the file."""
    data = bytearray(path.read_bytes())
    pos = data.find(marker)
    assert pos != -1, f"marker {marker!r} not found"
    data[pos] ^= 0xFF
    path.write_bytes(bytes(data))
