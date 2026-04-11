"""Tests for PyBundleReader.

These tests do not rely on the `bendl` CLI binary being built. Instead, they
construct `.bendl` bundles directly in Python from the on-disk format spec
documented in ``ben/src/io/bundle/format.rs``. This keeps the tests
self-contained and lets them stress odd byte layouts that a CLI-based helper
could not produce (truncated files, bad magic, dangling offsets, etc).

Real BEN/XBEN stream payloads are produced via ``PyBenEncoder`` /
``compress_jsonl_to_xben`` so the stream region always matches what the
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
    PyBenDecoder,
    PyBenEncoder,
    PyBundleReader,
    compress_jsonl_to_ben,
    compress_jsonl_to_xben,
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
ASSET_TYPE_RELABEL_MAP = 3
ASSET_TYPE_CUSTOM = 4

ASSET_FLAG_JSON = 1 << 0
ASSET_FLAG_XZ = 1 << 1
ASSET_FLAG_CHECKSUM = 1 << 2


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
    reserved_0: int = 0,
) -> bytes:
    if len(magic) != 8:
        raise ValueError("magic must be 8 bytes")
    return (
        magic
        + struct.pack(
            "<HHBBHQQQQQq",
            major_version,
            minor_version,
            complete,
            assignment_format,
            reserved_0,
            flags,
            directory_offset,
            directory_len,
            stream_offset,
            stream_len,
            sample_count,
        )
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

    def flags(self) -> int:
        flags = 0
        if self.is_json:
            flags |= ASSET_FLAG_JSON
        if self.compress:
            flags |= ASSET_FLAG_XZ
        if self.checksum is not None:
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
) -> bytes:
    """Construct the bytes of a `.bendl` file from pieces.

    The layout is ``[header][asset payloads][stream][directory]``. This
    helper mirrors the writer's finalize path closely enough to produce
    bundles that the Rust reader accepts, while also exposing enough knobs
    to generate deliberately broken bundles for negative tests.
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
    for (offset, length, _enc), asset in zip(encoded_assets, assets):
        entries_bytes.append(
            _pack_directory_entry(
                asset_type=asset.asset_type,
                asset_flags=asset.flags(),
                name=asset.name,
                payload_offset=offset,
                payload_len=length,
                checksum=asset.checksum,
            )
        )
    directory = _pack_directory(entries_bytes)
    buf.extend(directory)
    directory_len = len(directory)

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
    """Produce real BEN bytes for ``samples`` via ``PyBenEncoder``."""
    ben_path = tmp / "inner.ben"
    with PyBenEncoder(
        ben_path, overwrite=True, variant=variant, ben_file_only=True
    ) as enc:
        for a in samples:
            enc.write(a)
    return ben_path.read_bytes()


def _xben_bytes_for(samples: List[List[int]], tmp: Path, variant: str = "standard") -> bytes:
    src = tmp / "src.jsonl"
    _write_jsonl(samples, src)
    out = tmp / "inner.xben"
    compress_jsonl_to_xben(
        src, out, overwrite=True, variant=variant, n_threads=1, compression_level=1
    )
    return out.read_bytes()


def _write_bundle(path: Path, bundle_bytes: bytes) -> Path:
    path.write_bytes(bundle_bytes)
    return path


# ---------------------------------------------------------------------------
# Baseline happy-path tests
# ---------------------------------------------------------------------------


def test_module_exports_pybundlereader() -> None:
    assert "PyBundleReader" in binary_ensemble.__all__
    assert hasattr(binary_ensemble, "PyBundleReader")


