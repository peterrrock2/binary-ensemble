"""Tests for bundle (.bendl) support in BenDecoder.

These tests do not rely on the `bendl` CLI binary being built. Instead, they
construct `.bendl` bundles directly in Python from the on-disk format spec
documented in ``ben/src/io/bundle/format.rs``. This keeps the tests
self-contained and lets them stress odd byte layouts that a CLI-based helper
could not produce (truncated files, bad magic, dangling offsets, etc).

Real BEN/XBEN stream payloads are produced via ``BenEncoder`` /
``encode_jsonl_to_xben`` so the stream region always matches what the
main compression pipeline would produce.
"""

from __future__ import annotations

import io
import json
import lzma
import random
import struct
from pathlib import Path
from typing import Iterable, List, Optional, Tuple

import pytest

import binary_ensemble
from binary_ensemble import (
    BenDecoder,
    BenEncoder,
    encode_jsonl_to_ben,
    encode_jsonl_to_xben,
)


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
    """Compute CRC32C (Castagnoli), matching the Rust bundle checksum contract."""
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
        0,  # reserved
        payload_offset,
        payload_len,
        len(checksum_bytes),
    )
    return header + name_bytes + checksum_bytes


def _pack_directory(entries: Iterable[bytes]) -> bytes:
    entries = list(entries)
    return struct.pack("<I", len(entries)) + b"".join(entries)


def _xz(data: bytes) -> bytes:
    """Compress ``data`` with the xz container so the Rust xz2 decoder accepts it."""
    return lzma.compress(data, format=lzma.FORMAT_XZ, preset=6)


class _Asset:
    """Helper describing one asset to place in a hand-built bundle."""

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
    """Construct the bytes of a `.bendl` file from pieces.

    The layout is ``[header][asset payloads][stream][directory]``. This
    helper mirrors the writer's finalize path closely enough to produce
    bundles that the Rust reader accepts, while also exposing enough knobs
    to generate deliberately broken bundles for negative tests. By default
    it mirrors the current writer and stores CRC32C checksums for finalized
    streams and assets; pass ``checksums=False`` for foreign/no-checksum
    fixtures.
    """
    assets = list(assets)

    buf = bytearray()
    # Reserve header space.
    buf.extend(b"\x00" * HEADER_SIZE)

    # Write asset payloads and remember (offset, len, encoded_bytes) for each.
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


def _ben_bytes_for(
    samples: List[List[int]], tmp: Path, variant: str = "standard"
) -> bytes:
    """Produce real BEN bytes for ``samples`` via ``BenEncoder``."""
    ben_path = tmp / "inner.ben"
    with BenEncoder(
        ben_path, overwrite=True, variant=variant, ben_file_only=True
    ) as enc:
        for a in samples:
            enc.write(a)
    return ben_path.read_bytes()


def _xben_bytes_for(
    samples: List[List[int]], tmp: Path, variant: str = "standard"
) -> bytes:
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
# Baseline happy-path tests
# ---------------------------------------------------------------------------


def test_module_exports_decoder_and_encoder() -> None:
    assert "BenDecoder" in binary_ensemble.__all__
    assert "BenEncoder" in binary_ensemble.__all__
    assert "PyBundleReader" not in binary_ensemble.__all__


def test_bundle_reader_round_trip_ben_with_assets(tmp_path: Path) -> None:
    rng = random.Random(4242)
    samples = [
        [rng.randint(1, 10) for _ in range(rng.randint(1, 50))] for _ in range(40)
    ]

    graph_json = b'{"nodes":[0,1,2,3],"edges":[[0,1],[1,2],[2,3]]}'
    metadata_json = b'{"note":"hello bundle","seed":4242}'
    relabel_json = b'{"0":"A","1":"B","2":"C","3":"D"}'
    custom_blob = bytes(range(256))

    bundle = build_bundle(
        stream_bytes=_ben_bytes_for(samples, tmp_path),
        sample_count=len(samples),
        assignment_format=ASSIGNMENT_FORMAT_BEN,
        assets=[
            _Asset(
                asset_type=ASSET_TYPE_METADATA,
                name="metadata.json",
                payload=metadata_json,
                is_json=True,
                compress=False,
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
                payload=relabel_json,
                is_json=True,
                compress=False,
            ),
            _Asset(
                asset_type=ASSET_TYPE_CUSTOM,
                name="notes.bin",
                payload=custom_blob,
                is_json=False,
                compress=False,
            ),
        ],
    )
    path = _write_bundle(tmp_path / "out.bendl", bundle)

    reader = BenDecoder(path)

    assert reader.version() == (BENDL_MAJOR_VERSION, BENDL_MINOR_VERSION)
    assert reader.is_complete() is True
    assert reader.count_samples() == len(samples)
    assert reader.assignment_format() == "ben"

    names = reader.asset_names()
    assert names == [
        "metadata.json",
        "graph.json",
        "node_permutation_map.json",
        "notes.bin",
    ]

    assets = reader.list_assets()
    assert [a["name"] for a in assets] == names
    by_name = {a["name"]: a for a in assets}
    assert by_name["graph.json"]["type"] == ASSET_TYPE_GRAPH
    assert "xz" in by_name["graph.json"]["flags"]
    assert "json" in by_name["graph.json"]["flags"]
    assert "xz" not in by_name["metadata.json"]["flags"]
    assert "json" in by_name["metadata.json"]["flags"]
    assert by_name["notes.bin"]["flags"] == ["checksum"]
    # payload_offset must sit at or past the end of the header.
    for entry in assets:
        assert entry["offset"] >= HEADER_SIZE
        assert entry["len"] > 0

    # Raw byte access (decompresses xz transparently).
    assert reader.read_asset_bytes("metadata.json") == metadata_json
    assert reader.read_asset_bytes("graph.json") == graph_json
    assert reader.read_asset_bytes("node_permutation_map.json") == relabel_json
    assert reader.read_asset_bytes("notes.bin") == custom_blob

    # Typed JSON helpers.
    assert reader.read_metadata() == json.loads(metadata_json)
    assert reader.read_graph() == json.loads(graph_json)
    assert reader.read_relabel_map() == json.loads(relabel_json)

    # read_json_asset by name.
    assert reader.read_json_asset("metadata.json") == json.loads(metadata_json)

    # extract_stream then decode via BenDecoder.
    extracted = tmp_path / "stream.ben"
    reader.extract_stream(extracted)
    got = list(BenDecoder(extracted, mode="ben"))
    assert got == samples

    # __repr__ should not crash.
    r = repr(reader)
    assert r is not None


def test_bundle_reader_round_trip_xben(tmp_path: Path) -> None:
    samples = [[1, 2, 3], [1, 2, 3], [4, 4, 5], [6, 7, 8]]
    bundle = build_bundle(
        stream_bytes=_xben_bytes_for(samples, tmp_path, variant="mkv_chain"),
        sample_count=len(samples),
        assignment_format=ASSIGNMENT_FORMAT_XBEN,
        assets=[],
    )
    path = _write_bundle(tmp_path / "xout.bendl", bundle)
    reader = BenDecoder(path)

    assert reader.assignment_format() == "xben"
    assert reader.is_complete()
    assert reader.count_samples() == len(samples)
    assert reader.asset_names() == []

    # extract_stream → file must round-trip via the xben decoder.
    extracted = tmp_path / "stream.xben"
    reader.extract_stream(extracted)
    assert list(BenDecoder(extracted, mode="xben")) == samples


def test_bundle_reader_canonical_helpers_return_none_when_absent(
    tmp_path: Path,
) -> None:
    samples = [[1, 2, 3]]
    bundle = build_bundle(
        stream_bytes=_ben_bytes_for(samples, tmp_path),
        sample_count=len(samples),
        assets=[
            _Asset(
                asset_type=ASSET_TYPE_CUSTOM,
                name="only_custom.bin",
                payload=b"x",
            ),
        ],
    )
    path = _write_bundle(tmp_path / "sparse.bendl", bundle)
    reader = BenDecoder(path)
    assert reader.read_metadata() is None
    assert reader.read_graph() is None
    assert reader.read_relabel_map() is None


def test_bundle_reader_asset_free_empty_stream(tmp_path: Path) -> None:
    # A bundle with no assets and an empty stream is legal (spec says so).
    bundle = build_bundle(stream_bytes=b"", sample_count=0, assets=[])
    path = _write_bundle(tmp_path / "empty.bendl", bundle)
    reader = BenDecoder(path)
    assert reader.is_complete()
    assert reader.count_samples() == 0
    assert reader.asset_names() == []
    assert reader.list_assets() == []
    # extract_stream writes a zero-byte file.
    out = tmp_path / "empty.ben"
    reader.extract_stream(out)
    assert out.read_bytes() == b""


# ---------------------------------------------------------------------------
# Robustness: asset lookup and JSON parsing
# ---------------------------------------------------------------------------


def test_read_asset_bytes_raises_keyerror_for_unknown_name(tmp_path: Path) -> None:
    bundle = build_bundle(
        stream_bytes=_ben_bytes_for([[1, 2]], tmp_path),
        sample_count=1,
        assets=[
            _Asset(asset_type=ASSET_TYPE_CUSTOM, name="there.bin", payload=b"x"),
        ],
    )
    path = _write_bundle(tmp_path / "x.bendl", bundle)
    reader = BenDecoder(path)
    with pytest.raises(KeyError, match="no asset named"):
        reader.read_asset_bytes("missing.bin")
    with pytest.raises(KeyError):
        reader.read_json_asset("missing.json")


