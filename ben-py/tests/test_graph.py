"""Tests for the standalone ``binary_ensemble.graph`` reordering utilities."""

from __future__ import annotations

import json
from pathlib import Path

import pytest

from binary_ensemble import graph as g

EXAMPLE_GRAPH = (
    Path(__file__).resolve().parent.parent
    / "docs"
    / "user"
    / "example_data"
    / "gerrymandria.json"
)


def _graph():
    return json.loads(EXAMPLE_GRAPH.read_text())


def _n():
    return len(_graph()["nodes"])


def _check_consistent(reordered, pmap, n):
    # reordered is a live NetworkX graph.
    assert reordered.number_of_nodes() == n
    mapping = pmap["node_permutation_old_to_new"]
    assert len(mapping) == n
    # old->new is a bijection over [0, n).
    assert sorted(int(k) for k in mapping) == list(range(n))
    assert sorted(mapping.values()) == list(range(n))


def test_reorder_rcm() -> None:
    n = _n()
    reordered, pmap = g.reorder(_graph(), "rcm")
    _check_consistent(reordered, pmap, n)
    assert pmap["ordering_method"] == "reverse-cuthill-mckee"
    assert pmap["key"] is None


def test_reorder_mlc() -> None:
    n = _n()
    reordered, pmap = g.reorder_multi_level_cluster(_graph())
    _check_consistent(reordered, pmap, n)
    assert pmap["ordering_method"] == "multi-level-cluster"


def test_reorder_reverse_cuthill_mckee_helper() -> None:
    n = _n()
    reordered, pmap = g.reorder_reverse_cuthill_mckee(_graph())
    _check_consistent(reordered, pmap, n)
    assert pmap["ordering_method"] == "reverse-cuthill-mckee"


def test_reorder_by_key_id() -> None:
    n = _n()
    reordered, pmap = g.reorder_by_key(_graph(), "id")
    _check_consistent(reordered, pmap, n)
    assert pmap["key"] == "id"
    assert pmap["ordering_method"] is None


def test_reorder_accepts_bytes_and_path() -> None:
    n = _n()
    raw = EXAMPLE_GRAPH.read_bytes()
    r1, p1 = g.reorder(raw, "rcm")
    r2, p2 = g.reorder(str(EXAMPLE_GRAPH), "rcm")
    _check_consistent(r1, p1, n)
    # path and bytes inputs agree (NetworkX graphs compare by identity, so check
    # node order and the permutation map instead).
    assert list(r1.nodes) == list(r2.nodes)
    assert p1 == p2


def test_reorder_rejects_unparseable_graph() -> None:
    with pytest.raises(Exception, match="Failed to reorder graph"):
        g.reorder(b"not valid json at all", "rcm")