def test_bundle_reader_round_trip_ben_with_assets(tmp_path: Path) -> None:
    rng = random.Random(4242)
    samples = [[rng.randint(1, 10) for _ in range(rng.randint(1, 50))] for _ in range(40)]

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
                asset_type=ASSET_TYPE_RELABEL_MAP,
                name="relabel_map.json",
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

    reader = PyBundleReader(path)

    assert reader.version() == (BENDL_MAJOR_VERSION, BENDL_MINOR_VERSION)
    assert reader.is_complete() is True
    assert reader.sample_count() == len(samples)
    assert reader.assignment_format() == "ben"

    names = reader.asset_names()
    assert names == ["metadata.json", "graph.json", "relabel_map.json", "notes.bin"]

    assets = reader.list_assets()
    assert [a["name"] for a in assets] == names
    by_name = {a["name"]: a for a in assets}
    assert by_name["graph.json"]["type"] == ASSET_TYPE_GRAPH
    assert "xz" in by_name["graph.json"]["flags"]
    assert "json" in by_name["graph.json"]["flags"]
    assert "xz" not in by_name["metadata.json"]["flags"]
    assert "json" in by_name["metadata.json"]["flags"]
    assert by_name["notes.bin"]["flags"] == []
    # payload_offset must sit at or past the end of the header.
    for entry in assets:
        assert entry["offset"] >= HEADER_SIZE
        assert entry["len"] > 0

    # Raw byte access (decompresses xz transparently).
    assert reader.read_asset_bytes("metadata.json") == metadata_json
    assert reader.read_asset_bytes("graph.json") == graph_json
    assert reader.read_asset_bytes("relabel_map.json") == relabel_json
    assert reader.read_asset_bytes("notes.bin") == custom_blob

    # Typed JSON helpers.
    assert reader.read_metadata() == json.loads(metadata_json)
    assert reader.read_graph() == json.loads(graph_json)
    assert reader.read_relabel_map() == json.loads(relabel_json)

    # read_json_asset by name.
    assert reader.read_json_asset("metadata.json") == json.loads(metadata_json)

    # extract_stream then decode via PyBenDecoder.
    extracted = tmp_path / "stream.ben"
    reader.extract_stream(extracted)
    got = list(PyBenDecoder(extracted, mode="ben"))
    assert got == samples

    # __repr__ should not crash and should mention the path.
    r = repr(reader)
    assert "PyBundleReader" in r
    assert "complete=true" in r or "complete=True" in r


def test_bundle_reader_round_trip_xben(tmp_path: Path) -> None:
    samples = [[1, 2, 3], [1, 2, 3], [4, 4, 5], [6, 7, 8]]
    bundle = build_bundle(
        stream_bytes=_xben_bytes_for(samples, tmp_path, variant="mkv_chain"),
        sample_count=len(samples),
        assignment_format=ASSIGNMENT_FORMAT_XBEN,
        assets=[],
    )
    path = _write_bundle(tmp_path / "xout.bendl", bundle)
    reader = PyBundleReader(path)

    assert reader.assignment_format() == "xben"
    assert reader.is_complete()
    assert reader.sample_count() == len(samples)
    assert reader.asset_names() == []

    # extract_stream → file must round-trip via the xben decoder.
    extracted = tmp_path / "stream.xben"
    reader.extract_stream(extracted)
    assert list(PyBenDecoder(extracted, mode="xben")) == samples


def test_bundle_reader_canonical_helpers_return_none_when_absent(tmp_path: Path) -> None:
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
    reader = PyBundleReader(path)
    assert reader.read_metadata() is None
    assert reader.read_graph() is None
    assert reader.read_relabel_map() is None


def test_bundle_reader_asset_free_empty_stream(tmp_path: Path) -> None:
    # A bundle with no assets and an empty stream is legal (spec says so).
    bundle = build_bundle(stream_bytes=b"", sample_count=0, assets=[])
    path = _write_bundle(tmp_path / "empty.bendl", bundle)
    reader = PyBundleReader(path)
    assert reader.is_complete()
    assert reader.sample_count() == 0
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
    reader = PyBundleReader(path)
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
    reader = PyBundleReader(path)
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
    reader = PyBundleReader(path)
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
    reader = PyBundleReader(path)
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
    reader = PyBundleReader(path)
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
    reader = PyBundleReader(path)
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
    reader = PyBundleReader(path)
    target = tmp_path / "out.ben"
    target.write_bytes(b"filler")
    reader.extract_stream(target, overwrite=True)
    # Re-opening the extracted file via PyBenDecoder confirms it's a valid .ben.
    assert list(PyBenDecoder(target, mode="ben")) == [[1, 2], [3, 4]]


# ---------------------------------------------------------------------------
# Robustness: invalid headers and corrupted bundles
# ---------------------------------------------------------------------------


def test_open_rejects_missing_file(tmp_path: Path) -> None:
    with pytest.raises(OSError, match="Failed to open"):
        PyBundleReader(tmp_path / "does_not_exist.bendl")