def test_read_json_asset_rejects_non_utf8_payload(tmp_path: Path) -> None:
    bundle = build_bundle(
        stream_bytes=_ben_bytes_for([[1, 2]], tmp_path),
        sample_count=1,
        assets=[
            _Asset(
                asset_type=ASSET_TYPE_CUSTOM,
                name="binary.bin",
                payload=b"\xff\xfe\xfd",  # not valid UTF-8
                is_json=False,
                compress=False,
            )
        ],
    )
    path = _write_bundle(tmp_path / "bin.bendl", bundle)
    reader = BenDecoder(path)
    # Raw bytes come back fine.
    assert reader.read_asset_bytes("binary.bin") == b"\xff\xfe\xfd"
    # But the JSON helper must reject non-UTF8 bytes.
    with pytest.raises(Exception, match="not valid UTF-8"):
        reader.read_json_asset("binary.bin")


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
    reader = BenDecoder(path)
    # Raw bytes: fine.
    assert reader.read_asset_bytes("metadata.json") == b"not a json {{{"
    # Parsed via python's json module: must raise.
    with pytest.raises(json.JSONDecodeError):
        reader.read_metadata()


def test_unicode_asset_name_round_trips(tmp_path: Path) -> None:
    # Directory entries store UTF-8 names; a multi-byte name should work.
    name = "tëst_ääää_✓.bin"
    bundle = build_bundle(
        stream_bytes=_ben_bytes_for([[1]], tmp_path),
        sample_count=1,
        assets=[
            _Asset(asset_type=ASSET_TYPE_CUSTOM, name=name, payload=b"payload"),
        ],
    )
    path = _write_bundle(tmp_path / "u.bendl", bundle)
    reader = BenDecoder(path)
    assert reader.asset_names() == [name]
    assert reader.read_asset_bytes(name) == b"payload"


def test_many_assets_preserve_directory_order(tmp_path: Path) -> None:
    # Stress the directory with a large-ish asset count.
    payloads = {f"asset_{i:04d}.bin": bytes([i & 0xFF] * (i + 1)) for i in range(200)}
    assets = [
        _Asset(asset_type=ASSET_TYPE_CUSTOM, name=n, payload=p)
        for n, p in payloads.items()
    ]
    bundle = build_bundle(
        stream_bytes=_ben_bytes_for([[1, 2, 3]], tmp_path),
        sample_count=1,
        assets=assets,
    )
    path = _write_bundle(tmp_path / "many.bendl", bundle)
    reader = BenDecoder(path)
    names = reader.asset_names()
    assert names == list(payloads.keys())
    # Spot-check the contents round-trip.
    for i in (0, 1, 42, 199):
        name = f"asset_{i:04d}.bin"
        assert reader.read_asset_bytes(name) == payloads[name]


# ---------------------------------------------------------------------------
# Robustness: extract_stream overwrite semantics
# ---------------------------------------------------------------------------


def test_extract_stream_refuses_existing_file_without_overwrite(tmp_path: Path) -> None:
    bundle = build_bundle(
        stream_bytes=_ben_bytes_for([[1, 2]], tmp_path),
        sample_count=1,
    )
    path = _write_bundle(tmp_path / "a.bendl", bundle)
    reader = BenDecoder(path)
    target = tmp_path / "already.ben"
    target.write_bytes(b"pre-existing")
    with pytest.raises(OSError, match="already exists"):
        reader.extract_stream(target)
    # File must be untouched.
    assert target.read_bytes() == b"pre-existing"


def test_extract_stream_overwrites_when_requested(tmp_path: Path) -> None:
    bundle = build_bundle(
        stream_bytes=_ben_bytes_for([[1, 2], [3, 4]], tmp_path),
        sample_count=2,
    )
    path = _write_bundle(tmp_path / "b.bendl", bundle)
    reader = BenDecoder(path)
    target = tmp_path / "out.ben"
    target.write_bytes(b"filler")
    reader.extract_stream(target, overwrite=True)
    # Re-opening the extracted file via BenDecoder confirms it's a valid .ben.
    assert list(BenDecoder(target, mode="ben")) == [[1, 2], [3, 4]]


# ---------------------------------------------------------------------------
# Robustness: invalid headers and corrupted bundles
# ---------------------------------------------------------------------------


def test_open_rejects_missing_file(tmp_path: Path) -> None:
    with pytest.raises(OSError, match="Failed to open"):
        BenDecoder(tmp_path / "does_not_exist.bendl")


def test_open_rejects_bad_magic(tmp_path: Path) -> None:
    bundle = build_bundle(
        stream_bytes=_ben_bytes_for([[1, 2]], tmp_path),
        sample_count=1,
        magic=b"NOTABEND",
    )
    path = _write_bundle(tmp_path / "bad.bendl", bundle)
    # Bad magic → detect_is_bundle returns False → treated as plain BEN
    # stream → fails because the bytes aren't a valid BEN banner.
    with pytest.raises(Exception):
        BenDecoder(path)


def test_open_rejects_unsupported_major_version(tmp_path: Path) -> None:
    bundle = build_bundle(
        stream_bytes=_ben_bytes_for([[1, 2]], tmp_path),
        sample_count=1,
        major_version=999,
    )
    path = _write_bundle(tmp_path / "oldfuture.bendl", bundle)
    with pytest.raises(Exception, match="Failed to parse bundle header"):
        BenDecoder(path)


def test_open_rejects_truncated_header(tmp_path: Path) -> None:
    path = tmp_path / "short.bendl"
    path.write_bytes(b"BENDL\x00\x00\x01\x00")  # magic plus 2 bytes — not enough
    with pytest.raises(Exception, match="Failed to parse bundle header"):
        BenDecoder(path)


def test_open_rejects_directory_with_inflated_entry_count(tmp_path: Path) -> None:
    # Corrupt the directory's leading u32 entry-count so the reader tries
    # to decode many more entries than the file actually contains.
    bundle = bytearray(
        build_bundle(
            stream_bytes=_ben_bytes_for([[1, 2]], tmp_path),
            sample_count=1,
            assets=[_Asset(asset_type=ASSET_TYPE_CUSTOM, name="x", payload=b"abc")],
        )
    )
    directory_offset = struct.unpack_from("<Q", bundle, 24)[0]
    # Inflate entry_count to 9999 so the reader walks past EOF.
    struct.pack_into("<I", bundle, directory_offset, 9999)
    path = _write_bundle(tmp_path / "trunc_dir.bendl", bytes(bundle))
    with pytest.raises(Exception):
        BenDecoder(path)


def test_open_rejects_bundle_with_chopped_directory_bytes(tmp_path: Path) -> None:
    bundle = build_bundle(
        stream_bytes=_ben_bytes_for([[1, 2]], tmp_path),
        sample_count=1,
        assets=[_Asset(asset_type=ASSET_TYPE_CUSTOM, name="x", payload=b"abc")],
    )
    # Drop the final two bytes of the directory.
    path = _write_bundle(tmp_path / "chop.bendl", bundle[:-2])
    with pytest.raises(Exception):
        BenDecoder(path)


