"""Byte-level robustness tests for ``BendlDecoder``.

These construct ``.bendl`` bundles directly in Python from the on-disk format
spec (``ben/src/io/bundle/format.rs``). This keeps the tests self-contained and
lets them stress odd byte layouts a writer could not produce (truncated files,
bad magic, dangling offsets, etc). Real BEN/XBEN stream payloads are produced via
``BenEncoder`` / ``encode_jsonl_to_xben`` so the stream region always matches
what the main pipeline produces.

Bundle *authoring* (``BendlEncoder``) is covered in ``test_bundle_api.py``.
"""

from __future__ import annotations

import json
import lzma
import random
import struct
from pathlib import Path
from typing import Iterable, List, Optional, Tuple

import pytest

from binary_ensemble import BenDecoder, BenEncoder, encode_jsonl_to_xben
from binary_ensemble.bundle import BendlDecoder, BendlEncoder

# ---------------------------------------------------------------------------
# Format constants (mirror ben/src/io/bundle/format.rs)
# ---------------------------------------------------------------------------

BENDL_MAGIC = b"BENDL\x00\x00\x01"
BENDL_MAJOR_VERSION = 1
BENDL_MINOR_VERSION = 0
HEADER_SIZE = 64

COMPLETE_NO = 0
COMPLETE_YES = 1

ASSIGNMENT_FORMAT_BEN = 1
ASSIGNMENT_FORMAT_XBEN = 2

ASSET_TYPE_METADATA = 1
ASSET_TYPE_GRAPH = 2
ASSET_TYPE_NODE_PERMUTATION_MAP = 3
ASSET_TYPE_CUSTOM = 4

ASSET_FLAG_JSON = 1 << 0
ASSET_FLAG_XZ = 1 << 1
ASSET_FLAG_CHECKSUM = 1 << 2

HEADER_FLAG_STREAM_CHECKSUM = 1 << 0


def _crc32c(data: bytes) -> int:
    crc = 0xFFFFFFFF
    for byte in data:
        crc ^= byte
        for _ in range(8):
            mask = -(crc & 1)
            crc = (crc >> 1) ^ (0x82F63B78 & mask)
    return (~crc) & 0xFFFFFFFF


# ---------------------------------------------------------------------------
# Byte-level bundle construction
# ---------------------------------------------------------------------------


def _pack_header(
    *,
    complete: int,
    assignment_format: int,
    directory_offset: int,
    directory_len: int,
    stream_offset: int,
    stream_len: int,
    sample_count: int,
    magic: bytes = BENDL_MAGIC,
    major_version: int = BENDL_MAJOR_VERSION,
    minor_version: int = BENDL_MINOR_VERSION,
    flags: int = 0,
    stream_checksum: int = 0,
    reserved_0: int = 0,
) -> bytes:
    if len(magic) != 8:
        raise ValueError("magic must be 8 bytes")
    return magic + struct.pack(
        "<HHBBHIIQQQQq",
        major_version,
        minor_version,
        complete,
        assignment_format,
        reserved_0,
        flags,
        stream_checksum,
        directory_offset,
        directory_len,
        stream_offset,
        stream_len,
        sample_count,
    )


def _pack_directory_entry(
    *,
    asset_type: int,
    asset_flags: int,
    name: str,
    payload_offset: int,
    payload_len: int,
    checksum: Optional[bytes] = None,
) -> bytes:
    name_bytes = name.encode("utf-8")
    checksum_bytes = checksum or b""
    header = struct.pack(
        "<HHHHQQI",
        asset_type,
        asset_flags,
        len(name_bytes),
        0,
        payload_offset,
        payload_len,
        len(checksum_bytes),
    )
    return header + name_bytes + checksum_bytes


def _pack_directory(entries: Iterable[bytes]) -> bytes:
    entries = list(entries)
    return struct.pack("<I", len(entries)) + b"".join(entries)


def _xz(data: bytes) -> bytes:
    return lzma.compress(data, format=lzma.FORMAT_XZ, preset=6)


class _Asset:
    def __init__(
        self,
        *,
        asset_type: int,
        name: str,
        payload: bytes,
        is_json: bool = False,
        compress: bool = False,
        checksum: Optional[bytes] = None,
    ) -> None:
        self.asset_type = asset_type
        self.name = name
        self.raw_payload = payload
        self.is_json = is_json
        self.compress = compress
        self.checksum = checksum

    def encoded_bytes(self) -> bytes:
        return _xz(self.raw_payload) if self.compress else self.raw_payload

    def flags(self, *, has_checksum: bool) -> int:
        flags = 0
        if self.is_json:
            flags |= ASSET_FLAG_JSON
        if self.compress:
            flags |= ASSET_FLAG_XZ
        if has_checksum:
            flags |= ASSET_FLAG_CHECKSUM
        return flags