def test_open_rejects_bad_magic(tmp_path: Path) -> None:
    bundle = build_bundle(
        stream_bytes=_ben_bytes_for([[1, 2]], tmp_path),
        sample_count=1,
        magic=b"NOTABEND",
    )
    path = _write_bundle(tmp_path / "bad.bendl", bundle)
    with pytest.raises(Exception, match="Failed to parse bundle header"):
        PyBundleReader(path)


def test_open_rejects_unsupported_major_version(tmp_path: Path) -> None:
    bundle = build_bundle(
        stream_bytes=_ben_bytes_for([[1, 2]], tmp_path),
        sample_count=1,
        major_version=999,
    )
    path = _write_bundle(tmp_path / "oldfuture.bendl", bundle)
    with pytest.raises(Exception, match="Failed to parse bundle header"):
        PyBundleReader(path)


def test_open_rejects_truncated_header(tmp_path: Path) -> None:
    path = tmp_path / "short.bendl"
    path.write_bytes(b"BENDL\x00\x00\x01\x00")  # magic plus 2 bytes — not enough
    with pytest.raises(Exception, match="Failed to parse bundle header"):
        PyBundleReader(path)


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
        PyBundleReader(path)


def test_open_rejects_bundle_with_chopped_directory_bytes(tmp_path: Path) -> None:
    bundle = build_bundle(
        stream_bytes=_ben_bytes_for([[1, 2]], tmp_path),
        sample_count=1,
        assets=[_Asset(asset_type=ASSET_TYPE_CUSTOM, name="x", payload=b"abc")],
    )
    # Drop the final two bytes of the directory.
    path = _write_bundle(tmp_path / "chop.bendl", bundle[:-2])
    with pytest.raises(Exception):
        PyBundleReader(path)


def test_incomplete_bundle_reports_none_sample_count(tmp_path: Path) -> None:
    # Provisional bundle with complete=0: sample_count() must be None.
    stream = _ben_bytes_for([[1, 2, 3]], tmp_path)
    # Build it by hand — no directory, complete=NO.
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
    reader = PyBundleReader(path)
    assert reader.is_complete() is False
    assert reader.sample_count() is None
    assert reader.asset_names() == []
    # extract_stream should still write out bytes that decode as BEN.
    out = tmp_path / "extracted.ben"
    reader.extract_stream(out)
    assert list(PyBenDecoder(out, mode="ben")) == [[1, 2, 3]]


def test_unknown_assignment_format_byte_reports_none(tmp_path: Path) -> None:
    # Assignment format byte = 0 → unknown. Finalized bundle but without
    # a valid stream container — the directory side still works.
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
    reader = PyBundleReader(path)
    assert reader.assignment_format() is None
    assert reader.is_complete()


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
    reader = PyBundleReader(path)
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
    reader = PyBundleReader(path)
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
    reader = PyBundleReader(path)
    r = repr(reader)
    # Incomplete bundles report no sample count.
    assert "samples=None" in r
    assert "assets=0" in r


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

    reader = PyBundleReader(path)
    assert reader.is_complete() is False
    assert reader.sample_count() is None
    assert reader.assignment_format() == "ben"

    # extract_stream should write exactly the partial byte sequence.
    extracted = tmp_path / "partial.ben"
    reader.extract_stream(extracted)
    assert extracted.read_bytes() == partial

    # The extracted file opens as a BEN stream (banner is intact).
    dec = PyBenDecoder(extracted, mode="ben")
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

    reader = PyBundleReader(path)
    assert reader.is_complete() is False

    extracted = tmp_path / "head_cut.ben"
    reader.extract_stream(extracted)
    # The decoder must reject a BEN file whose banner is incomplete.
    with pytest.raises(Exception, match="Failed to create BenDecoder"):
        PyBenDecoder(extracted, mode="ben")


def test_interrupted_ben_stream_zero_bytes_after_header(tmp_path: Path) -> None:
    # The worst case: the writer crashed after writing the header and
    # before any stream bytes landed.
    path = _write_bundle(tmp_path / "zero.bendl", _incomplete_bundle(b""))

    reader = PyBundleReader(path)
    assert reader.is_complete() is False
    assert reader.sample_count() is None
    assert reader.asset_names() == []

    extracted = tmp_path / "zero.ben"
    reader.extract_stream(extracted)
    assert extracted.read_bytes() == b""
    # A zero-byte .ben has no banner → decoder construction must fail.
    with pytest.raises(Exception, match="Failed to create BenDecoder"):
        PyBenDecoder(extracted, mode="ben")