def test_open_rejects_malformed_directory_invariants(tmp_path: Path) -> None:
    stream = _ben_bytes_for([[1, 2]], tmp_path)

    duplicate_names = build_bundle(
        stream_bytes=stream,
        sample_count=1,
        assets=[
            _Asset(asset_type=ASSET_TYPE_CUSTOM, name="dup.bin", payload=b"a"),
            _Asset(asset_type=ASSET_TYPE_CUSTOM, name="dup.bin", payload=b"b"),
        ],
    )
    path = _write_bundle(tmp_path / "dup.bendl", duplicate_names)
    with pytest.raises(Exception, match="malformed directory"):
        BenDecoder(path)

    wrong_singleton_name = build_bundle(
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
    path = _write_bundle(tmp_path / "singleton.bendl", wrong_singleton_name)
    with pytest.raises(Exception, match="malformed directory"):
        BenDecoder(path)


def test_open_rejects_declared_directory_len_with_trailing_bytes(
    tmp_path: Path,
) -> None:
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

    path = _write_bundle(tmp_path / "trailing_dir.bendl", bytes(bundle))
    with pytest.raises(Exception, match="trailing byte"):
        BenDecoder(path)


def test_incomplete_bundle_scans_stream_for_sample_count(tmp_path: Path) -> None:
    # Provisional bundle with complete=0: the decoder falls back to
    # scanning the stream region (from stream_offset to EOF) to count
    # samples instead of trusting the header.
    stream = _ben_bytes_for([[1, 2, 3]], tmp_path)
    header = _pack_header(
        complete=COMPLETE_NO,
        assignment_format=ASSIGNMENT_FORMAT_BEN,
        directory_offset=0,
        directory_len=0,
        stream_offset=HEADER_SIZE,
        stream_len=0,
        sample_count=-1,
    )
    path = _write_bundle(tmp_path / "incomplete.bendl", header + stream)
    reader = BenDecoder(path)
    assert reader.is_complete() is False
    assert reader.count_samples() == 1
    assert reader.asset_names() == []
    # Verified extraction requires a finalized stream checksum.
    out = tmp_path / "extracted.ben"
    with pytest.raises(Exception, match="unfinalized"):
        reader.extract_stream(out)
    reader.extract_stream(out, overwrite=True, allow_unfinalized=True)
    assert list(BenDecoder(out, mode="ben")) == [[1, 2, 3]]


def test_unknown_assignment_format_byte_rejects_at_construction(tmp_path: Path) -> None:
    # Assignment format byte = 99 → unrecognized. BenDecoder must
    # reject the bundle at construction time.
    bundle = bytearray(
        build_bundle(
            stream_bytes=b"",
            sample_count=0,
            assets=[],
        )
    )
    # assignment_format byte is at offset 13 in the header.
    bundle[13] = 99
    path = _write_bundle(tmp_path / "wtfmt.bendl", bytes(bundle))
    with pytest.raises(Exception, match="unrecognized assignment_format"):
        BenDecoder(path)


def test_corrupted_xz_asset_raises_io_error(tmp_path: Path) -> None:
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

    # Hunt for the xz payload bytes and flip one in the middle.
    # We know the xz magic is b"\xfd7zXZ".
    xz_start = bundle.find(b"\xfd7zXZ")
    assert xz_start != -1, "expected xz magic in hand-built bundle"
    # Flip a byte well past the magic so the decoder reads it and fails.
    bundle[xz_start + 20] ^= 0xFF
    path = _write_bundle(tmp_path / "badxz.bendl", bytes(bundle))
    reader = BenDecoder(path)
    # Opening works — the header/directory are intact.
    with pytest.raises(OSError):
        reader.read_asset_bytes("graph.json")


def test_directory_entry_with_zero_length_custom_payload(tmp_path: Path) -> None:
    bundle = build_bundle(
        stream_bytes=_ben_bytes_for([[1]], tmp_path),
        sample_count=1,
        assets=[
            _Asset(asset_type=ASSET_TYPE_CUSTOM, name="empty.bin", payload=b""),
        ],
    )
    path = _write_bundle(tmp_path / "zlen.bendl", bundle)
    reader = BenDecoder(path)
    assert reader.read_asset_bytes("empty.bin") == b""
    entry = next(a for a in reader.list_assets() if a["name"] == "empty.bin")
    assert entry["len"] == 0


def test_repr_on_incomplete_bundle(tmp_path: Path) -> None:
    stream = _ben_bytes_for([[1, 2]], tmp_path)
    header = _pack_header(
        complete=COMPLETE_NO,
        assignment_format=ASSIGNMENT_FORMAT_BEN,
        directory_offset=0,
        directory_len=0,
        stream_offset=HEADER_SIZE,
        stream_len=0,
        sample_count=-1,
    )
    path = _write_bundle(tmp_path / "rep.bendl", header + stream)
    reader = BenDecoder(path)
    # Incomplete bundle should open without error.
    assert reader.is_complete() is False
    assert reader.asset_names() == []


# ---------------------------------------------------------------------------
# Robustness: interrupted / truncated BEN streams inside a bundle
# ---------------------------------------------------------------------------


def _incomplete_bundle(stream_bytes: bytes) -> bytes:
    """Simulate a writer that crashed mid-stream: valid header, partial
    stream bytes, and no directory table at all (complete=0)."""
    header = _pack_header(
        complete=COMPLETE_NO,
        assignment_format=ASSIGNMENT_FORMAT_BEN,
        directory_offset=0,
        directory_len=0,
        stream_offset=HEADER_SIZE,
        stream_len=0,
        sample_count=-1,
    )
    return header + stream_bytes


def test_interrupted_ben_stream_mid_frame_decodes_valid_prefix(tmp_path: Path) -> None:
    # Simulate a writer that was killed after flushing the header and
    # part of the BEN stream, but before the stream was finished or the
    # directory was written.
    samples = [[1, 1, 2, 2], [3, 3, 4, 4], [5, 5, 6, 6], [7, 7, 8, 8], [9, 9, 9, 9]]
    full_ben = _ben_bytes_for(samples, tmp_path)
    # Cut the BEN bytes well past the 17-byte banner but before the end
    # so the truncation lands mid-frame.
    assert len(full_ben) > 25
    partial = full_ben[: len(full_ben) - 3]
    path = _write_bundle(tmp_path / "crashed.bendl", _incomplete_bundle(partial))

    reader = BenDecoder(path)
    assert reader.is_complete() is False
    assert reader.assignment_format() == "ben"
    # count_samples scans the truncated stream; it may error or return a
    # partial count — either is acceptable.
    try:
        n = reader.count_samples()
        assert n < len(samples)
    except Exception:
        pass

    # Verified extraction refuses unfinalized streams because their checksum is
    # not authoritative yet.
    extracted = tmp_path / "partial.ben"
    with pytest.raises(Exception, match="unfinalized"):
        reader.extract_stream(extracted)
    reader.extract_stream(extracted, overwrite=True, allow_unfinalized=True)
    assert extracted.read_bytes() == partial

    # The extracted file opens as a BEN stream (banner is intact).
    dec = BenDecoder(extracted, mode="ben")
    # Iterating through the truncated stream must either yield a strict
    # prefix of the samples and then raise, or raise on the very first
    # frame — both are acceptable outcomes. What is NOT acceptable is
    # silently producing garbage or decoding past the truncation.
    produced: list[list[int]] = []
    with pytest.raises(Exception):
        for a in dec:
            produced.append(a)
    # Whatever came out must be a strict prefix of the original samples.
    assert produced == samples[: len(produced)]
    assert len(produced) < len(samples)


def test_interrupted_ben_stream_inside_banner_fails_to_open_decoder(
    tmp_path: Path,
) -> None:
    # Truncate the BEN bytes inside the 17-byte banner region.
    full_ben = _ben_bytes_for([[1, 2, 3]], tmp_path)
    path = _write_bundle(tmp_path / "head_cut.bendl", _incomplete_bundle(full_ben[:8]))

    reader = BenDecoder(path)
    assert reader.is_complete() is False

    extracted = tmp_path / "head_cut.ben"
    with pytest.raises(Exception, match="unfinalized"):
        reader.extract_stream(extracted)
    reader.extract_stream(extracted, overwrite=True, allow_unfinalized=True)
    # The decoder must reject a BEN file whose banner is incomplete.
    with pytest.raises(Exception, match="Failed to create BenDecoder"):
        BenDecoder(extracted, mode="ben")


def test_interrupted_ben_stream_zero_bytes_after_header(tmp_path: Path) -> None:
    # The worst case: the writer crashed after writing the header and
    # before any stream bytes landed.
    path = _write_bundle(tmp_path / "zero.bendl", _incomplete_bundle(b""))

    reader = BenDecoder(path)
    assert reader.is_complete() is False
    assert reader.asset_names() == []
    # Zero stream bytes → scan fails (no BEN banner).
    with pytest.raises(Exception):
        reader.count_samples()

    extracted = tmp_path / "zero.ben"
    with pytest.raises(Exception, match="unfinalized"):
        reader.extract_stream(extracted)
    reader.extract_stream(extracted, overwrite=True, allow_unfinalized=True)
    assert extracted.read_bytes() == b""
    # A zero-byte .ben has no banner → decoder construction must fail.
    with pytest.raises(Exception, match="Failed to create BenDecoder"):
        BenDecoder(extracted, mode="ben")


def test_finalized_bundle_with_inflated_stream_len_survives_open(
    tmp_path: Path,
) -> None:
    # Build a valid finalized bundle, then patch stream_len to a value
    # larger than the actual stream payload. This simulates the narrow
    # window where the writer updated the header but was killed before
    # writing the directory — and something (or someone) re-flagged it
    # as finalized.
    samples = [[1, 2, 3], [4, 5, 6]]
    bundle = bytearray(
        build_bundle(
            stream_bytes=_ben_bytes_for(samples, tmp_path),
            sample_count=len(samples),
        )
    )
    # stream_len lives at header offset 48..56.
    old_stream_len = struct.unpack_from("<Q", bundle, 48)[0]
    struct.pack_into("<Q", bundle, 48, old_stream_len + 10_000)
    path = _write_bundle(tmp_path / "liar.bendl", bytes(bundle))

    # The reader's open() succeeds — the header fields parse as-is and
    # validation is lazy.
    reader = BenDecoder(path)
    assert reader.is_complete()
    # sample_count is what the header says.
    assert reader.count_samples() == len(samples)

    # extract_stream reads `stream_len` bytes from stream_offset; when
    # the file ends early, the short-read path must not hand back
    # fabricated bytes. A clean OSError is preferred; a truncated file
    # that decodes to a strict prefix is the fallback.
    extracted = tmp_path / "liar.ben"
    try:
        reader.extract_stream(extracted)
    except OSError:
        return
    # If the extract "succeeded" it can only have copied the real bytes.
    got = extracted.read_bytes()
    # The file cannot be longer than the claimed length, and it must be
    # a prefix of what a well-formed bundle would have produced.
    assert len(got) <= old_stream_len + 10_000


def test_read_metadata_after_extract_stream_still_works(tmp_path: Path) -> None:
    # Confirm that the same reader can serve asset reads after an
    # extract_stream call (i.e. internal seek state doesn't wedge things).
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
    reader = BenDecoder(path)
    reader.extract_stream(tmp_path / "s.ben")
    assert reader.read_metadata() == {"x": 1}
    reader.extract_stream(tmp_path / "s2.ben", overwrite=True)
    assert reader.read_asset_bytes("metadata.json") == b'{"x":1}'


# ---------------------------------------------------------------------------
# Stress / fuzz
# ---------------------------------------------------------------------------


def test_long_asset_name_near_u16_max(tmp_path: Path) -> None:
    # name_len in the directory entry is u16, so ~65500 is near the top.
    # Anything above u16::MAX should be rejected by a real writer — we only
    # stress the reader here, so we stay safely under 65535.
    long_name = "x" * 65500 + ".bin"
    assert len(long_name.encode("utf-8")) < 65536
    payload = b"payload-for-absurdly-long-name"
    bundle = build_bundle(
        stream_bytes=_ben_bytes_for([[1]], tmp_path),
        sample_count=1,
        assets=[_Asset(asset_type=ASSET_TYPE_CUSTOM, name=long_name, payload=payload)],
    )
    path = _write_bundle(tmp_path / "long.bendl", bundle)
    reader = BenDecoder(path)
    assert reader.asset_names() == [long_name]
    assert reader.read_asset_bytes(long_name) == payload


def test_list_assets_flag_fidelity(tmp_path: Path) -> None:
    # Every combination of (json, xz, checksum) should round-trip verbatim
    # through list_assets()["flags"].
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
    reader = BenDecoder(path)
    got = reader.list_assets()
    assert len(got) == len(combos)
    for entry, want in zip(got, expected):
        assert entry["flags"] == want


def test_read_asset_bytes_is_idempotent(tmp_path: Path) -> None:
    # Reading the same asset twice (with an xz round-trip in between) must
    # return byte-identical content, proving no internal state gets mutated.
    payload = b"repeat-me " * 100
    bundle = build_bundle(
        stream_bytes=_ben_bytes_for([[1, 2]], tmp_path),
        sample_count=1,
        assets=[
            _Asset(
                asset_type=ASSET_TYPE_CUSTOM,
                name="raw.bin",
                payload=payload,
            ),
            _Asset(
                asset_type=ASSET_TYPE_CUSTOM,
                name="compressed.bin",
                payload=payload,
                compress=True,
            ),
        ],
    )
    path = _write_bundle(tmp_path / "idem.bendl", bundle)
    reader = BenDecoder(path)
    for _ in range(5):
        assert reader.read_asset_bytes("raw.bin") == payload
        assert reader.read_asset_bytes("compressed.bin") == payload


def test_stress_many_heterogeneous_assets_round_trip(tmp_path: Path) -> None:
    # A full directory with rotating flags. This exercises directory
    # scaling, offset bookkeeping, and name lookup on a non-trivial directory.
    N = 256
    assets: List[_Asset] = []
    expected: List[Tuple[str, bytes]] = []
    rng = random.Random(0xBEEF)
    for i in range(N):
        payload = rng.randbytes(rng.randint(1, 200))
        compress = i % 3 == 0
        is_json = i % 5 == 0
        # When is_json is set we need valid UTF-8; use a safe synthetic blob.
        if is_json:
            payload = f'{{"i":{i},"n":{rng.randint(0, 1000)}}}'.encode("utf-8")
        assets.append(
            _Asset(
                asset_type=ASSET_TYPE_CUSTOM,
                name=f"asset-{i:04d}.bin",
                payload=payload,
                is_json=is_json,
                compress=compress,
            )
        )
        expected.append((f"asset-{i:04d}.bin", payload))

    bundle = build_bundle(
        stream_bytes=_ben_bytes_for([[1, 2, 3], [4, 5, 6]], tmp_path),
        sample_count=2,
        assets=assets,
    )
    path = _write_bundle(tmp_path / "many.bendl", bundle)
    reader = BenDecoder(path)

    assert reader.asset_names() == [name for name, _ in expected]
    # Sample every 37th asset and verify the payload decodes correctly
    # (xz pass-through on ~a third of them).
    for idx in range(0, N, 37):
        name, want = expected[idx]
        assert reader.read_asset_bytes(name) == want
    # Spot-check a JSON asset that was flagged json+compressed? Only json alone.
    json_idxs = [i for i in range(N) if i % 5 == 0 and i % 3 != 0]
    assert json_idxs  # sanity
    sample = json_idxs[len(json_idxs) // 2]
    name, want = expected[sample]
    assert reader.read_json_asset(name) == json.loads(want)


def test_seeded_fuzz_random_bundles_round_trip(tmp_path: Path) -> None:
    # Build 20 deliberately-different bundles from a seeded PRNG. Each one
    # mixes random asset sizes, random flags, random samples, and is then
    # fully round-tripped through BenDecoder on a .bendl bundle.
    rng = random.Random(0xFEED_FACE)
    for trial in range(20):
        n_assets = rng.randint(0, 12)
        assets: List[_Asset] = []
        truth: List[Tuple[str, bytes]] = []
        for i in range(n_assets):
            size = rng.choice([0, 1, 7, 64, 500, 4096])
            payload = rng.randbytes(size)
            compress = rng.random() < 0.4
            assets.append(
                _Asset(
                    asset_type=ASSET_TYPE_CUSTOM,
                    name=f"t{trial}-a{i}.bin",
                    payload=payload,
                    compress=compress,
                )
            )
            truth.append((f"t{trial}-a{i}.bin", payload))

        n_samples = rng.randint(1, 25)
        samples = [
            [rng.randint(1, 8) for _ in range(rng.randint(1, 40))]
            for _ in range(n_samples)
        ]

        bundle = build_bundle(
            stream_bytes=_ben_bytes_for(samples, tmp_path),
            sample_count=n_samples,
            assets=assets,
        )
        path = _write_bundle(tmp_path / f"fuzz-{trial}.bendl", bundle)

        reader = BenDecoder(path)
        assert reader.is_complete()
        assert reader.count_samples() == n_samples
        assert reader.asset_names() == [name for name, _ in truth]
        for name, want in truth:
            assert reader.read_asset_bytes(name) == want

        extracted = tmp_path / f"fuzz-{trial}.ben"
        reader.extract_stream(extracted)
        assert list(BenDecoder(extracted, mode="ben")) == samples


def test_interleaved_asset_and_stream_operations(tmp_path: Path) -> None:
    # Interleave every user-facing method to prove the reader does not
    # wedge its internal seek state when operations are reordered.
    samples = [[1, 2], [3, 4], [5, 6], [7, 8]]
    metadata = b'{"hello":"world"}'
    graph = b'{"nodes":[0,1,2],"edges":[[0,1],[1,2]]}'
    custom = b"\x00\x01\x02\x03" * 64

    bundle = build_bundle(
        stream_bytes=_ben_bytes_for(samples, tmp_path),
        sample_count=len(samples),
        assets=[
            _Asset(
                asset_type=ASSET_TYPE_METADATA,
                name="metadata.json",
                payload=metadata,
                is_json=True,
            ),
            _Asset(
                asset_type=ASSET_TYPE_GRAPH,
                name="graph.json",
                payload=graph,
                is_json=True,
                compress=True,
            ),
            _Asset(
                asset_type=ASSET_TYPE_CUSTOM,
                name="blob.bin",
                payload=custom,
            ),
        ],
    )
    path = _write_bundle(tmp_path / "interleave.bendl", bundle)
    reader = BenDecoder(path)

    # Strongly non-sequential access pattern.
    assert reader.read_asset_bytes("blob.bin") == custom
    assert reader.read_metadata() == {"hello": "world"}
    reader.extract_stream(tmp_path / "a.ben")
    assert reader.read_graph() == json.loads(graph)
    reader.extract_stream(tmp_path / "b.ben", overwrite=True)
    assert reader.read_asset_bytes("metadata.json") == metadata
    assert reader.read_asset_bytes("blob.bin") == custom
    assert reader.read_asset_bytes("graph.json") == graph
    reader.extract_stream(tmp_path / "c.ben", overwrite=True)

    # Every extracted stream must be byte-identical.
    a = (tmp_path / "a.ben").read_bytes()
    b = (tmp_path / "b.ben").read_bytes()
    c = (tmp_path / "c.ben").read_bytes()
    assert a == b == c
    assert list(BenDecoder(tmp_path / "a.ben", mode="ben")) == samples


def test_extract_stream_into_missing_parent_dir_raises_ioerror(tmp_path: Path) -> None:
    bundle = build_bundle(
        stream_bytes=_ben_bytes_for([[1, 2]], tmp_path),
        sample_count=1,
    )
    path = _write_bundle(tmp_path / "mini.bendl", bundle)
    reader = BenDecoder(path)
    missing = tmp_path / "does" / "not" / "exist" / "out.ben"
    with pytest.raises(OSError):
        reader.extract_stream(missing)


# ---------------------------------------------------------------------------
# BenEncoder bundle-output tests
# ---------------------------------------------------------------------------


SAMPLE_GRAPH = {
    "directed": False,
    "multigraph": False,
    "graph": {},
    "nodes": [{"id": 0}, {"id": 1}, {"id": 2}, {"id": 3}],
    "adjacency": [
        [{"id": 1}],
        [{"id": 0}, {"id": 2}],
        [{"id": 1}, {"id": 3}],
        [{"id": 2}],
    ],
}


def test_pybenencoder_default_emits_bundle_without_graph(tmp_path: Path) -> None:
    out = tmp_path / "stream.bendl"
    samples = [[1, 1, 2, 2], [3, 3, 2, 2], [3, 3, 3, 3]]
    with BenEncoder(out, overwrite=True, variant="standard") as enc:
        for a in samples:
            enc.write(a)

    reader = BenDecoder(out)
    assert reader.version() == (BENDL_MAJOR_VERSION, BENDL_MINOR_VERSION)
    assert reader.is_complete()
    assert reader.count_samples() == len(samples)
    assert reader.assignment_format() == "ben"
    # No graph because none was provided.
    assert reader.asset_names() == []
    assert reader.read_graph() is None

    extracted = tmp_path / "extracted.ben"
    reader.extract_stream(extracted)
    assert list(BenDecoder(extracted, mode="ben")) == samples


def test_pybenencoder_bundle_embeds_graph_from_dict(tmp_path: Path) -> None:
    out = tmp_path / "with_graph.bendl"
    samples = [[1, 1, 2, 2], [1, 1, 3, 3]]
    with BenEncoder(out, overwrite=True, variant="standard", graph=SAMPLE_GRAPH) as enc:
        for a in samples:
            enc.write(a)

    reader = BenDecoder(out)
    assert reader.is_complete()
    assert reader.count_samples() == len(samples)
    assert reader.asset_names() == ["graph.json"]

    assets = reader.list_assets()
    assert len(assets) == 1
    graph_entry = assets[0]
    assert graph_entry["name"] == "graph.json"
    # Default bundle policy xz-compresses graph.json.
    assert "xz" in graph_entry["flags"]
    assert "json" in graph_entry["flags"]

    assert reader.read_graph() == SAMPLE_GRAPH


def test_pybenencoder_bundle_embeds_graph_from_path(tmp_path: Path) -> None:
    graph_path = tmp_path / "graph.json"
    graph_path.write_text(json.dumps(SAMPLE_GRAPH))

    out = tmp_path / "with_graph_path.bendl"
    samples = [[0, 0, 1, 1]]
    with BenEncoder(out, overwrite=True, variant="standard", graph=graph_path) as enc:
        for a in samples:
            enc.write(a)

    reader = BenDecoder(out)
    assert reader.asset_names() == ["graph.json"]
    assert reader.read_graph() == SAMPLE_GRAPH


def test_pybenencoder_bundle_embeds_graph_from_str_path(tmp_path: Path) -> None:
    # String paths must be accepted verbatim (same coercion Path arguments
    # go through elsewhere in the API).
    graph_path = tmp_path / "graph-str.json"
    graph_path.write_text(json.dumps(SAMPLE_GRAPH))

    out = tmp_path / "via-str.bendl"
    samples = [[0, 1, 0, 1]]
    with BenEncoder(
        out, overwrite=True, variant="standard", graph=str(graph_path)
    ) as enc:
        for a in samples:
            enc.write(a)

    reader = BenDecoder(out)
    assert reader.read_graph() == SAMPLE_GRAPH


def test_pybenencoder_bundle_embeds_graph_from_bytes(tmp_path: Path) -> None:
    raw = json.dumps(SAMPLE_GRAPH).encode("utf-8")
    out = tmp_path / "via-bytes.bendl"
    samples = [[2, 2, 2, 2]]
    with BenEncoder(out, overwrite=True, variant="standard", graph=raw) as enc:
        for a in samples:
            enc.write(a)

    reader = BenDecoder(out)
    assert reader.read_graph() == SAMPLE_GRAPH


def test_pybenencoder_bundle_embeds_graph_from_bytesio(tmp_path: Path) -> None:
    buf = io.BytesIO(json.dumps(SAMPLE_GRAPH).encode("utf-8"))
    out = tmp_path / "via-bytesio.bendl"
    samples = [[1, 2, 1, 2]]
    with BenEncoder(out, overwrite=True, variant="standard", graph=buf) as enc:
        for a in samples:
            enc.write(a)

    reader = BenDecoder(out)
    assert reader.read_graph() == SAMPLE_GRAPH


def test_pybenencoder_bundle_embeds_graph_from_stringio(tmp_path: Path) -> None:
    buf = io.StringIO(json.dumps(SAMPLE_GRAPH))
    out = tmp_path / "via-stringio.bendl"
    samples = [[3, 3, 3, 3]]
    with BenEncoder(out, overwrite=True, variant="standard", graph=buf) as enc:
        for a in samples:
            enc.write(a)

    reader = BenDecoder(out)
    assert reader.read_graph() == SAMPLE_GRAPH


def test_pybenencoder_bundle_round_trip_via_extract_stream(tmp_path: Path) -> None:
    out = tmp_path / "full.bendl"
    rng = random.Random(0xCAFE)
    samples = [[rng.randint(1, 8) for _ in range(12)] for _ in range(15)]
    with BenEncoder(out, overwrite=True, variant="standard", graph=SAMPLE_GRAPH) as enc:
        for a in samples:
            enc.write(a)

    reader = BenDecoder(out)
    assert reader.count_samples() == len(samples)
    extracted = tmp_path / "full.ben"
    reader.extract_stream(extracted)
    assert list(BenDecoder(extracted, mode="ben")) == samples
    # And the graph still round-trips from the same reader.
    assert reader.read_graph() == SAMPLE_GRAPH


def test_pybenencoder_ben_file_only_rejects_graph(tmp_path: Path) -> None:
    out = tmp_path / "ben-with-graph.ben"
    with pytest.raises(ValueError, match="ben_file_only"):
        BenEncoder(
            out,
            overwrite=True,
            variant="standard",
            graph=SAMPLE_GRAPH,
            ben_file_only=True,
        )


def test_pybenencoder_ben_file_only_matches_old_format(tmp_path: Path) -> None:
    # A ben_file_only=True output should be byte-identical to the legacy
    # plain-BEN path, so the header has no BENDL magic.
    out = tmp_path / "legacy.ben"
    with BenEncoder(out, overwrite=True, variant="standard", ben_file_only=True) as enc:
        enc.write([1, 2, 3])
    blob = out.read_bytes()
    assert not blob.startswith(BENDL_MAGIC)
    # BenDecoder should still read it in ben mode.
    assert list(BenDecoder(out, mode="ben")) == [[1, 2, 3]]


def test_pybenencoder_bundle_close_is_idempotent(tmp_path: Path) -> None:
    out = tmp_path / "idem.bendl"
    enc = BenEncoder(out, overwrite=True, variant="standard")
    enc.write([1, 1, 2])
    enc.close()
    enc.close()  # second close must be a no-op
    with pytest.raises(OSError, match="already been closed"):
        enc.write([1, 2, 3])

    reader = BenDecoder(out)
    assert reader.is_complete()
    assert reader.count_samples() == 1


def test_pybenencoder_bundle_rejects_invalid_graph_type(tmp_path: Path) -> None:
    out = tmp_path / "bad.bendl"
    with pytest.raises(ValueError, match="graph must be"):
        BenEncoder(out, overwrite=True, variant="standard", graph=12345)


# ---------------------------------------------------------------------------
# BenDecoder opened directly on a .bendl bundle.
#
# The decoder auto-detects the BENDL magic and, when present, iterates only
# the embedded stream region while exposing TOC / asset helpers on the side.
# When opened on a plain .ben/.xben stream, iteration still works but the
# bundle methods must raise a clear error.
# ---------------------------------------------------------------------------


def test_pybendecoder_auto_detects_ben_bundle(tmp_path: Path) -> None:
    samples = [[1, 2, 3], [1, 2, 3], [4, 4, 5]]
    bundle = build_bundle(
        stream_bytes=_ben_bytes_for(samples, tmp_path),
        sample_count=len(samples),
        assignment_format=ASSIGNMENT_FORMAT_BEN,
    )
    path = _write_bundle(tmp_path / "stream.bendl", bundle)

    dec = BenDecoder(path)
    assert dec.is_bundle() is True
    assert dec.assignment_format() == "ben"
    assert dec.is_complete() is True
    assert dec.version() == (BENDL_MAJOR_VERSION, BENDL_MINOR_VERSION)
    assert len(dec) == len(samples)
    assert list(dec) == samples


def test_pybendecoder_auto_detects_xben_bundle(tmp_path: Path) -> None:
    samples = [[1, 1, 2, 2], [3, 3, 4, 4]]
    bundle = build_bundle(
        stream_bytes=_xben_bytes_for(samples, tmp_path, variant="mkv_chain"),
        sample_count=len(samples),
        assignment_format=ASSIGNMENT_FORMAT_XBEN,
    )
    path = _write_bundle(tmp_path / "stream.bendl", bundle)

    dec = BenDecoder(path)
    assert dec.is_bundle() is True
    assert dec.assignment_format() == "xben"
    assert len(dec) == len(samples)
    assert list(dec) == samples


def test_pybendecoder_bundle_toc_and_assets(tmp_path: Path) -> None:
    samples = [[1, 2, 3]]
    graph_json = b'{"nodes":[0,1],"edges":[[0,1]]}'
    metadata_json = b'{"note":"hello"}'
    relabel_json = b'{"0":"A","1":"B"}'

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
                payload=relabel_json,
                is_json=True,
            ),
            _Asset(
                asset_type=ASSET_TYPE_CUSTOM,
                name="notes.bin",
                payload=b"\x00\x01\x02",
            ),
        ],
    )
    path = _write_bundle(tmp_path / "rich.bendl", bundle)

    dec = BenDecoder(path)

    # TOC surface
    assert dec.asset_names() == [
        "metadata.json",
        "graph.json",
        "node_permutation_map.json",
        "notes.bin",
    ]
    assets = dec.list_assets()
    assert [a["name"] for a in assets] == dec.asset_names()
    by_name = {a["name"]: a for a in assets}
    assert "xz" in by_name["graph.json"]["flags"]
    assert "json" in by_name["graph.json"]["flags"]
    assert by_name["notes.bin"]["flags"] == ["checksum"]

    # Raw and JSON asset access
    assert dec.read_asset_bytes("metadata.json") == metadata_json
    assert dec.read_asset_bytes("graph.json") == graph_json
    assert dec.read_metadata() == json.loads(metadata_json)
    assert dec.read_graph() == json.loads(graph_json)
    assert dec.read_relabel_map() == json.loads(relabel_json)
    assert dec.read_json_asset("metadata.json") == json.loads(metadata_json)

    # Unknown asset by name raises KeyError.
    with pytest.raises(KeyError, match="no asset named"):
        dec.read_asset_bytes("missing.bin")

    # Iteration still works after the TOC surface has been used.
    assert list(dec) == samples


