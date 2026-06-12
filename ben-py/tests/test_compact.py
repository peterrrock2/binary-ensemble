"""Tests for the ``_core`` bundle-compaction machinery (dead-space reclamation).

Compaction must be *semantically invisible*: same stream bytes, same decoded asset payloads,
same metadata, same wire format — just no unreferenced byte ranges (left behind by
directory-only removals and superseded directories). These tests pin both halves: the space
actually comes back, and nothing else changes. The public facade has no standalone compact —
the facade transforms (``remove_asset``, ``compress_stream``, ``relabel_bundle``) emit
compact bundles themselves, while appends leave a small superseded directory behind — so the
machinery is exercised through ``_core``, which also reports which strategy ran
(``"none"`` / ``"tail"`` / ``"full"``).
"""

from __future__ import annotations

import json
import os
import random
import stat
from pathlib import Path

import pytest

from binary_ensemble import _core
from binary_ensemble.bundle import BendlDecoder, BendlEncoder, compress_stream

EXAMPLE_GRAPH = Path(__file__).resolve().parent / "data" / "gerrymandria.json"


def _graph():
    return json.loads(EXAMPLE_GRAPH.read_text())


def _n():
    return len(_graph()["nodes"])


def _build_bundle_with_dead_space(path: Path) -> tuple[list[list[int]], int]:
    """A finalized bundle that has been appended to and had a large asset removed.

    Returns ``(samples, live_size)`` where ``live_size`` is the file size before the bloating
    asset was added — an upper bound on what a compacted file may occupy (compaction also drops
    the superseded directories the appends left behind).
    """
    n = _n()
    samples = [[(i + j) % 4 + 1 for j in range(n)] for i in range(8)]
    enc = BendlEncoder(path, overwrite=True)
    enc.add_graph(_graph(), sort=None)
    enc.add_metadata({"seed": 99})
    with enc.stream() as s:
        for a in samples:
            s.write(a)
    enc.add_asset("notes.txt", "keep me", content_type="text")
    live_size = path.stat().st_size

    # Bloat: a genuinely incompressible 64 KiB blob (seeded random bytes — a periodic pattern
    # would be crushed by the xz storage compression and leave no dead space), removed through
    # the *core* binding, whose removal is directory-only (the facade's remove_asset compacts
    # automatically, which would destroy the dead space these tests exist to exercise).
    blob = random.Random(0).randbytes(64 * 1024)
    core_appender = _core.BendlEncoder.append(path)
    core_appender.add_asset("bloat.bin", blob, "binary")
    core_appender.remove_asset("bloat.bin")
    return samples, live_size


def test_compact_reclaims_dead_space_and_preserves_everything(tmp_path: Path) -> None:
    path = tmp_path / "in.bendl"
    samples, live_size = _build_bundle_with_dead_space(path)
    bloated_size = path.stat().st_size
    assert bloated_size > live_size + 60_000  # the dead bytes really are in the file

    before = BendlDecoder(path)
    stream_size_before = before.stream_size()
    names_before = before.asset_names()

    _core.compact_bundle_in_place(path)  # in place

    assert path.stat().st_size <= live_size
    after = BendlDecoder(path)
    # Semantically identical: same plans, same assets, same metadata, same wire format.
    assert list(after) == samples
    assert after.asset_names() == names_before
    assert after.read_metadata() == {"seed": 99}
    assert after.read_asset_bytes("notes.txt") == b"keep me"
    assert after.assignment_format() == "ben"
    assert len(after) == len(samples)
    # The stream is copied verbatim, so its recorded size is unchanged.
    assert after.stream_size() == stream_size_before
    # And every checksum in the compacted bundle holds.
    after.verify()


def test_compact_copies_stream_bytes_verbatim(tmp_path: Path) -> None:
    path = tmp_path / "in.bendl"
    _build_bundle_with_dead_space(path)

    before_stream = tmp_path / "before.ben"
    BendlDecoder(path).extract_stream(before_stream)
    _core.compact_bundle_in_place(path)
    after_stream = tmp_path / "after.ben"
    BendlDecoder(path).extract_stream(after_stream)

    assert before_stream.read_bytes() == after_stream.read_bytes()