def build_bundle(
    *,
    stream_bytes: bytes,
    sample_count: int,
    assignment_format: int = ASSIGNMENT_FORMAT_BEN,
    assets: Iterable[_Asset] = (),
    complete: int = COMPLETE_YES,
    magic: bytes = BENDL_MAGIC,
    major_version: int = BENDL_MAJOR_VERSION,
    checksums: bool = True,
) -> bytes:
    assets = list(assets)
    buf = bytearray()
    buf.extend(b"\x00" * HEADER_SIZE)

    encoded_assets: List[Tuple[int, int, bytes]] = []
    for asset in assets:
        offset = len(buf)
        encoded = asset.encoded_bytes()
        buf.extend(encoded)
        encoded_assets.append((offset, len(encoded), encoded))

    stream_offset = len(buf)
    buf.extend(stream_bytes)
    stream_len = len(stream_bytes)

    directory_offset = len(buf)
    entries_bytes: List[bytes] = []
    for (offset, length, encoded), asset in zip(encoded_assets, assets):
        checksum = asset.checksum
        if checksums and checksum is None:
            checksum = struct.pack("<I", _crc32c(encoded))
        entries_bytes.append(
            _pack_directory_entry(
                asset_type=asset.asset_type,
                asset_flags=asset.flags(has_checksum=checksum is not None),
                name=asset.name,
                payload_offset=offset,
                payload_len=length,
                checksum=checksum,
            )
        )
    directory = _pack_directory(entries_bytes)
    buf.extend(directory)
    directory_len = len(directory)

    header_flags = 0
    stream_checksum = 0
    if checksums and complete == COMPLETE_YES:
        header_flags |= HEADER_FLAG_STREAM_CHECKSUM
        stream_checksum = _crc32c(stream_bytes)

    header = _pack_header(
        complete=complete,
        assignment_format=assignment_format,
        directory_offset=directory_offset,
        directory_len=directory_len,
        stream_offset=stream_offset,
        stream_len=stream_len,
        sample_count=sample_count,
        magic=magic,
        major_version=major_version,
        flags=header_flags,
        stream_checksum=stream_checksum,
    )
    buf[:HEADER_SIZE] = header
    return bytes(buf)


# ---------------------------------------------------------------------------
# Real BEN/XBEN stream helpers
# ---------------------------------------------------------------------------


def _write_jsonl(samples: List[List[int]], path: Path) -> None:
    with path.open("w", encoding="utf-8") as f:
        for i, a in enumerate(samples, start=1):
            json.dump({"assignment": a, "sample": i}, f, separators=(",", ":"))
            f.write("\n")


def _ben_bytes_for(samples: List[List[int]], tmp: Path, variant: str = "standard") -> bytes:
    ben_path = tmp / "inner.ben"
    with BenEncoder(ben_path, overwrite=True, variant=variant) as enc:
        for a in samples:
            enc.write(a)
    return ben_path.read_bytes()


def _xben_bytes_for(samples: List[List[int]], tmp: Path, variant: str = "standard") -> bytes:
    src = tmp / "src.jsonl"
    _write_jsonl(samples, src)
    out = tmp / "inner.xben"
    encode_jsonl_to_xben(
        src, out, overwrite=True, variant=variant, n_threads=1, compression_level=1
    )
    return out.read_bytes()


def _write_bundle(path: Path, bundle_bytes: bytes) -> Path:
    path.write_bytes(bundle_bytes)
    return path


# ---------------------------------------------------------------------------
# Happy path
# ---------------------------------------------------------------------------