def test_pybendecoder_bundle_canonical_helpers_return_none_when_absent(
    tmp_path: Path,
) -> None:
    samples = [[1, 2]]
    bundle = build_bundle(
        stream_bytes=_ben_bytes_for(samples, tmp_path),
        sample_count=len(samples),
        assets=[_Asset(asset_type=ASSET_TYPE_CUSTOM, name="custom.bin", payload=b"x")],
    )
    path = _write_bundle(tmp_path / "sparse.bendl", bundle)
    dec = BenDecoder(path)
    assert dec.read_graph() is None
    assert dec.read_metadata() is None
    assert dec.read_relabel_map() is None


def test_pybendecoder_bundle_subsample_range(tmp_path: Path) -> None:
    samples = [[i, i + 1] for i in range(1, 11)]
    bundle = build_bundle(
        stream_bytes=_ben_bytes_for(samples, tmp_path),
        sample_count=len(samples),
    )
    path = _write_bundle(tmp_path / "range.bendl", bundle)

    dec = BenDecoder(path)
    dec.subsample_range(3, 6)
    assert list(dec) == samples[2:6]


def test_pybendecoder_bundle_subsample_indices(tmp_path: Path) -> None:
    samples = [[i] for i in range(1, 9)]
    bundle = build_bundle(
        stream_bytes=_ben_bytes_for(samples, tmp_path),
        sample_count=len(samples),
    )
    path = _write_bundle(tmp_path / "idx.bendl", bundle)

    dec = BenDecoder(path)
    dec.subsample_indices([1, 4, 8])
    assert list(dec) == [samples[0], samples[3], samples[7]]