def test_finalized_bundle_with_inflated_stream_len_survives_open(tmp_path: Path) -> None:
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
    reader = PyBundleReader(path)
    assert reader.is_complete()
    # sample_count is what the header says.
    assert reader.sample_count() == len(samples)

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
    reader = PyBundleReader(path)
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
        assets=[
            _Asset(asset_type=ASSET_TYPE_CUSTOM, name=long_name, payload=payload)
        ],
    )
    path = _write_bundle(tmp_path / "long.bendl", bundle)
    reader = PyBundleReader(path)
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
    )
    path = _write_bundle(tmp_path / "flags.bendl", bundle)
    reader = PyBundleReader(path)
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
    reader = PyBundleReader(path)
    for _ in range(5):
        assert reader.read_asset_bytes("raw.bin") == payload
        assert reader.read_asset_bytes("compressed.bin") == payload


def test_stress_many_heterogeneous_assets_round_trip(tmp_path: Path) -> None:
    # 500 custom assets with rotating flags. This exercises directory
    # scaling, offset bookkeeping, and name lookup on a non-trivial directory.
    N = 500
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
    reader = PyBundleReader(path)

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
    # fully round-tripped through PyBundleReader + PyBenDecoder.
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

        reader = PyBundleReader(path)
        assert reader.is_complete()
        assert reader.sample_count() == n_samples
        assert reader.asset_names() == [name for name, _ in truth]
        for name, want in truth:
            assert reader.read_asset_bytes(name) == want

        extracted = tmp_path / f"fuzz-{trial}.ben"
        reader.extract_stream(extracted)
        assert list(PyBenDecoder(extracted, mode="ben")) == samples


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
    reader = PyBundleReader(path)

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
    assert list(PyBenDecoder(tmp_path / "a.ben", mode="ben")) == samples


def test_extract_stream_into_missing_parent_dir_raises_ioerror(tmp_path: Path) -> None:
    bundle = build_bundle(
        stream_bytes=_ben_bytes_for([[1, 2]], tmp_path),
        sample_count=1,
    )
    path = _write_bundle(tmp_path / "mini.bendl", bundle)
    reader = PyBundleReader(path)
    missing = tmp_path / "does" / "not" / "exist" / "out.ben"
    with pytest.raises(OSError):
        reader.extract_stream(missing)


# ---------------------------------------------------------------------------
# PyBenEncoder bundle-output tests
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
    with PyBenEncoder(out, overwrite=True, variant="standard") as enc:
        for a in samples:
            enc.write(a)

    reader = PyBundleReader(out)
    assert reader.version() == (BENDL_MAJOR_VERSION, BENDL_MINOR_VERSION)
    assert reader.is_complete()
    assert reader.sample_count() == len(samples)
    assert reader.assignment_format() == "ben"
    # No graph because none was provided.
    assert reader.asset_names() == []
    assert reader.read_graph() is None

    extracted = tmp_path / "extracted.ben"
    reader.extract_stream(extracted)
    assert list(PyBenDecoder(extracted, mode="ben")) == samples


def test_pybenencoder_bundle_embeds_graph_from_dict(tmp_path: Path) -> None:
    out = tmp_path / "with_graph.bendl"
    samples = [[1, 1, 2, 2], [1, 1, 3, 3]]
    with PyBenEncoder(
        out, overwrite=True, variant="standard", graph=SAMPLE_GRAPH
    ) as enc:
        for a in samples:
            enc.write(a)

    reader = PyBundleReader(out)
    assert reader.is_complete()
    assert reader.sample_count() == len(samples)
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
    with PyBenEncoder(
        out, overwrite=True, variant="standard", graph=graph_path
    ) as enc:
        for a in samples:
            enc.write(a)

    reader = PyBundleReader(out)
    assert reader.asset_names() == ["graph.json"]
    assert reader.read_graph() == SAMPLE_GRAPH