def test_bundle_round_trip_ben_with_assets(tmp_path: Path) -> None:
    rng = random.Random(4242)
    samples = [[rng.randint(1, 10) for _ in range(rng.randint(1, 50))] for _ in range(40)]
    # NetworkX adjacency format (what read_graph rebuilds into a live graph).
    graph_json = (
        b'{"directed":false,"multigraph":false,"graph":{},'
        b'"nodes":[{"id":0},{"id":1},{"id":2},{"id":3}],'
        b'"adjacency":[[{"id":1}],[{"id":0},{"id":2}],[{"id":1},{"id":3}],[{"id":2}]]}'
    )
    metadata_json = b'{"note":"hello bundle","seed":4242}'
    perm_json = b'{"node_permutation_old_to_new":{"0":1,"1":0}}'
    custom_blob = bytes(range(256))

    bundle = build_bundle(
        stream_bytes=_ben_bytes_for(samples, tmp_path),
        sample_count=len(samples),
        assets=[
            _Asset(
                asset_type=ASSET_TYPE_METADATA,
                name="metadata.json",
                payload=metadata_json,
                is_json=True,
            ),
            _Asset(
                asset_type=ASSET_TYPE_GRAPH,
                name="graph.json",
                payload=graph_json,
                is_json=True,
                compress=True,
            ),
            _Asset(
                asset_type=ASSET_TYPE_NODE_PERMUTATION_MAP,
                name="node_permutation_map.json",
                payload=perm_json,
                is_json=True,
            ),
            _Asset(asset_type=ASSET_TYPE_CUSTOM, name="notes.bin", payload=custom_blob),
        ],
    )
    path = _write_bundle(tmp_path / "out.bendl", bundle)
    dec = BendlDecoder(path)

    assert dec.version() == (BENDL_MAJOR_VERSION, BENDL_MINOR_VERSION)
    assert dec.is_complete() is True
    assert dec.count_samples() == len(samples)
    assert dec.assignment_format() == "ben"
    assert dec.asset_names() == [
        "metadata.json",
        "graph.json",
        "node_permutation_map.json",
        "notes.bin",
    ]

    by_name = {a["name"]: a for a in dec.list_assets()}
    assert by_name["graph.json"]["type"] == ASSET_TYPE_GRAPH
    assert "xz" in by_name["graph.json"]["flags"]
    assert by_name["notes.bin"]["flags"] == ["checksum"]
    for entry in dec.list_assets():
        assert entry["offset"] >= HEADER_SIZE

    assert dec.read_asset_bytes("graph.json") == graph_json
    assert dec.read_asset_bytes("notes.bin") == custom_blob
    assert dec.read_metadata() == json.loads(metadata_json)
    # read_graph() rebuilds a live NetworkX graph; raw JSON stays on read_json_asset.
    assert dec.read_json_asset("graph.json") == json.loads(graph_json)
    graph_obj = dec.read_graph()
    assert sorted(graph_obj.nodes) == [0, 1, 2, 3]
    assert {tuple(sorted(e)) for e in graph_obj.edges} == {(0, 1), (1, 2), (2, 3)}
    assert dec.read_node_permutation_map() == json.loads(perm_json)
    assert dec.read_json_asset("metadata.json") == json.loads(metadata_json)

    extracted = tmp_path / "stream.ben"
    dec.extract_stream(extracted)
    assert list(BenDecoder(extracted, mode="ben")) == samples
    assert repr(dec) is not None


def test_bundle_round_trip_xben(tmp_path: Path) -> None:
    samples = [[1, 2, 3], [1, 2, 3], [4, 4, 5], [6, 7, 8]]
    bundle = build_bundle(
        stream_bytes=_xben_bytes_for(samples, tmp_path, variant="mkv_chain"),
        sample_count=len(samples),
        assignment_format=ASSIGNMENT_FORMAT_XBEN,
    )
    path = _write_bundle(tmp_path / "xout.bendl", bundle)
    dec = BendlDecoder(path)
    assert dec.assignment_format() == "xben"
    assert dec.count_samples() == len(samples)
    assert dec.asset_names() == []
    extracted = tmp_path / "stream.xben"
    dec.extract_stream(extracted)
    assert list(BenDecoder(extracted, mode="xben")) == samples


def test_canonical_helpers_return_none_when_absent(tmp_path: Path) -> None:
    bundle = build_bundle(
        stream_bytes=_ben_bytes_for([[1, 2, 3]], tmp_path),
        sample_count=1,
        assets=[_Asset(asset_type=ASSET_TYPE_CUSTOM, name="only_custom.bin", payload=b"x")],
    )
    path = _write_bundle(tmp_path / "sparse.bendl", bundle)
    dec = BendlDecoder(path)
    assert dec.read_metadata() is None
    assert dec.read_graph() is None
    assert dec.read_node_permutation_map() is None


def test_asset_free_empty_stream(tmp_path: Path) -> None:
    bundle = build_bundle(stream_bytes=b"", sample_count=0)
    path = _write_bundle(tmp_path / "empty.bendl", bundle)
    dec = BendlDecoder(path)
    assert dec.is_complete()
    assert dec.count_samples() == 0
    assert len(dec) == 0
    assert list(dec) == []
    assert dec.asset_names() == []
    out = tmp_path / "empty.ben"
    dec.extract_stream(out)
    assert out.read_bytes() == b""


def test_banner_only_zero_frame_stream(tmp_path: Path) -> None:
    # A real BEN banner with zero frames iterates to [] and counts 0.
    bundle = build_bundle(stream_bytes=_ben_bytes_for([], tmp_path), sample_count=0)
    path = _write_bundle(tmp_path / "banner.bendl", bundle)
    dec = BendlDecoder(path)
    assert len(dec) == 0
    assert list(dec) == []


# ---------------------------------------------------------------------------
# Asset lookup / JSON parsing
# ---------------------------------------------------------------------------


def test_read_asset_bytes_raises_keyerror_for_unknown_name(tmp_path: Path) -> None:
    bundle = build_bundle(
        stream_bytes=_ben_bytes_for([[1, 2]], tmp_path),
        sample_count=1,
        assets=[_Asset(asset_type=ASSET_TYPE_CUSTOM, name="there.bin", payload=b"x")],
    )
    path = _write_bundle(tmp_path / "x.bendl", bundle)
    dec = BendlDecoder(path)
    with pytest.raises(KeyError, match="no asset named"):
        dec.read_asset_bytes("missing.bin")
    with pytest.raises(KeyError):
        dec.read_json_asset("missing.json")