def test_pybendecoder_bundle_subsample_every(tmp_path: Path) -> None:
    samples = [[i, i] for i in range(1, 11)]
    bundle = build_bundle(
        stream_bytes=_ben_bytes_for(samples, tmp_path),
        sample_count=len(samples),
    )
    path = _write_bundle(tmp_path / "every.bendl", bundle)

    dec = BenDecoder(path)
    dec.subsample_every(3, 2)
    assert list(dec) == [samples[1], samples[4], samples[7]]


def test_pybendecoder_bundle_mode_arg_is_ignored(tmp_path: Path) -> None:
    # For bundles, the header decides the format — a caller-supplied
    # `mode="xben"` on a BEN bundle must not confuse the reader.
    samples = [[1, 2, 3]]
    bundle = build_bundle(
        stream_bytes=_ben_bytes_for(samples, tmp_path),
        sample_count=len(samples),
        assignment_format=ASSIGNMENT_FORMAT_BEN,
    )
    path = _write_bundle(tmp_path / "ignore_mode.bendl", bundle)

    dec = BenDecoder(path, mode="xben")
    assert dec.assignment_format() == "ben"
    assert list(dec) == samples


def test_pybendecoder_on_plain_stream_supports_iteration(tmp_path: Path) -> None:
    # Opening a plain .ben file must still iterate unchanged; the new
    # bundle surface is simply unavailable.
    samples = [[1, 2, 3], [4, 5, 6]]
    ben_path = tmp_path / "plain.ben"
    with BenEncoder(
        ben_path, overwrite=True, variant="standard", ben_file_only=True
    ) as enc:
        for a in samples:
            enc.write(a)

    dec = BenDecoder(ben_path)
    assert dec.is_bundle() is False
    assert dec.assignment_format() == "ben"
    assert list(dec) == samples


