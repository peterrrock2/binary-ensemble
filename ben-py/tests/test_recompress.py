"""Tests for ``binary_ensemble.bundle.compress_stream`` (BEN bundle → XBEN bundle)."""

from __future__ import annotations

import json
from pathlib import Path

import pytest

from binary_ensemble.bundle import BendlDecoder, BendlEncoder, compress_stream

EXAMPLE_GRAPH = (
    Path(__file__).resolve().parent.parent
    / "docs"
    / "user"
    / "example_data"
    / "gerrymandria.json"
)


def _graph():
    return json.loads(EXAMPLE_GRAPH.read_text())


def _build_ben_bundle(path: Path):
    n = len(_graph()["nodes"])
    samples = [[(i + j) % 4 + 1 for j in range(n)] for i in range(8)]
    with BendlEncoder(path, overwrite=True) as enc:
        enc.add_graph(_graph(), sort="rcm")
        enc.add_metadata({"seed": 99})
        with enc.stream("ben") as s:
            for a in samples:
                s.write(a)
        enc.add_asset("notes.txt", "hi", content_type="text")
    return samples


def _assert_preserved(src_dec, out_dec):
    assert out_dec.assignment_format() == "xben"
    assert out_dec.asset_names() == src_dec.asset_names()
    # Decoded payloads + JSON flag preserved semantically.
    src_flags = {a["name"]: ("json" in a["flags"]) for a in src_dec.list_assets()}
    out_flags = {a["name"]: ("json" in a["flags"]) for a in out_dec.list_assets()}
    assert src_flags == out_flags
    for name in src_dec.asset_names():
        assert out_dec.read_asset_bytes(name) == src_dec.read_asset_bytes(name)


def test_compress_stream_explicit_out_path(tmp_path: Path) -> None:
    src = tmp_path / "in.bendl"
    out = tmp_path / "out.bendl"
    samples = _build_ben_bundle(src)
    src_dec = BendlDecoder(src)

    compress_stream(src, out_file=out)

    out_dec = BendlDecoder(out)
    _assert_preserved(src_dec, out_dec)
    assert list(out_dec) == samples
    assert out_dec.read_metadata() == {"seed": 99}
    assert out_dec.read_node_permutation_map() is not None
    # Source bundle is untouched and still BEN.
    assert BendlDecoder(src).assignment_format() == "ben"


def test_compress_stream_in_place(tmp_path: Path) -> None:
    src = tmp_path / "in.bendl"
    samples = _build_ben_bundle(src)
    before = BendlDecoder(src)
    before_assets = {n: before.read_asset_bytes(n) for n in before.asset_names()}

    compress_stream(src, in_place=True)

    after = BendlDecoder(src)
    assert after.assignment_format() == "xben"
    assert list(after) == samples
    for name, payload in before_assets.items():
        assert after.read_asset_bytes(name) == payload


def test_compress_stream_arg_validation(tmp_path: Path) -> None:
    src = tmp_path / "in.bendl"
    _build_ben_bundle(src)
    with pytest.raises(ValueError, match="either in_place=True or out_file"):
        compress_stream(src)
    with pytest.raises(ValueError, match="not both"):
        compress_stream(src, out_file=tmp_path / "o.bendl", in_place=True)


def test_compress_stream_assets_only_bundle(tmp_path: Path) -> None:
    src = tmp_path / "assets.bendl"
    enc = BendlEncoder(src, overwrite=True)
    enc.add_metadata({"only": "assets"})
    enc.close()

    out = tmp_path / "assets.xben.bendl"
    compress_stream(src, out_file=out)

    dec = BendlDecoder(out)
    assert dec.assignment_format() == "xben"
    assert dec.is_complete()
    assert dec.count_samples() == 0
    assert list(dec) == []
    assert dec.read_metadata() == {"only": "assets"}


def test_compress_stream_out_file_refuses_existing(tmp_path: Path) -> None:
    src = tmp_path / "in.bendl"
    _build_ben_bundle(src)
    out = tmp_path / "exists.bendl"
    out.write_bytes(b"existing")
    with pytest.raises(OSError, match="already exists"):
        compress_stream(src, out_file=out)