def test_read_json_asset_rejects_non_utf8(tmp_path: Path) -> None:
    bundle = build_bundle(
        stream_bytes=_ben_bytes_for([[1, 2]], tmp_path),
        sample_count=1,
        assets=[_Asset(asset_type=ASSET_TYPE_CUSTOM, name="binary.bin", payload=b"\xff\xfe\xfd")],
    )
    path = _write_bundle(tmp_path / "bin.bendl", bundle)
    dec = BendlDecoder(path)
    assert dec.read_asset_bytes("binary.bin") == b"\xff\xfe\xfd"
    with pytest.raises(Exception, match="not valid UTF-8"):
        dec.read_json_asset("binary.bin")


def test_read_json_asset_rejects_malformed_json(tmp_path: Path) -> None:
    bundle = build_bundle(
        stream_bytes=_ben_bytes_for([[1]], tmp_path),
        sample_count=1,
        assets=[
            _Asset(
                asset_type=ASSET_TYPE_METADATA,
                name="metadata.json",
                payload=b"not a json {{{",
                is_json=True,
            )
        ],
    )
    path = _write_bundle(tmp_path / "m.bendl", bundle)
    dec = BendlDecoder(path)
    assert dec.read_asset_bytes("metadata.json") == b"not a json {{{"
    with pytest.raises(json.JSONDecodeError):
        dec.read_metadata()


def test_unicode_asset_name_round_trips(tmp_path: Path) -> None:
    name = "tëst_ääää_✓.bin"
    bundle = build_bundle(
        stream_bytes=_ben_bytes_for([[1]], tmp_path),
        sample_count=1,
        assets=[_Asset(asset_type=ASSET_TYPE_CUSTOM, name=name, payload=b"payload")],
    )
    path = _write_bundle(tmp_path / "u.bendl", bundle)
    dec = BendlDecoder(path)
    assert dec.asset_names() == [name]
    assert dec.read_asset_bytes(name) == b"payload"


def test_many_assets_preserve_directory_order(tmp_path: Path) -> None:
    payloads = {f"asset_{i:04d}.bin": bytes([i & 0xFF] * (i + 1)) for i in range(200)}
    assets = [_Asset(asset_type=ASSET_TYPE_CUSTOM, name=n, payload=p) for n, p in payloads.items()]
    bundle = build_bundle(
        stream_bytes=_ben_bytes_for([[1, 2, 3]], tmp_path),
        sample_count=1,
        assets=assets,
    )
    path = _write_bundle(tmp_path / "many.bendl", bundle)
    dec = BendlDecoder(path)
    assert dec.asset_names() == list(payloads.keys())
    for i in (0, 1, 42, 199):
        name = f"asset_{i:04d}.bin"
        assert dec.read_asset_bytes(name) == payloads[name]


def test_list_assets_flag_fidelity(tmp_path: Path) -> None:
    combos: List[Tuple[bool, bool, bool]] = [
        (False, False, False),
        (True, False, False),
        (False, True, False),
        (False, False, True),
        (True, True, False),
        (True, False, True),
        (False, True, True),
        (True, True, True),
    ]
    assets: List[_Asset] = []
    expected: List[List[str]] = []
    for i, (is_json, compress, has_checksum) in enumerate(combos):
        payload = f'{{"i":{i}}}'.encode("utf-8") if is_json else bytes([i % 256]) * 32
        checksum = b"\xde\xad\xbe\xef" if has_checksum else None
        assets.append(
            _Asset(
                asset_type=ASSET_TYPE_CUSTOM,
                name=f"asset-{i}.bin",
                payload=payload,
                is_json=is_json,
                compress=compress,
                checksum=checksum,
            )
        )
        want: List[str] = []
        if is_json:
            want.append("json")
        if compress:
            want.append("xz")
        if has_checksum:
            want.append("checksum")
        expected.append(want)
    bundle = build_bundle(
        stream_bytes=_ben_bytes_for([[1, 2]], tmp_path),
        sample_count=1,
        assets=assets,
        checksums=False,
    )
    path = _write_bundle(tmp_path / "flags.bendl", bundle)
    got = BendlDecoder(path).list_assets()
    for entry, want in zip(got, expected):
        assert entry["flags"] == want


def test_zero_length_custom_payload(tmp_path: Path) -> None:
    bundle = build_bundle(
        stream_bytes=_ben_bytes_for([[1]], tmp_path),
        sample_count=1,
        assets=[_Asset(asset_type=ASSET_TYPE_CUSTOM, name="empty.bin", payload=b"")],
    )
    path = _write_bundle(tmp_path / "zlen.bendl", bundle)
    dec = BendlDecoder(path)
    assert dec.read_asset_bytes("empty.bin") == b""
    entry = next(a for a in dec.list_assets() if a["name"] == "empty.bin")
    assert entry["len"] == 0


# ---------------------------------------------------------------------------
# extract_stream overwrite semantics
# ---------------------------------------------------------------------------


