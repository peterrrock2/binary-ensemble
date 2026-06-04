"""Tests for ``binary_ensemble.bundle.relabel_bundle`` (reorder graph + relabel stream)."""

from __future__ import annotations

import json
from pathlib import Path

import pytest

from binary_ensemble.bundle import (
    BendlDecoder,
    BendlEncoder,
    compress_stream,
    relabel_bundle,
)

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


def _build_ben_bundle(path: Path, with_graph: bool = True):
    n = _n()
    samples = [[(i + j) % 4 + 1 for j in range(n)] for i in range(8)]
    enc = BendlEncoder(path, overwrite=True)
    if with_graph:
        enc.add_graph(_graph(), preprocess_method=None)  # store in raw order
    enc.add_metadata({"seed": 99})
    with enc.stream("ben") as s:
        for a in samples:
            s.write(a)
    enc.add_asset("notes.txt", "hi", content_type="text")
    return samples


def _depermute(dst_plan, old_to_new):
    """Map an MLC-ordered plan back to the source node order."""
    return [dst_plan[old_to_new[i]] for i in range(len(dst_plan))]


def test_relabel_out_file_is_lossless_and_preserves_assets(tmp_path: Path) -> None:
    src = tmp_path / "in.bendl"
    out = tmp_path / "out.bendl"
    samples = _build_ben_bundle(src)

    relabel_bundle(src, out_file=out, method="mlc")

    dec = BendlDecoder(out)
    # Stays BEN, same sample count, canonical graph + permutation map present.
    assert dec.assignment_format() == "ben"
    assert len(dec) == len(samples)
    assert dec.asset_names() == [
        "graph.json",
        "node_permutation_map.json",
        "metadata.json",
        "notes.txt",
    ]
    # Metadata + custom assets carried over.
    assert dec.read_metadata() == {"seed": 99}
    assert dec.read_asset_bytes("notes.txt") == b"hi"

    pmap = dec.read_node_permutation_map()
    old_to_new = {int(k): v for k, v in pmap["node_permutation_old_to_new"].items()}
    assert sorted(old_to_new) == list(range(_n()))
    assert sorted(old_to_new.values()) == list(range(_n()))

    # Relabeling is lossless: de-permuting reproduces the source plans exactly.
    relabeled = list(dec)
    assert [_depermute(p, old_to_new) for p in relabeled] == samples
    # Source bundle is untouched.
    assert list(BendlDecoder(src)) == samples


def test_relabel_in_place(tmp_path: Path) -> None:
    src = tmp_path / "in.bendl"
    samples = _build_ben_bundle(src)

    relabel_bundle(src, in_place=True, method="rcm")

    dec = BendlDecoder(src)
    assert dec.assignment_format() == "ben"
    assert len(dec) == len(samples)
    assert dec.read_node_permutation_map()["ordering_method"] == "reverse-cuthill-mckee"
    old_to_new = {
        int(k): v
        for k, v in dec.read_node_permutation_map()[
            "node_permutation_old_to_new"
        ].items()
    }
    assert [_depermute(p, old_to_new) for p in dec] == samples


def test_relabel_arg_validation(tmp_path: Path) -> None:
    src = tmp_path / "in.bendl"
    _build_ben_bundle(src)
    with pytest.raises(ValueError, match="either in_place=True or out_file"):
        relabel_bundle(src)
    with pytest.raises(ValueError, match="not both"):
        relabel_bundle(src, out_file=tmp_path / "o.bendl", in_place=True)


def test_relabel_requires_graph(tmp_path: Path) -> None:
    src = tmp_path / "nograph.bendl"
    _build_ben_bundle(src, with_graph=False)
    with pytest.raises(ValueError, match="no graph.json"):
        relabel_bundle(src, out_file=tmp_path / "o.bendl")


def test_relabel_rejects_xben_bundle(tmp_path: Path) -> None:
    src = tmp_path / "in.bendl"
    xben = tmp_path / "in.xben.bendl"
    _build_ben_bundle(src)
    compress_stream(src, out_file=xben)
    with pytest.raises(ValueError, match="only supports BEN"):
        relabel_bundle(xben, out_file=tmp_path / "o.bendl")