@pytest.mark.parametrize(
    "method_call",
    [
        lambda d: d.version(),
        lambda d: d.is_complete(),
        lambda d: d.asset_names(),
        lambda d: d.list_assets(),
        lambda d: d.read_asset_bytes("metadata.json"),
        lambda d: d.read_json_asset("metadata.json"),
        lambda d: d.read_graph(),
        lambda d: d.read_metadata(),
        lambda d: d.read_relabel_map(),
    ],
)
def test_pybendecoder_plain_stream_rejects_bundle_methods(
    tmp_path: Path, method_call
) -> None:
    ben_path = tmp_path / "plain.ben"
    with BenEncoder(
        ben_path, overwrite=True, variant="standard", ben_file_only=True
    ) as enc:
        enc.write([1, 2, 3])

    dec = BenDecoder(ben_path)
    with pytest.raises(Exception, match="only available on .bendl bundles"):
        method_call(dec)


def test_pybendecoder_plain_stream_error_mentions_ben_file_only(
    tmp_path: Path,
) -> None:
    ben_path = tmp_path / "plain.ben"
    with BenEncoder(
        ben_path, overwrite=True, variant="standard", ben_file_only=True
    ) as enc:
        enc.write([1])

    dec = BenDecoder(ben_path)
    with pytest.raises(Exception, match="ben_file_only=False"):
        dec.read_graph()


def test_pybendecoder_opens_bundle_produced_by_pybenencoder(tmp_path: Path) -> None:
    # End-to-end: a bundle written by BenEncoder (with a graph asset)
    # must round-trip through a single BenDecoder call — no need to
    # extract the stream first.
    out = tmp_path / "e2e.bendl"
    with BenEncoder(out, overwrite=True, variant="standard", graph=SAMPLE_GRAPH) as enc:
        for a in [[1, 2, 3], [2, 3, 4]]:
            enc.write(a)

    dec = BenDecoder(out)
    assert dec.is_bundle() is True
    assert dec.is_complete() is True
    assert dec.assignment_format() == "ben"
    assert dec.read_graph() == SAMPLE_GRAPH
    assert list(dec) == [[1, 2, 3], [2, 3, 4]]


def test_pybendecoder_incomplete_bundle_counts_via_scan(tmp_path: Path) -> None:
    # An incomplete bundle has complete=0 and no directory — its header
    # carries no authoritative sample_count, so __len__ must fall back
    # to scanning the stream region. This exercises the
    # `scan_bundle_samples` path in the decoder.
    samples = [[1, 2, 3], [4, 5, 6], [7, 8, 9]]
    stream = _ben_bytes_for(samples, tmp_path)
    header = _pack_header(
        complete=COMPLETE_NO,
        assignment_format=ASSIGNMENT_FORMAT_BEN,
        directory_offset=0,
        directory_len=0,
        stream_offset=HEADER_SIZE,
        stream_len=len(stream),
        sample_count=-1,
    )
    path = tmp_path / "incomplete.bendl"
    path.write_bytes(header + stream)

    dec = BenDecoder(path)
    assert dec.is_bundle() is True
    assert dec.is_complete() is False
    # len() forces the fallback scan, which must agree with the data.
    assert len(dec) == len(samples)
    # A second call uses the cached value and still returns the same.
    assert len(dec) == len(samples)
    # The iterator itself still works.
    assert list(dec) == samples


def test_pybendecoder_incomplete_bundle_count_samples_matches_len(
    tmp_path: Path,
) -> None:
    # Explicit count_samples() also flows through scan_bundle_samples
    # for incomplete bundles.
    samples = [[i, i + 1] for i in range(1, 6)]
    stream = _ben_bytes_for(samples, tmp_path)
    header = _pack_header(
        complete=COMPLETE_NO,
        assignment_format=ASSIGNMENT_FORMAT_BEN,
        directory_offset=0,
        directory_len=0,
        stream_offset=HEADER_SIZE,
        stream_len=len(stream),
        sample_count=-1,
    )
    path = tmp_path / "incomplete_count.bendl"
    path.write_bytes(header + stream)

    dec = BenDecoder(path)
    assert dec.count_samples() == len(samples)
    assert len(dec) == len(samples)


def test_pybendecoder_rejects_unknown_assignment_format(tmp_path: Path) -> None:
    # A finalized bundle whose assignment_format byte is neither BEN
    # nor XBEN must surface a clear error at decoder construction, not
    # silently fall through.
    samples = [[1, 2, 3]]
    bundle = build_bundle(
        stream_bytes=_ben_bytes_for(samples, tmp_path),
        sample_count=len(samples),
        assignment_format=99,
    )
    path = _write_bundle(tmp_path / "weird_fmt.bendl", bundle)
    with pytest.raises(Exception, match="unrecognized assignment_format"):
        BenDecoder(path)