def test_in_place_compaction_picks_tail_rewrite_for_post_stream_dead_space(
    tmp_path: Path,
) -> None:
    path = tmp_path / "in.bendl"
    _build_bundle_with_dead_space(path)  # dead space is post-stream by construction
    # The fast path rebuilds only the tail (the stream is never read), and a second pass
    # finds nothing left to reclaim.
    assert _core.compact_bundle_in_place(path) == "tail"
    assert _core.compact_bundle_in_place(path) == "none"
    BendlDecoder(path).verify()


def test_in_place_compaction_full_rewrite_for_pre_stream_dead_space(tmp_path: Path) -> None:
    path = tmp_path / "in.bendl"
    _build_bundle_with_dead_space(path)
    # graph.json is a pre-stream asset: removing it (directory-only, via the core binding)
    # leaves dead bytes before the stream, which only the full rewrite can reclaim.
    core = _core.BendlEncoder.append(path)
    core.remove_asset("graph.json")
    assert _core.compact_bundle_in_place(path) == "full"
    dec = BendlDecoder(path)
    assert "graph.json" not in dec.asset_names()
    dec.verify()


def test_compact_is_idempotent(tmp_path: Path) -> None:
    path = tmp_path / "in.bendl"
    _build_bundle_with_dead_space(path)
    _core.compact_bundle_in_place(path)
    once = path.read_bytes()
    _core.compact_bundle_in_place(path)
    assert path.read_bytes() == once


def test_compact_preserves_xben_bundles(tmp_path: Path) -> None:
    src = tmp_path / "in.bendl"
    samples, _live = _build_bundle_with_dead_space(src)
    xben = tmp_path / "in.xben.bendl"
    compress_stream(src, out_file=xben)

    # Manufacture dead space in the XBEN bundle via the directory-only core removal,
    # then compact it.
    core_appender = _core.BendlEncoder.append(xben)
    core_appender.add_asset("temp.bin", b"\x00" * 4096, "binary")
    core_appender.remove_asset("temp.bin")
    bloated = xben.stat().st_size
    _core.compact_bundle_in_place(xben)

    assert xben.stat().st_size < bloated
    after = BendlDecoder(xben)
    assert after.assignment_format() == "xben"  # wire format preserved, not re-encoded
    assert list(after) == samples
    after.verify()


def test_compact_out_file_mode_and_overwrite(tmp_path: Path) -> None:
    src = tmp_path / "in.bendl"
    samples, _live = _build_bundle_with_dead_space(src)
    src_bytes = src.read_bytes()

    out = tmp_path / "out.bendl"
    out.write_bytes(b"existing")
    with pytest.raises(OSError, match="already exists"):
        _core.compact_bundle(src, out)
    _core.compact_bundle(src, out, overwrite=True)

    # Original untouched; the copy is the compacted one.
    assert src.read_bytes() == src_bytes
    assert out.stat().st_size < src.stat().st_size
    assert list(BendlDecoder(out)) == samples


def test_compact_assets_only_bundle(tmp_path: Path) -> None:
    path = tmp_path / "assets.bendl"
    with BendlEncoder(path, overwrite=True) as enc:
        enc.add_metadata({"only": "assets"})
        enc.add_asset("a.txt", "alpha", content_type="text")
    enc = BendlEncoder.append(path)
    enc.add_asset("b.txt", "beta", content_type="text")
    enc.remove_asset("a.txt")

    _core.compact_bundle_in_place(path)

    dec = BendlDecoder(path)
    assert dec.is_complete()
    assert dec.stream_size() == 0
    assert list(dec) == []
    assert dec.asset_names() == ["metadata.json", "b.txt"]
    assert dec.read_asset_bytes("b.txt") == b"beta"
    dec.verify()