def test_extract_stream_refuses_existing_file_without_overwrite(tmp_path: Path) -> None:
    bundle = build_bundle(stream_bytes=_ben_bytes_for([[1, 2]], tmp_path), sample_count=1)
    path = _write_bundle(tmp_path / "a.bendl", bundle)
    dec = BendlDecoder(path)
    target = tmp_path / "already.ben"
    target.write_bytes(b"pre-existing")
    with pytest.raises(OSError, match="already exists"):
        dec.extract_stream(target)
    assert target.read_bytes() == b"pre-existing"


def test_extract_stream_into_missing_parent_dir_raises(tmp_path: Path) -> None:
    bundle = build_bundle(stream_bytes=_ben_bytes_for([[1, 2]], tmp_path), sample_count=1)
    path = _write_bundle(tmp_path / "mini.bendl", bundle)
    dec = BendlDecoder(path)
    with pytest.raises(OSError):
        dec.extract_stream(tmp_path / "does" / "not" / "exist" / "out.ben")


# ---------------------------------------------------------------------------
# Invalid headers / corrupted bundles
# ---------------------------------------------------------------------------


def test_open_rejects_missing_file(tmp_path: Path) -> None:
    with pytest.raises(OSError, match="Failed to open"):
        BendlDecoder(tmp_path / "does_not_exist.bendl")


def test_open_rejects_plain_stream(tmp_path: Path) -> None:
    plain = tmp_path / "plain.ben"
    with BenEncoder(plain, overwrite=True, variant="standard") as enc:
        enc.write([1, 2, 3])
    with pytest.raises(Exception, match="not a .bendl file"):
        BendlDecoder(plain)


def test_open_rejects_unsupported_major_version(tmp_path: Path) -> None:
    bundle = build_bundle(
        stream_bytes=_ben_bytes_for([[1, 2]], tmp_path),
        sample_count=1,
        major_version=999,
    )
    path = _write_bundle(tmp_path / "oldfuture.bendl", bundle)
    with pytest.raises(Exception, match="Failed to parse bundle header"):
        BendlDecoder(path)


def test_open_rejects_truncated_header(tmp_path: Path) -> None:
    path = tmp_path / "short.bendl"
    path.write_bytes(b"BENDL\x00\x00\x01\x00")
    with pytest.raises(Exception, match="Failed to parse bundle header"):
        BendlDecoder(path)


def test_open_rejects_inflated_entry_count(tmp_path: Path) -> None:
    bundle = bytearray(
        build_bundle(
            stream_bytes=_ben_bytes_for([[1, 2]], tmp_path),
            sample_count=1,
            assets=[_Asset(asset_type=ASSET_TYPE_CUSTOM, name="x", payload=b"abc")],
        )
    )
    directory_offset = struct.unpack_from("<Q", bundle, 24)[0]
    struct.pack_into("<I", bundle, directory_offset, 9999)
    path = _write_bundle(tmp_path / "trunc_dir.bendl", bytes(bundle))
    with pytest.raises(Exception):
        BendlDecoder(path)


def test_open_rejects_chopped_directory(tmp_path: Path) -> None:
    bundle = build_bundle(
        stream_bytes=_ben_bytes_for([[1, 2]], tmp_path),
        sample_count=1,
        assets=[_Asset(asset_type=ASSET_TYPE_CUSTOM, name="x", payload=b"abc")],
    )
    path = _write_bundle(tmp_path / "chop.bendl", bundle[:-2])
    with pytest.raises(Exception):
        BendlDecoder(path)


def test_open_rejects_malformed_directory_invariants(tmp_path: Path) -> None:
    stream = _ben_bytes_for([[1, 2]], tmp_path)
    dup = build_bundle(
        stream_bytes=stream,
        sample_count=1,
        assets=[
            _Asset(asset_type=ASSET_TYPE_CUSTOM, name="dup.bin", payload=b"a"),
            _Asset(asset_type=ASSET_TYPE_CUSTOM, name="dup.bin", payload=b"b"),
        ],
    )
    with pytest.raises(Exception, match="malformed directory"):
        BendlDecoder(_write_bundle(tmp_path / "dup.bendl", dup))
    wrong = build_bundle(
        stream_bytes=stream,
        sample_count=1,
        assets=[
            _Asset(
                asset_type=ASSET_TYPE_METADATA,
                name="not_metadata.json",
                payload=b"{}",
                is_json=True,
            )
        ],
    )
    with pytest.raises(Exception, match="malformed directory"):
        BendlDecoder(_write_bundle(tmp_path / "singleton.bendl", wrong))


def test_open_rejects_trailing_directory_bytes(tmp_path: Path) -> None:
    bundle = bytearray(
        build_bundle(
            stream_bytes=_ben_bytes_for([[1, 2]], tmp_path),
            sample_count=1,
            assets=[_Asset(asset_type=ASSET_TYPE_CUSTOM, name="x", payload=b"abc")],
        )
    )
    directory_len = struct.unpack_from("<Q", bundle, 32)[0]
    struct.pack_into("<Q", bundle, 32, directory_len + 1)
    bundle.append(0)
    path = _write_bundle(tmp_path / "trailing.bendl", bytes(bundle))
    with pytest.raises(Exception, match="trailing byte"):
        BendlDecoder(path)