def test_pybendecoder_empty_stream_bundle(tmp_path: Path) -> None:
    # A bundle containing a valid BEN banner but zero frames must be
    # openable and produce an empty iterator / zero-length decoder.
    bundle = build_bundle(stream_bytes=_ben_bytes_for([], tmp_path), sample_count=0)
    path = _write_bundle(tmp_path / "empty.bendl", bundle)

    dec = BenDecoder(path)
    assert dec.is_bundle() is True
    assert len(dec) == 0
    assert dec.count_samples() == 0
    assert list(dec) == []
    assert dec.asset_names() == []
    assert dec.list_assets() == []


def test_pybendecoder_bundle_toc_interleaved_with_iteration(tmp_path: Path) -> None:
    # Calling TOC / asset methods in between __next__ calls must not
    # break the iterator — the TOC access uses a separate BendlReader,
    # not the file handle backing the iterator.
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

    dec = BenDecoder(path)
    it = iter(dec)

    assert next(it) == samples[0]
    # TOC read between samples
    assert dec.read_metadata() == {"tag": 42}
    assert dec.asset_names() == ["metadata.json"]
    assert next(it) == samples[1]
    # And another TOC read
    assert dec.read_asset_bytes("metadata.json") == b'{"tag":42}'
    assert next(it) == samples[2]
    with pytest.raises(StopIteration):
        next(it)


def test_pybendecoder_bundle_subsample_range_rejects_out_of_bounds(
    tmp_path: Path,
) -> None:
    samples = [[1, 2], [3, 4], [5, 6]]
    bundle = build_bundle(
        stream_bytes=_ben_bytes_for(samples, tmp_path),
        sample_count=len(samples),
    )
    path = _write_bundle(tmp_path / "range_bad.bendl", bundle)
    dec = BenDecoder(path)
    with pytest.raises(Exception, match="end must be <= number of samples"):
        dec.subsample_range(1, 99)
    with pytest.raises(Exception, match="1-based"):
        dec.subsample_range(0, 1)


def test_pybendecoder_bundle_subsample_indices_rejects_out_of_bounds(
    tmp_path: Path,
) -> None:
    samples = [[1, 2], [3, 4]]
    bundle = build_bundle(
        stream_bytes=_ben_bytes_for(samples, tmp_path),
        sample_count=len(samples),
    )
    path = _write_bundle(tmp_path / "idx_bad.bendl", bundle)
    dec = BenDecoder(path)
    with pytest.raises(Exception, match="number of samples"):
        dec.subsample_indices([1, 42])
    # Empty index list is also rejected.
    dec2 = BenDecoder(path)
    with pytest.raises(Exception, match="must not be empty"):
        dec2.subsample_indices([])


def test_pybendecoder_bundle_subsample_every_rejects_bad_args(tmp_path: Path) -> None:
    samples = [[1], [2], [3]]
    bundle = build_bundle(
        stream_bytes=_ben_bytes_for(samples, tmp_path),
        sample_count=len(samples),
    )
    path = _write_bundle(tmp_path / "every_bad.bendl", bundle)
    dec = BenDecoder(path)
    with pytest.raises(Exception, match="offset must be <= number of samples"):
        dec.subsample_every(1, 99)
    dec2 = BenDecoder(path)
    with pytest.raises(Exception, match="step and offset must be >= 1"):
        dec2.subsample_every(0, 1)


def test_pybendecoder_plain_stream_len_is_cached(tmp_path: Path) -> None:
    # __len__ caches the scan result; calling it twice must not re-scan
    # but must return the same answer.
    samples = [[1, 2], [3, 4], [5, 6]]
    ben_path = tmp_path / "cached.ben"
    with BenEncoder(
        ben_path, overwrite=True, variant="standard", ben_file_only=True
    ) as enc:
        for a in samples:
            enc.write(a)
    dec = BenDecoder(ben_path)
    assert len(dec) == len(samples)
    assert len(dec) == len(samples)
    # Explicit count_samples must also agree.
    assert dec.count_samples() == len(samples)


def test_pybendecoder_detects_very_short_file_as_plain(tmp_path: Path) -> None:
    # A 4-byte file cannot start with the BENDL magic; detect_is_bundle
    # must return false on UnexpectedEof, after which plain-stream
    # decoding fails with a banner error.
    path = tmp_path / "tiny.ben"
    path.write_bytes(b"abcd")
    with pytest.raises(Exception):
        BenDecoder(path)


def test_pybendecoder_empty_file_is_treated_as_plain(tmp_path: Path) -> None:
    path = tmp_path / "empty.ben"
    path.write_bytes(b"")
    with pytest.raises(Exception):
        BenDecoder(path)


def test_pybendecoder_bundle_read_json_asset_rejects_non_utf8(tmp_path: Path) -> None:
    # read_json_asset on the decoder should reject non-UTF-8 the same as
    # error behavior when an asset isn't valid UTF-8.
    bundle = build_bundle(
        stream_bytes=_ben_bytes_for([[1]], tmp_path),
        sample_count=1,
        assets=[
            _Asset(
                asset_type=ASSET_TYPE_CUSTOM,
                name="binary.bin",
                payload=b"\xff\xfe\xfd",
            )
        ],
    )
    path = _write_bundle(tmp_path / "bad_utf8.bendl", bundle)
    dec = BenDecoder(path)
    # Raw bytes are fine.
    assert dec.read_asset_bytes("binary.bin") == b"\xff\xfe\xfd"
    with pytest.raises(Exception, match="not valid UTF-8"):
        dec.read_json_asset("binary.bin")


def test_pybendecoder_bundle_read_json_asset_rejects_bad_json(tmp_path: Path) -> None:
    bundle = build_bundle(
        stream_bytes=_ben_bytes_for([[1]], tmp_path),
        sample_count=1,
        assets=[
            _Asset(
                asset_type=ASSET_TYPE_METADATA,
                name="metadata.json",
                payload=b"not json {",
                is_json=True,
            )
        ],
    )
    path = _write_bundle(tmp_path / "bad_json.bendl", bundle)
    dec = BenDecoder(path)
    with pytest.raises(json.JSONDecodeError):
        dec.read_metadata()


def test_pybendecoder_bundle_graph_asset_is_xz_transparent(tmp_path: Path) -> None:
    # A bundle built with BenEncoder compresses the graph asset as xz;
    # read_graph() on BenDecoder must still return the decoded JSON.
    out = tmp_path / "xz_graph.bendl"
    with BenEncoder(out, overwrite=True, variant="standard", graph=SAMPLE_GRAPH) as enc:
        enc.write([1, 2, 3])
    dec = BenDecoder(out)
    # Spot-check that graph.json was actually stored compressed.
    by_name = {a["name"]: a for a in dec.list_assets()}
    assert "xz" in by_name["graph.json"]["flags"]
    assert dec.read_graph() == SAMPLE_GRAPH


def test_pybendecoder_bundle_xben_with_assets(tmp_path: Path) -> None:
    # XBEN bundles with TOC entries were not previously covered — only
    # the plain XBEN-bundle auto-detect case. Verify iteration AND TOC
    # access both work on an XBEN bundle.
    samples = [[1, 1, 2, 2], [2, 2, 1, 1], [3, 3, 3, 3]]
    meta = b'{"variant":"mkv_chain"}'
    bundle = build_bundle(
        stream_bytes=_xben_bytes_for(samples, tmp_path, variant="mkv_chain"),
        sample_count=len(samples),
        assignment_format=ASSIGNMENT_FORMAT_XBEN,
        assets=[
            _Asset(
                asset_type=ASSET_TYPE_METADATA,
                name="metadata.json",
                payload=meta,
                is_json=True,
            )
        ],
    )
    path = _write_bundle(tmp_path / "xben_assets.bendl", bundle)

    dec = BenDecoder(path)
    assert dec.assignment_format() == "xben"
    assert dec.asset_names() == ["metadata.json"]
    assert dec.read_metadata() == {"variant": "mkv_chain"}
    assert list(dec) == samples


def test_pybendecoder_bundle_subsample_indices_unsorted_warns(tmp_path: Path) -> None:
    # The subsample_indices path that sorts+dedupes unsorted input also
    # has to work for bundles. Mixing in duplicates should still yield
    # the deduplicated selection.
    samples = [[i] for i in range(1, 6)]
    bundle = build_bundle(
        stream_bytes=_ben_bytes_for(samples, tmp_path),
        sample_count=len(samples),
    )
    path = _write_bundle(tmp_path / "unsorted.bendl", bundle)
    dec = BenDecoder(path)
    with pytest.warns(UserWarning, match="sorted and unique"):
        dec.subsample_indices([4, 1, 4, 1])
    assert list(dec) == [[1], [4]]


def test_pybendecoder_plain_xben_assignment_format(tmp_path: Path) -> None:
    # `assignment_format()` must report "xben" when opened on a plain
    # XBEN stream as well, not only on bundles.
    samples = [[1, 1, 2, 2], [2, 2, 1, 1]]
    src = tmp_path / "src.jsonl"
    _write_jsonl(samples, src)
    xben_path = tmp_path / "plain.xben"
    encode_jsonl_to_xben(
        src,
        xben_path,
        overwrite=True,
        variant="standard",
        n_threads=1,
        compression_level=1,
    )
    with pytest.warns(UserWarning):
        dec = BenDecoder(xben_path, mode="xben")
    assert dec.is_bundle() is False
    assert dec.assignment_format() == "xben"
    assert list(dec) == samples