def test_compact_rejects_unfinalized_bundle(tmp_path: Path) -> None:
    path = tmp_path / "partial.bendl"
    with pytest.raises(RuntimeError, match="boom"):
        with BendlEncoder(path, overwrite=True) as enc:
            with enc.stream() as s:
                s.write([1] * _n())
                raise RuntimeError("boom")
    with pytest.raises(Exception, match="finalized"):
        _core.compact_bundle_in_place(path)


def _flip_byte_at(path: Path, marker: bytes) -> None:
    """XOR the first byte of ``marker`` wherever it occurs in the file."""
    data = bytearray(path.read_bytes())
    pos = data.find(marker)
    assert pos != -1, f"marker {marker!r} not found"
    data[pos] ^= 0xFF
    path.write_bytes(bytes(data))


def test_full_compact_refuses_corrupt_stream(tmp_path: Path) -> None:
    path = tmp_path / "in.bendl"
    _build_bundle_with_dead_space(path)
    # Flip a byte inside the stream region (the stream's banner — the default variant is
    # twodelta). The full rewrite copies the stream through the verified reader, so it must
    # refuse and must not leave a destination file behind.
    _flip_byte_at(path, b"TWODELTA BEN FILE")
    corrupted = path.read_bytes()

    out = tmp_path / "out.bendl"
    with pytest.raises(Exception):
        _core.compact_bundle(path, out, overwrite=True)
    assert path.read_bytes() == corrupted  # source untouched
    assert not out.exists()  # no partial destination left behind
    assert list(tmp_path.glob("*.tmp")) == []  # and no stray temp files either

    # The in-place form takes the tail-rewrite fast path here (all dead space is post-stream),
    # which by design never reads the stream — so it succeeds, the corruption travels along
    # unread, and verify() is what catches it. This is the documented trade-off that makes
    # removal O(tail) instead of O(stream) on huge bundles.
    _core.compact_bundle_in_place(path)
    with pytest.raises(Exception):
        BendlDecoder(path).verify()


def test_compact_refuses_corrupt_asset(tmp_path: Path) -> None:
    path = tmp_path / "in.bendl"
    _build_bundle_with_dead_space(path)
    _flip_byte_at(path, b"keep me")  # corrupt the notes.txt payload bytes (post-stream)
    # The full rewrite decodes every asset (verify-on-touch) and must refuse.
    with pytest.raises(Exception):
        _core.compact_bundle(path, tmp_path / "out.bendl", overwrite=True)
    # The in-place tail path relocates post-stream assets as raw bytes without decoding; the
    # corruption travels along with its (now mismatching) stored checksum, and verify()
    # catches it.
    _core.compact_bundle_in_place(path)
    with pytest.raises(Exception):
        BendlDecoder(path).verify()


def test_public_append_leaves_a_superseded_directory(tmp_path: Path) -> None:
    """Pins the dead-space story the docs tell: an immediate-commit ``add_asset`` supersedes
    the previous directory (a few dead bytes, reported as ``"tail"``-reclaimable), while the
    facade transforms emit compact bundles themselves."""
    path = tmp_path / "appended.bendl"
    enc = BendlEncoder(path, overwrite=True)
    with enc.stream() as s:
        s.write([1, 2, 3])
    enc.add_asset("notes.txt", "hello", content_type="text")  # commits immediately

    assert _core.compact_bundle_in_place(path) == "tail"
    assert _core.compact_bundle_in_place(path) == "none"

    dec = BendlDecoder(path)
    assert dec.read_asset_bytes("notes.txt") == b"hello"
    assert list(dec) == [[1, 2, 3]]
    dec.verify()