def test_unknown_assignment_format_rejected(tmp_path: Path) -> None:
    bundle = bytearray(build_bundle(stream_bytes=b"", sample_count=0))
    bundle[13] = 99
    path = _write_bundle(tmp_path / "wtfmt.bendl", bytes(bundle))
    with pytest.raises(Exception, match="unrecognized assignment_format"):
        BendlDecoder(path)


def test_corrupted_xz_asset_raises(tmp_path: Path) -> None:
    bundle = bytearray(
        build_bundle(
            stream_bytes=_ben_bytes_for([[1, 2]], tmp_path),
            sample_count=1,
            assets=[
                _Asset(
                    asset_type=ASSET_TYPE_GRAPH,
                    name="graph.json",
                    payload=b'{"nodes":[0,1,2,3,4,5,6,7,8,9]}',
                    is_json=True,
                    compress=True,
                )
            ],
        )
    )
    xz_start = bundle.find(b"\xfd7zXZ")
    assert xz_start != -1
    bundle[xz_start + 20] ^= 0xFF
    path = _write_bundle(tmp_path / "badxz.bendl", bytes(bundle))
    dec = BendlDecoder(path)
    with pytest.raises(OSError):
        dec.read_asset_bytes("graph.json")


# ---------------------------------------------------------------------------
# Incomplete / truncated streams
# ---------------------------------------------------------------------------


def _incomplete_bundle(stream_bytes: bytes, stream_len: int = 0) -> bytes:
    header = _pack_header(
        complete=COMPLETE_NO,
        assignment_format=ASSIGNMENT_FORMAT_BEN,
        directory_offset=0,
        directory_len=0,
        stream_offset=HEADER_SIZE,
        stream_len=stream_len,
        sample_count=-1,
    )
    return header + stream_bytes


def test_incomplete_bundle_scans_for_sample_count(tmp_path: Path) -> None:
    stream = _ben_bytes_for([[1, 2, 3]], tmp_path)
    path = _write_bundle(tmp_path / "incomplete.bendl", _incomplete_bundle(stream))
    dec = BendlDecoder(path)
    assert dec.is_complete() is False
    assert dec.count_samples() == 1
    assert dec.asset_names() == []
    out = tmp_path / "extracted.ben"
    with pytest.raises(Exception, match="unfinalized"):
        dec.extract_stream(out)
    dec.extract_stream(out, overwrite=True, allow_unfinalized=True)
    assert list(BenDecoder(out, mode="ben")) == [[1, 2, 3]]


def test_interrupted_ben_stream_decodes_valid_prefix(tmp_path: Path) -> None:
    samples = [[1, 1, 2, 2], [3, 3, 4, 4], [5, 5, 6, 6], [7, 7, 8, 8], [9, 9, 9, 9]]
    full_ben = _ben_bytes_for(samples, tmp_path)
    partial = full_ben[: len(full_ben) - 3]
    path = _write_bundle(tmp_path / "crashed.bendl", _incomplete_bundle(partial))
    dec = BendlDecoder(path)
    assert dec.is_complete() is False
    extracted = tmp_path / "partial.ben"
    with pytest.raises(Exception, match="unfinalized"):
        dec.extract_stream(extracted)
    dec.extract_stream(extracted, overwrite=True, allow_unfinalized=True)
    assert extracted.read_bytes() == partial


def test_interrupted_zero_bytes_after_header(tmp_path: Path) -> None:
    path = _write_bundle(tmp_path / "zero.bendl", _incomplete_bundle(b""))
    dec = BendlDecoder(path)
    assert dec.is_complete() is False
    assert dec.asset_names() == []
    with pytest.raises(Exception):
        dec.count_samples()
    extracted = tmp_path / "zero.ben"
    dec.extract_stream(extracted, overwrite=True, allow_unfinalized=True)
    assert extracted.read_bytes() == b""


def test_finalized_bundle_with_inflated_stream_len_survives_open(
    tmp_path: Path,
) -> None:
    samples = [[1, 2, 3], [4, 5, 6]]
    bundle = bytearray(
        build_bundle(stream_bytes=_ben_bytes_for(samples, tmp_path), sample_count=len(samples))
    )
    old_stream_len = struct.unpack_from("<Q", bundle, 48)[0]
    struct.pack_into("<Q", bundle, 48, old_stream_len + 10_000)
    path = _write_bundle(tmp_path / "liar.bendl", bytes(bundle))
    dec = BendlDecoder(path)
    assert dec.is_complete()
    assert dec.count_samples() == len(samples)
    extracted = tmp_path / "liar.ben"
    try:
        dec.extract_stream(extracted)
    except OSError:
        return
    assert len(extracted.read_bytes()) <= old_stream_len + 10_000


# ---------------------------------------------------------------------------
# Interleaving / idempotence
# ---------------------------------------------------------------------------


