"""Tests for transcoding a bundle's stream between BEN and XBEN."""

from __future__ import annotations

import struct
from pathlib import Path

import pytest

from binary_ensemble.bundle import BendlDecoder, BendlEncoder, compress_stream, decompress_stream
from helpers import example_graph as _graph


def _crc32c(data: bytes) -> int:
    crc = 0xFFFFFFFF
    for byte in data:
        crc ^= byte
        for _ in range(8):
            mask = -(crc & 1)
            crc = (crc >> 1) ^ (0x82F63B78 & mask)
    return (~crc) & 0xFFFFFFFF


def _restamp_header(path: Path, data: bytearray) -> None:
    data[64:68] = struct.pack("<I", _crc32c(data[:64]))
    path.write_bytes(data)


def _build_ben_bundle(path: Path):
    n = len(_graph()["nodes"])
    samples = [[(i + j) % 4 + 1 for j in range(n)] for i in range(8)]
    with BendlEncoder(path, overwrite=True) as enc:
        enc.add_graph(_graph(), sort="rcm")
        enc.add_metadata({"seed": 99})
        with enc.ben_stream() as s:
            for a in samples:
                s.write(a)
        enc.add_asset("notes.txt", "hi", content_type="text")
    return samples


def _assert_preserved(src_dec, out_dec, assignment_format):
    assert out_dec.assignment_format() == assignment_format
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
    _assert_preserved(src_dec, out_dec, "xben")
    assert list(out_dec) == samples
    assert out_dec.read_metadata() == {"seed": 99}
    assert out_dec.read_node_permutation_map() is not None
    # Source bundle is untouched and still BEN.
    assert BendlDecoder(src).assignment_format() == "ben"


def test_compress_stream_in_place_by_default(tmp_path: Path) -> None:
    src = tmp_path / "in.bendl"
    samples = _build_ben_bundle(src)
    before = BendlDecoder(src)
    before_assets = {n: before.read_asset_bytes(n) for n in before.asset_names()}

    # out_file=None means in place: src is atomically replaced.
    compress_stream(src)

    after = BendlDecoder(src)
    assert after.assignment_format() == "xben"
    assert list(after) == samples
    for name, payload in before_assets.items():
        assert after.read_asset_bytes(name) == payload


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
    # overwrite=True is the explicit opt-in to replace it.
    compress_stream(src, out_file=out, overwrite=True)
    assert BendlDecoder(out).assignment_format() == "xben"


def test_decompress_stream_explicit_out_path(tmp_path: Path) -> None:
    src = tmp_path / "in.bendl"
    compressed = tmp_path / "compressed.bendl"
    out = tmp_path / "out.bendl"
    samples = _build_ben_bundle(src)
    compress_stream(src, out_file=compressed)
    compressed_dec = BendlDecoder(compressed)

    decompress_stream(compressed, out_file=out)

    out_dec = BendlDecoder(out)
    _assert_preserved(compressed_dec, out_dec, "ben")
    assert list(out_dec) == samples
    assert out_dec.read_metadata() == {"seed": 99}
    assert out_dec.read_node_permutation_map() is not None
    assert BendlDecoder(compressed).assignment_format() == "xben"


def test_decompress_stream_in_place_by_default(tmp_path: Path) -> None:
    path = tmp_path / "in.bendl"
    samples = _build_ben_bundle(path)
    compress_stream(path)
    before = BendlDecoder(path)
    before_assets = {name: before.read_asset_bytes(name) for name in before.asset_names()}

    decompress_stream(path)

    after = BendlDecoder(path)
    assert after.assignment_format() == "ben"
    assert list(after) == samples
    for name, payload in before_assets.items():
        assert after.read_asset_bytes(name) == payload


def test_decompress_stream_assets_only_bundle(tmp_path: Path) -> None:
    src = tmp_path / "assets.bendl"
    with BendlEncoder(src, overwrite=True) as enc:
        enc.add_metadata({"only": "assets"})
    compressed = tmp_path / "assets.xben.bendl"
    out = tmp_path / "assets.ben.bendl"
    compress_stream(src, out_file=compressed)

    decompress_stream(compressed, out_file=out)

    dec = BendlDecoder(out)
    assert dec.assignment_format() == "ben"
    assert dec.is_complete()
    assert dec.count_samples() == 0
    assert list(dec) == []
    assert dec.read_metadata() == {"only": "assets"}


def test_decompress_stream_assets_only_verifies_stream_checksum(tmp_path: Path) -> None:
    src = tmp_path / "assets.bendl"
    with BendlEncoder(src, overwrite=True) as enc:
        enc.add_metadata({"only": "assets"})
    compressed = tmp_path / "assets.xben.bendl"
    compress_stream(src, out_file=compressed)

    data = bytearray(compressed.read_bytes())
    data[20:24] = struct.pack("<I", 1)
    _restamp_header(compressed, data)

    with pytest.raises(Exception, match="source stream.*checksum mismatch"):
        decompress_stream(compressed, out_file=tmp_path / "out.bendl")


def test_decompress_stream_verifies_sample_count(tmp_path: Path) -> None:
    src = tmp_path / "in.bendl"
    compressed = tmp_path / "compressed.bendl"
    _build_ben_bundle(src)
    compress_stream(src, out_file=compressed)

    data = bytearray(compressed.read_bytes())
    data[56:64] = struct.pack("<q", 99)
    _restamp_header(compressed, data)

    with pytest.raises(Exception, match="source sample count.*sample_count"):
        decompress_stream(compressed, out_file=tmp_path / "out.bendl")


def test_stream_transcodes_require_the_opposite_format(tmp_path: Path) -> None:
    ben = tmp_path / "ben.bendl"
    xben = tmp_path / "xben.bendl"
    _build_ben_bundle(ben)
    compress_stream(ben, out_file=xben)

    with pytest.raises(Exception, match="requires an embedded XBEN stream"):
        decompress_stream(ben, out_file=tmp_path / "bad-ben.bendl")
    with pytest.raises(Exception, match="requires an embedded BEN stream"):
        compress_stream(xben, out_file=tmp_path / "bad-xben.bendl")