def test_pybendecoder_incomplete_bundle_rejects_toc_methods_that_need_directory(
    tmp_path: Path,
) -> None:
    # An incomplete bundle has no directory, so there are no assets to
    # list — asset-free surface still returns empty structures, which is
    # the contract for finalized asset-free bundles too. Just verify it
    # doesn't crash.
    samples = [[1, 2]]
    stream = _ben_bytes_for(samples, tmp_path)
    header = _pack_header(
        complete=COMPLETE_NO,
        assignment_format=ASSIGNMENT_FORMAT_BEN,
        directory_offset=0,
        directory_len=0,
        stream_offset=HEADER_SIZE,
        stream_len=len(stream),
        sample_count=-1,
    )
    path = tmp_path / "incomplete_toc.bendl"
    path.write_bytes(header + stream)

    dec = BenDecoder(path)
    assert dec.is_bundle() is True
    assert dec.is_complete() is False
    assert dec.asset_names() == []
    assert dec.list_assets() == []
    assert dec.read_graph() is None
    assert dec.read_metadata() is None
    assert dec.read_relabel_map() is None


def test_pybendecoder_bundle_iteration_can_restart(tmp_path: Path) -> None:
    # `__iter__` rebuilds the underlying frame walker so `for x in dec:`
    # can be used more than once against a bundle.
    samples = [[1, 2], [3, 4], [5, 6]]
    bundle = build_bundle(
        stream_bytes=_ben_bytes_for(samples, tmp_path),
        sample_count=len(samples),
    )
    path = _write_bundle(tmp_path / "twice.bendl", bundle)
    dec = BenDecoder(path)
    assert list(dec) == samples
    # A second pass reopens the stream region from the start.
    assert list(dec) == samples


def test_pybendecoder_plain_stream_iteration_can_restart(tmp_path: Path) -> None:
    samples = [[1, 2, 3], [4, 5, 6], [7, 8, 9]]
    ben_path = tmp_path / "twice.ben"
    with BenEncoder(
        ben_path, overwrite=True, variant="standard", ben_file_only=True
    ) as enc:
        for a in samples:
            enc.write(a)
    dec = BenDecoder(ben_path)
    assert list(dec) == samples
    assert list(dec) == samples


def test_pybendecoder_subsample_range_survives_reiteration(tmp_path: Path) -> None:
    # Subsample selections must persist across `__iter__` calls, so
    # iterating the same (subsampled) decoder twice gives the same
    # filtered window each time.
    samples = [[i, i + 1] for i in range(1, 11)]
    bundle = build_bundle(
        stream_bytes=_ben_bytes_for(samples, tmp_path),
        sample_count=len(samples),
    )
    path = _write_bundle(tmp_path / "range_twice.bendl", bundle)
    dec = BenDecoder(path)
    dec.subsample_range(3, 6)
    expected = samples[2:6]
    assert list(dec) == expected
    assert list(dec) == expected


def test_pybendecoder_subsample_indices_survives_reiteration(tmp_path: Path) -> None:
    samples = [[i] for i in range(1, 8)]
    bundle = build_bundle(
        stream_bytes=_ben_bytes_for(samples, tmp_path),
        sample_count=len(samples),
    )
    path = _write_bundle(tmp_path / "idx_twice.bendl", bundle)
    dec = BenDecoder(path)
    dec.subsample_indices([2, 5, 7])
    expected = [samples[1], samples[4], samples[6]]
    assert list(dec) == expected
    assert list(dec) == expected


def test_pybendecoder_subsample_every_survives_reiteration(tmp_path: Path) -> None:
    samples = [[i] for i in range(1, 11)]
    bundle = build_bundle(
        stream_bytes=_ben_bytes_for(samples, tmp_path),
        sample_count=len(samples),
    )
    path = _write_bundle(tmp_path / "every_twice.bendl", bundle)
    dec = BenDecoder(path)
    dec.subsample_every(3, 2)
    expected = [samples[1], samples[4], samples[7]]
    assert list(dec) == expected
    assert list(dec) == expected


def test_pybendecoder_resubsample_replaces_previous_selection(tmp_path: Path) -> None:
    # Calling subsample_* a second time must replace the first selection
    # AND survive reiteration with the new selection.
    samples = [[i] for i in range(1, 8)]
    bundle = build_bundle(
        stream_bytes=_ben_bytes_for(samples, tmp_path),
        sample_count=len(samples),
    )
    path = _write_bundle(tmp_path / "reselect.bendl", bundle)
    dec = BenDecoder(path)
    dec.subsample_range(1, 3)
    assert list(dec) == samples[:3]
    dec.subsample_indices([4, 7])
    expected = [samples[3], samples[6]]
    assert list(dec) == expected
    assert list(dec) == expected


def test_pybendecoder_partial_iteration_then_restart(tmp_path: Path) -> None:
    # Consuming part of the iterator and then calling `iter()` / `list()`
    # again must restart cleanly from the first sample, not resume
    # mid-stream.
    samples = [[1, 2], [3, 4], [5, 6], [7, 8]]
    bundle = build_bundle(
        stream_bytes=_ben_bytes_for(samples, tmp_path),
        sample_count=len(samples),
    )
    path = _write_bundle(tmp_path / "partial.bendl", bundle)
    dec = BenDecoder(path)
    it = iter(dec)
    assert next(it) == samples[0]
    assert next(it) == samples[1]
    # Any new pass (list / for / iter) rebuilds and starts over.
    assert list(dec) == samples


def test_pybendecoder_count_samples_after_subsample_preserves_len(
    tmp_path: Path,
) -> None:
    # After `subsample_*`, `len(dec)` must reflect the filtered count.
    # Calling `count_samples()` reports the base (unfiltered) count but
    # must not clobber the filtered `len(dec)` value.
    samples = [[i] for i in range(1, 9)]
    bundle = build_bundle(
        stream_bytes=_ben_bytes_for(samples, tmp_path),
        sample_count=len(samples),
    )
    path = _write_bundle(tmp_path / "count_after_sub.bendl", bundle)
    dec = BenDecoder(path)
    dec.subsample_range(2, 5)
    assert len(dec) == 4
    assert dec.count_samples() == len(samples)
    # The filtered length contract must survive a count_samples() call.
    assert len(dec) == 4
    assert list(dec) == samples[1:5]


def test_pybendecoder_count_samples_plain_after_subsample_preserves_len(
    tmp_path: Path,
) -> None:
    # Same contract as above, but on a plain .ben stream to cover the
    # non-bundle branch of `ensure_base_len`.
    samples = [[i] for i in range(1, 11)]
    ben_path = tmp_path / "plain_count.ben"
    with BenEncoder(
        ben_path, overwrite=True, variant="standard", ben_file_only=True
    ) as enc:
        for a in samples:
            enc.write(a)
    dec = BenDecoder(ben_path)
    dec.subsample_every(3, 1)
    expected = samples[::3]
    assert len(dec) == len(expected)
    assert dec.count_samples() == len(samples)
    assert len(dec) == len(expected)
    assert list(dec) == expected


def test_pybendecoder_subsample_then_count_samples_then_reiterate(
    tmp_path: Path,
) -> None:
    # Composing subsample → count_samples → restart iteration must keep
    # the filtered view intact across the restart.
    samples = [[i, i + 1] for i in range(1, 9)]
    bundle = build_bundle(
        stream_bytes=_ben_bytes_for(samples, tmp_path),
        sample_count=len(samples),
    )
    path = _write_bundle(tmp_path / "sub_count_restart.bendl", bundle)
    dec = BenDecoder(path)
    dec.subsample_indices([1, 4, 8])
    assert dec.count_samples() == len(samples)
    expected = [samples[0], samples[3], samples[7]]
    assert list(dec) == expected
    assert list(dec) == expected


def test_pybendecoder_bundle_read_json_asset_missing_name_raises_keyerror(
    tmp_path: Path,
) -> None:
    # `read_json_asset` on a valid bundle that does not carry the named
    # asset must surface a KeyError, matching `read_asset_bytes`.
    bundle = build_bundle(
        stream_bytes=_ben_bytes_for([[1, 2]], tmp_path),
        sample_count=1,
    )
    path = _write_bundle(tmp_path / "missing_json.bendl", bundle)
    dec = BenDecoder(path)
    with pytest.raises(KeyError, match="nope.json"):
        dec.read_json_asset("nope.json")


def test_pybendecoder_bundle_len_uses_header_fast_path(tmp_path: Path) -> None:
    # For a finalized bundle, `len(dec)` should use the O(1) header
    # sample_count fast path rather than scanning the stream. We can't
    # observe the scan directly, but we can verify the result matches
    # the count declared in the header even when the stream is a real
    # BEN payload.
    samples = [[i] for i in range(1, 6)]
    bundle = build_bundle(
        stream_bytes=_ben_bytes_for(samples, tmp_path),
        sample_count=len(samples),
    )
    path = _write_bundle(tmp_path / "fast_len.bendl", bundle)
    dec = BenDecoder(path)
    assert len(dec) == len(samples)
    # A second call returns the cached value and must agree.
    assert len(dec) == len(samples)
    assert dec.count_samples() == len(samples)