def test_facade_remove_asset_leaves_bundle_fully_compact(tmp_path: Path) -> None:
    """The facade's remove_asset compacts in place: a follow-up compaction finds nothing."""
    path = tmp_path / "removed.bendl"
    enc = BendlEncoder(path, overwrite=True)
    with enc.stream() as s:
        s.write([1, 2, 3])
    enc.add_asset("a.txt", "a", content_type="text")
    enc.add_asset("b.txt", "b", content_type="text")
    enc.remove_asset("a.txt")

    assert _core.compact_bundle_in_place(path) == "none"
    dec = BendlDecoder(path)
    assert dec.asset_names() == ["b.txt"]
    dec.verify()


def test_facade_remove_asset_failure_leaves_bundle_untouched(tmp_path: Path) -> None:
    """Removal and compaction commit together: when the rewrite fails mid-way (a corrupt
    surviving asset caught by verify-on-touch), the bundle is left byte-identical — the asset
    is still present and a retry still sees it. The removal used to commit its directory drop
    first, so a failed compaction left the asset already unreachable and a retry raised
    KeyError."""
    path = tmp_path / "atomic.bendl"
    enc = BendlEncoder(path, overwrite=True)
    enc.add_graph(_graph(), sort=None)
    with enc.stream() as s:
        s.write([1] * _n())
    enc.add_asset("notes.txt", "keep me", content_type="text")
    _flip_byte_at(path, b"keep me")  # corrupt the surviving post-stream asset

    before = path.read_bytes()
    # Removing the pre-stream graph forces the full rewrite, which reads every survivor.
    with pytest.raises(Exception, match="checksum"):
        enc.remove_asset("graph.json")
    assert path.read_bytes() == before
    assert "graph.json" in BendlDecoder(path).asset_names()


def test_facade_remove_asset_can_remove_a_corrupt_asset(tmp_path: Path) -> None:
    """The asset being removed is never read, so removal is the way out of a corrupt-asset
    situation, not blocked by it."""
    path = tmp_path / "corrupt-removal.bendl"
    enc = BendlEncoder(path, overwrite=True)
    with enc.stream() as s:
        s.write([1, 2, 3])
    enc.add_asset("bad.txt", "doomed bytes", content_type="text")
    _flip_byte_at(path, b"doomed bytes")

    enc.remove_asset("bad.txt")
    dec = BendlDecoder(path)
    assert dec.asset_names() == []
    dec.verify()


def test_failed_out_file_write_preserves_existing_destination(tmp_path: Path) -> None:
    """With overwrite=True, the destination used to be truncated up front, so a mid-write
    failure destroyed a previously good file. The temp-then-rename guard must leave it
    byte-identical."""
    src = tmp_path / "src.bendl"
    _build_bundle_with_dead_space(src)
    _flip_byte_at(src, b"keep me")  # corrupt a post-stream asset: the full rewrite refuses

    dest = tmp_path / "dest.bendl"
    _build_bundle_with_dead_space(dest)  # a valid bundle that must survive the failure
    dest_bytes = dest.read_bytes()

    with pytest.raises(Exception):
        _core.compact_bundle(src, dest, overwrite=True)
    assert dest.read_bytes() == dest_bytes
    assert list(tmp_path.glob("*.tmp")) == []


@pytest.mark.skipif(os.name != "posix", reason="POSIX file modes")
def test_in_place_transforms_preserve_file_mode(tmp_path: Path) -> None:
    """In-place transforms swap a temp file over the bundle; the swap must inherit the
    bundle's permissions (a 0o640 group-shared file must not silently become 0o600 or
    umask-default, and a private one must not go world-readable)."""
    path = tmp_path / "modes.bendl"
    _build_bundle_with_dead_space(path)
    os.chmod(path, 0o640)

    # Facade in-place recompression (BEN -> XBEN) goes through the temp-output guard.
    compress_stream(path)
    assert stat.S_IMODE(path.stat().st_mode) == 0o640

    # A pre-stream removal forces the core full rewrite's own temp-file swap.
    enc = BendlEncoder.append(path)
    enc.remove_asset("graph.json")
    assert stat.S_IMODE(path.stat().st_mode) == 0o640