def test_read_after_extract_still_works(tmp_path: Path) -> None:
    bundle = build_bundle(
        stream_bytes=_ben_bytes_for([[1, 2], [3, 4]], tmp_path),
        sample_count=2,
        assets=[
            _Asset(
                asset_type=ASSET_TYPE_METADATA,
                name="metadata.json",
                payload=b'{"x":1}',
                is_json=True,
            )
        ],
    )
    path = _write_bundle(tmp_path / "seq.bendl", bundle)
    dec = BendlDecoder(path)
    dec.extract_stream(tmp_path / "s.ben")
    assert dec.read_metadata() == {"x": 1}
    dec.extract_stream(tmp_path / "s2.ben", overwrite=True)
    assert dec.read_asset_bytes("metadata.json") == b'{"x":1}'


def test_toc_interleaved_with_iteration(tmp_path: Path) -> None:
    samples = [[1, 2], [3, 4], [5, 6]]
    bundle = build_bundle(
        stream_bytes=_ben_bytes_for(samples, tmp_path),
        sample_count=len(samples),
        assets=[
            _Asset(
                asset_type=ASSET_TYPE_METADATA,
                name="metadata.json",
                payload=b'{"tag":42}',
                is_json=True,
            )
        ],
    )
    path = _write_bundle(tmp_path / "interleave.bendl", bundle)
    dec = BendlDecoder(path)
    it = iter(dec)
    assert next(it) == samples[0]
    assert dec.read_metadata() == {"tag": 42}
    assert next(it) == samples[1]
    assert dec.read_asset_bytes("metadata.json") == b'{"tag":42}'
    assert next(it) == samples[2]
    with pytest.raises(StopIteration):
        next(it)


def test_read_asset_bytes_idempotent(tmp_path: Path) -> None:
    payload = b"repeat-me " * 100
    bundle = build_bundle(
        stream_bytes=_ben_bytes_for([[1, 2]], tmp_path),
        sample_count=1,
        assets=[
            _Asset(asset_type=ASSET_TYPE_CUSTOM, name="raw.bin", payload=payload),
            _Asset(
                asset_type=ASSET_TYPE_CUSTOM,
                name="compressed.bin",
                payload=payload,
                compress=True,
            ),
        ],
    )
    path = _write_bundle(tmp_path / "idem.bendl", bundle)
    dec = BendlDecoder(path)
    for _ in range(5):
        assert dec.read_asset_bytes("raw.bin") == payload
        assert dec.read_asset_bytes("compressed.bin") == payload


# ---------------------------------------------------------------------------
# Iteration restart and subsampling
# ---------------------------------------------------------------------------


def test_iteration_can_restart(tmp_path: Path) -> None:
    samples = [[1, 2], [3, 4], [5, 6]]
    bundle = build_bundle(stream_bytes=_ben_bytes_for(samples, tmp_path), sample_count=len(samples))
    path = _write_bundle(tmp_path / "twice.bendl", bundle)
    dec = BendlDecoder(path)
    assert list(dec) == samples
    assert list(dec) == samples


def test_partial_iteration_then_restart(tmp_path: Path) -> None:
    samples = [[1, 2], [3, 4], [5, 6], [7, 8]]
    bundle = build_bundle(stream_bytes=_ben_bytes_for(samples, tmp_path), sample_count=len(samples))
    path = _write_bundle(tmp_path / "partial.bendl", bundle)
    dec = BendlDecoder(path)
    it = iter(dec)
    assert next(it) == samples[0]
    assert next(it) == samples[1]
    assert list(dec) == samples


def test_subsample_modes(tmp_path: Path) -> None:
    samples = [[i] for i in range(1, 11)]
    bundle = build_bundle(stream_bytes=_ben_bytes_for(samples, tmp_path), sample_count=len(samples))
    path = _write_bundle(tmp_path / "sub.bendl", bundle)

    dec = BendlDecoder(path).subsample_range(3, 6)
    assert list(dec) == samples[2:6]
    assert list(dec) == samples[2:6]  # survives reiteration

    dec2 = BendlDecoder(path).subsample_indices([1, 4, 8])
    assert list(dec2) == [samples[0], samples[3], samples[7]]

    dec3 = BendlDecoder(path).subsample_every(3, 2)
    assert list(dec3) == [samples[1], samples[4], samples[7]]


def test_subsample_count_preserves_filtered_len(tmp_path: Path) -> None:
    samples = [[i] for i in range(1, 9)]
    bundle = build_bundle(stream_bytes=_ben_bytes_for(samples, tmp_path), sample_count=len(samples))
    path = _write_bundle(tmp_path / "cnt.bendl", bundle)
    dec = BendlDecoder(path).subsample_range(2, 5)
    assert len(dec) == 4
    assert dec.count_samples() == len(samples)
    assert len(dec) == 4
    assert list(dec) == samples[1:5]