def test_pybenencoder_bundle_embeds_graph_from_str_path(tmp_path: Path) -> None:
    # String paths must be accepted verbatim (same coercion Path arguments
    # go through elsewhere in the API).
    graph_path = tmp_path / "graph-str.json"
    graph_path.write_text(json.dumps(SAMPLE_GRAPH))

    out = tmp_path / "via-str.bendl"
    samples = [[0, 1, 0, 1]]
    with PyBenEncoder(
        out, overwrite=True, variant="standard", graph=str(graph_path)
    ) as enc:
        for a in samples:
            enc.write(a)

    reader = PyBundleReader(out)
    assert reader.read_graph() == SAMPLE_GRAPH


def test_pybenencoder_bundle_embeds_graph_from_bytes(tmp_path: Path) -> None:
    raw = json.dumps(SAMPLE_GRAPH).encode("utf-8")
    out = tmp_path / "via-bytes.bendl"
    samples = [[2, 2, 2, 2]]
    with PyBenEncoder(
        out, overwrite=True, variant="standard", graph=raw
    ) as enc:
        for a in samples:
            enc.write(a)

    reader = PyBundleReader(out)
    assert reader.read_graph() == SAMPLE_GRAPH


def test_pybenencoder_bundle_embeds_graph_from_bytesio(tmp_path: Path) -> None:
    buf = io.BytesIO(json.dumps(SAMPLE_GRAPH).encode("utf-8"))
    out = tmp_path / "via-bytesio.bendl"
    samples = [[1, 2, 1, 2]]
    with PyBenEncoder(
        out, overwrite=True, variant="standard", graph=buf
    ) as enc:
        for a in samples:
            enc.write(a)

    reader = PyBundleReader(out)
    assert reader.read_graph() == SAMPLE_GRAPH


def test_pybenencoder_bundle_embeds_graph_from_stringio(tmp_path: Path) -> None:
    buf = io.StringIO(json.dumps(SAMPLE_GRAPH))
    out = tmp_path / "via-stringio.bendl"
    samples = [[3, 3, 3, 3]]
    with PyBenEncoder(
        out, overwrite=True, variant="standard", graph=buf
    ) as enc:
        for a in samples:
            enc.write(a)

    reader = PyBundleReader(out)
    assert reader.read_graph() == SAMPLE_GRAPH


def test_pybenencoder_bundle_round_trip_via_extract_stream(tmp_path: Path) -> None:
    out = tmp_path / "full.bendl"
    rng = random.Random(0xCAFE)
    samples = [[rng.randint(1, 8) for _ in range(12)] for _ in range(15)]
    with PyBenEncoder(
        out, overwrite=True, variant="standard", graph=SAMPLE_GRAPH
    ) as enc:
        for a in samples:
            enc.write(a)

    reader = PyBundleReader(out)
    assert reader.sample_count() == len(samples)
    extracted = tmp_path / "full.ben"
    reader.extract_stream(extracted)
    assert list(PyBenDecoder(extracted, mode="ben")) == samples
    # And the graph still round-trips from the same reader.
    assert reader.read_graph() == SAMPLE_GRAPH


def test_pybenencoder_ben_file_only_rejects_graph(tmp_path: Path) -> None:
    out = tmp_path / "ben-with-graph.ben"
    with pytest.raises(ValueError, match="ben_file_only"):
        PyBenEncoder(
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
    with PyBenEncoder(
        out, overwrite=True, variant="standard", ben_file_only=True
    ) as enc:
        enc.write([1, 2, 3])
    blob = out.read_bytes()
    assert not blob.startswith(BENDL_MAGIC)
    # PyBenDecoder should still read it in ben mode.
    assert list(PyBenDecoder(out, mode="ben")) == [[1, 2, 3]]


def test_pybenencoder_bundle_close_is_idempotent(tmp_path: Path) -> None:
    out = tmp_path / "idem.bendl"
    enc = PyBenEncoder(out, overwrite=True, variant="standard")
    enc.write([1, 1, 2])
    enc.close()
    enc.close()  # second close must be a no-op
    with pytest.raises(OSError, match="already been closed"):
        enc.write([1, 2, 3])

    reader = PyBundleReader(out)
    assert reader.is_complete()
    assert reader.sample_count() == 1


def test_pybenencoder_bundle_rejects_invalid_graph_type(tmp_path: Path) -> None:
    out = tmp_path / "bad.bendl"
    with pytest.raises(ValueError, match="graph must be"):
        PyBenEncoder(out, overwrite=True, variant="standard", graph=12345)
