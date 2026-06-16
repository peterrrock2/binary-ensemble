"""Static typing assertions for the public ben-py surface.

This is not a pytest module: the type checkers check it (via ``task typecheck-python``) and
fail if the public signatures regress. Positive assertions use :func:`typing.assert_type`;
negative assertions are calls that *must not* type-check, suppressed with bare
``# type: ignore`` comments (both ty and pyright honor those) and kept honest by pyright's
``reportUnnecessaryTypeIgnoreComment``; if the call ever becomes legal, the now-unused ignore
fails the check.

Nothing here executes; the module exists purely for static analysis.
"""

from __future__ import annotations

import io
from pathlib import Path
from typing import assert_type

import networkx as nx

from binary_ensemble import (
    BenDecoder,
    BenEncoder,
    BendlDecoder,
    BendlEncoder,
    compress_stream,
    relabel_bundle,
)
from binary_ensemble import graph as bgraph
from binary_ensemble.types import (
    AssetEntry,
    AssignmentFormat,
    NodePermutationMap,
)


def bundle_encoder_surface(tmp: Path) -> None:
    enc = BendlEncoder(tmp / "out.bendl", overwrite=True)

    # Graph inputs: live NetworkX graphs, dicts, bytes, file-likes, and paths all type-check.
    enc.add_graph(nx.Graph(), sort=None)
    enc.add_graph({"nodes": [], "adjacency": []}, sort=None)
    enc.add_graph(b"{}", sort="rcm")
    enc.add_graph(io.BytesIO(b"{}"))
    enc.add_graph(tmp / "graph.json", sort="key", key="GEOID")
    enc.add_graph("graph.json")  # a plain str is a path for graphs
    enc.add_graph(tmp, sort="bogus")  # type: ignore

    enc.add_metadata({"seed": 1234})
    enc.add_metadata(tmp / "metadata.json")

    # add_asset overloads: payload shape is tied to content_type.
    enc.add_asset("params.json", {"node_repeats": 2}, "json")
    enc.add_asset("notes.txt", "plain text content", "text")
    enc.add_asset("blob.bin", b"\x00\x01", "binary")
    enc.add_asset("tracts.gpkg", tmp / "tracts.gpkg", "file")
    enc.add_asset("bad.txt", {"not": "text"}, "text")  # type: ignore
    enc.add_asset("bad.any", b"x", "blob")  # type: ignore

    with enc.ben_stream(variant="twodelta") as stream:
        stream.write([1, 2, 3])
        stream.write((1, 2, 3))
    BendlEncoder.append(tmp / "out.bendl").remove_asset("notes.txt")

    # ben_stream() has no format parameter, and variant is keyword-only with a literal "twodelta"
    # default; None is not a legal stand-in for it.
    enc.ben_stream("ben")  # type: ignore
    enc.ben_stream(variant="xben")  # type: ignore
    enc.ben_stream(variant=None)  # type: ignore


def bundle_decoder_surface(dec: BendlDecoder) -> None:
    for assignment in dec:
        assert_type(assignment, list[int])

    assert_type(dec.assignment_format(), AssignmentFormat)
    assert_type(dec.version(), tuple[int, int])
    assert_type(dec.stream_size(), int)
    assert_type(dec.asset_size("blob.bin"), int)
    assert_type(dec.read_asset_bytes("blob.bin"), bytes)

    entries = dec.list_assets()
    assert_type(entries[0], AssetEntry)
    assert_type(entries[0]["flags"], list[str])

    pmap = dec.read_node_permutation_map()
    if pmap is not None:
        assert_type(pmap["node_permutation_old_to_new"], dict[str, int])

    dec.subsample_indices([1, 500, 1000])
    dec.subsample_every(250, offset=2)
    dec.verify()


def graph_surface(tmp: Path) -> None:
    _graph, pmap = bgraph.reorder(tmp / "graph.json", sort="mlc")
    assert_type(pmap, NodePermutationMap)
    bgraph.reorder_by_key({"nodes": []}, key="GEOID")
    bgraph.reorder(tmp, sort="fancy")  # type: ignore


def stream_and_transforms_surface(tmp: Path) -> None:
    with BenEncoder(tmp / "t.ben", overwrite=True, variant="mkv_chain") as enc:
        enc.write([0, 1])
    dec = BenDecoder(tmp / "t.xben", mode="xben")
    dec.subsample_range(1, 3)
    BenDecoder(tmp, mode="jsonl")  # type: ignore

    compress_stream(tmp / "a.bendl")  # out_file=None means in place
    compress_stream(tmp / "a.bendl", out_file=tmp / "b.bendl", overwrite=True)
    relabel_bundle(tmp / "a.bendl", out_file=tmp / "c.bendl", overwrite=True)
    relabel_bundle(tmp / "a.bendl", sort="rcm")
    relabel_bundle(tmp / "a.bendl", in_place=True)  # type: ignore
    relabel_bundle(tmp / "a.bendl", sort="random")  # type: ignore