def test_subsample_out_of_bounds(tmp_path: Path) -> None:
    samples = [[1, 2], [3, 4], [5, 6]]
    bundle = build_bundle(stream_bytes=_ben_bytes_for(samples, tmp_path), sample_count=len(samples))
    path = _write_bundle(tmp_path / "oob.bendl", bundle)
    with pytest.raises(Exception, match="end must be <= number of samples"):
        BendlDecoder(path).subsample_range(1, 99)
    with pytest.raises(Exception, match="number of samples"):
        BendlDecoder(path).subsample_indices([1, 42])
    with pytest.warns(UserWarning, match="sorted and unique"):
        dec = BendlDecoder(path).subsample_indices([3, 1, 3, 1])
    assert list(dec) == [samples[0], samples[2]]


def test_len_uses_header_fast_path(tmp_path: Path) -> None:
    samples = [[i] for i in range(1, 6)]
    bundle = build_bundle(stream_bytes=_ben_bytes_for(samples, tmp_path), sample_count=len(samples))
    path = _write_bundle(tmp_path / "fast.bendl", bundle)
    dec = BendlDecoder(path)
    assert len(dec) == len(samples)
    assert len(dec) == len(samples)
    assert dec.count_samples() == len(samples)


def test_seeded_fuzz_random_bundles_round_trip(tmp_path: Path) -> None:
    rng = random.Random(0xFEED_FACE)
    for trial in range(15):
        n_assets = rng.randint(0, 10)
        assets: List[_Asset] = []
        truth: List[Tuple[str, bytes]] = []
        for i in range(n_assets):
            payload = rng.randbytes(rng.choice([0, 1, 7, 64, 500]))
            assets.append(
                _Asset(
                    asset_type=ASSET_TYPE_CUSTOM,
                    name=f"t{trial}-a{i}.bin",
                    payload=payload,
                    compress=rng.random() < 0.4,
                )
            )
            truth.append((f"t{trial}-a{i}.bin", payload))
        n_samples = rng.randint(1, 25)
        samples = [[rng.randint(1, 8) for _ in range(rng.randint(1, 40))] for _ in range(n_samples)]
        bundle = build_bundle(
            stream_bytes=_ben_bytes_for(samples, tmp_path),
            sample_count=n_samples,
            assets=assets,
        )
        path = _write_bundle(tmp_path / f"fuzz-{trial}.bendl", bundle)
        dec = BendlDecoder(path)
        assert dec.count_samples() == n_samples
        assert dec.asset_names() == [name for name, _ in truth]
        for name, want in truth:
            assert dec.read_asset_bytes(name) == want
        extracted = tmp_path / f"fuzz-{trial}.ben"
        dec.extract_stream(extracted)
        assert list(BenDecoder(extracted, mode="ben")) == samples


# ---------------------------------------------------------------------------
# verify(): explicit integrity checking
# ---------------------------------------------------------------------------


def _checksummed_bundle(path: Path) -> None:
    """A small finalized bundle written by the real encoder (checksums populated)."""
    with BendlEncoder(path, overwrite=True) as enc:
        enc.add_asset("notes.txt", "integrity matters", content_type="text")
        with enc.stream(variant="standard") as s:
            for a in ([1, 1, 2, 2], [2, 2, 1, 1]):
                s.write(a)


def _flip_byte_at_marker(path: Path, marker: bytes) -> None:
    """XOR one byte at the first occurrence of ``marker`` in the file."""
    data = bytearray(path.read_bytes())
    idx = data.index(marker)
    data[idx] ^= 0xFF
    path.write_bytes(bytes(data))


def test_verify_passes_on_pristine_bundle(tmp_path: Path) -> None:
    path = tmp_path / "ok.bendl"
    _checksummed_bundle(path)
    BendlDecoder(path).verify()  # must not raise


def test_verify_catches_stream_corruption(tmp_path: Path) -> None:
    # Iteration and subsampling read the stream without checksum verification (partial reads
    # cannot prove a whole-stream CRC); verify() is the explicit integrity gate and must catch
    # any byte flip in the stream region.
    path = tmp_path / "stream-corrupt.bendl"
    _checksummed_bundle(path)
    _flip_byte_at_marker(path, b"STANDARD BEN FILE")

    dec = BendlDecoder(path)  # directory is intact, so the bundle still opens
    with pytest.raises(Exception, match="stream verification failed"):
        dec.verify()


def test_verify_catches_asset_corruption(tmp_path: Path) -> None:
    path = tmp_path / "asset-corrupt.bendl"
    _checksummed_bundle(path)
    _flip_byte_at_marker(path, b"integrity matters")

    dec = BendlDecoder(path)
    with pytest.raises(Exception, match="asset verification failed"):
        dec.verify()


def test_verify_rejects_unfinalized_bundle(tmp_path: Path) -> None:
    # An unfinalized bundle's stream checksum is not authoritative, so verify() must refuse
    # rather than report a meaningless pass/fail.
    path = tmp_path / "unfinalized.bendl"
    with pytest.raises(RuntimeError, match="boom"):
        with BendlEncoder(path, overwrite=True) as enc:
            with enc.stream() as s:
                s.write([1, 2, 3])
                raise RuntimeError("boom")

    dec = BendlDecoder(path)
    with pytest.raises(Exception, match="stream verification failed"):
        dec.verify()
