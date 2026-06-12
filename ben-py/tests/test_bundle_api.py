"""Lifecycle tests for the ``BendlEncoder`` authoring facade.

Covers create vs append mode, the single-use stream session, asset/graph/metadata
adds before and after the stream, content-type validation, graph↔chain node-count
validation, the assets-only (stream-less) bundle, and the unfinalized-on-exception
recovery path.
"""

from __future__ import annotations

import json
import os
from pathlib import Path

import pytest

from binary_ensemble.bundle import BendlDecoder, BendlEncoder

EXAMPLE_GRAPH = Path(__file__).resolve().parent / "data" / "gerrymandria.json"


def _graph():
    return json.loads(EXAMPLE_GRAPH.read_text())


def _n():
    return len(_graph()["nodes"])


# ---------------------------------------------------------------------------
# Round trips
# ---------------------------------------------------------------------------


def test_create_round_trip_all_asset_kinds(tmp_path: Path) -> None:
    n = _n()
    samples = [[(i + j) % 4 + 1 for j in range(n)] for i in range(6)]
    path = tmp_path / "full.bendl"
    with BendlEncoder(path, overwrite=True) as enc:
        returned = enc.add_graph(_graph(), sort=None)
        enc.add_metadata({"seed": 1234})
        with enc.stream() as stream:
            for a in samples:
                stream.write(a)
        enc.add_asset("notes.txt", "hello world", content_type="text")
        enc.add_asset("post.json", json.dumps({"k": [1, 2, 3]}), content_type="json")

    # add_graph hands back a live NetworkX graph.
    assert returned.number_of_nodes() == n
    dec = BendlDecoder(path)
    assert dec.is_complete()
    assert dec.count_samples() == len(samples)
    assert dec.assignment_format() == "ben"
    assert dec.asset_names() == [
        "graph.json",
        "metadata.json",
        "notes.txt",
        "post.json",
    ]
    assert dec.read_metadata() == {"seed": 1234}
    assert dec.read_asset_bytes("notes.txt") == b"hello world"
    assert dec.read_json_asset("post.json") == {"k": [1, 2, 3]}
    assert dec.read_node_permutation_map() is None  # raw graph => no perm map
    assert list(dec) == samples


def test_post_stream_add_commits_immediately(tmp_path: Path) -> None:
    path = tmp_path / "commit.bendl"
    enc = BendlEncoder(path, overwrite=True)
    with enc.stream() as s:
        s.write([1, 2])
    enc.add_asset("a.txt", "one", content_type="text")
    # A successful post-stream add is durable on disk before close().
    assert BendlDecoder(path).read_asset_bytes("a.txt") == b"one"
    enc.add_asset("b.txt", "two", content_type="text")
    enc.close()
    assert BendlDecoder(path).asset_names() == ["a.txt", "b.txt"]


def test_context_manager_and_idempotent_close(tmp_path: Path) -> None:
    path = tmp_path / "ctx.bendl"
    enc = BendlEncoder(path, overwrite=True)
    with enc.stream() as s:
        s.write([1, 2, 3])
    enc.close()
    enc.close()  # idempotent
    assert list(BendlDecoder(path)) == [[1, 2, 3]]


def test_overwrite_required_for_existing_path(tmp_path: Path) -> None:
    path = tmp_path / "exists.bendl"
    path.write_bytes(b"existing")
    with pytest.raises(OSError, match="already exists"):
        BendlEncoder(path, overwrite=False)
    enc = BendlEncoder(path, overwrite=True)
    enc.close()
    assert BendlDecoder(path).is_complete()


# ---------------------------------------------------------------------------
# Assets-only bundle (stream-less close)
# ---------------------------------------------------------------------------


def test_stream_less_close_yields_assets_only_bundle(tmp_path: Path) -> None:
    path = tmp_path / "assets_only.bendl"
    enc = BendlEncoder(path, overwrite=True)
    enc.add_metadata({"only": "assets"})
    enc.close()
    dec = BendlDecoder(path)
    assert dec.is_complete()
    assert dec.count_samples() == 0
    assert len(dec) == 0
    assert list(dec) == []
    assert dec.read_metadata() == {"only": "assets"}


def test_empty_close_with_no_assets(tmp_path: Path) -> None:
    path = tmp_path / "truly_empty.bendl"
    with BendlEncoder(path, overwrite=True):
        pass
    dec = BendlDecoder(path)
    assert dec.is_complete()
    assert dec.asset_names() == []
    assert list(dec) == []


# ---------------------------------------------------------------------------
# Exception inside stream context
# ---------------------------------------------------------------------------


def test_exception_in_stream_leaves_bundle_unfinalized(tmp_path: Path) -> None:
    path = tmp_path / "fail.bendl"
    with pytest.raises(RuntimeError, match="boom"):
        with BendlEncoder(path, overwrite=True) as enc:
            with enc.stream() as s:
                s.write([1, 2, 3])
                raise RuntimeError("boom")
    dec = BendlDecoder(path)
    assert dec.is_complete() is False
    # Verified extraction refuses an unfinalized stream...
    with pytest.raises(Exception, match="unfinalized"):
        dec.extract_stream(tmp_path / "recovered.ben")
    # ...but the partial write is recoverable.
    dec.extract_stream(tmp_path / "recovered.ben", overwrite=True, allow_unfinalized=True)
    assert (tmp_path / "recovered.ben").stat().st_size > 0


# ---------------------------------------------------------------------------
# content_type validation
# ---------------------------------------------------------------------------


def test_add_asset_content_type_validation(tmp_path: Path) -> None:
    enc = BendlEncoder(tmp_path / "v.bendl", overwrite=True)
    with pytest.raises(ValueError, match="must be 'json', 'text', 'binary', or 'file'"):
        enc.add_asset("x", b"data", content_type="parquet")
    with pytest.raises(ValueError, match="valid UTF-8 JSON"):
        enc.add_asset("bad.json", "not json", content_type="json")
    with pytest.raises(ValueError, match="valid UTF-8"):
        enc.add_asset("bad.txt", b"\xff\xfe", content_type="text")
    # Valid forms succeed.
    enc.add_asset("ok.json", '{"a":1}', content_type="json")
    enc.add_asset("ok.txt", "fine", content_type="text")
    enc.add_asset("ok.bin", b"\xff\xfe\x00\x01", content_type="binary")
    enc.close()
    dec = BendlDecoder(tmp_path / "v.bendl")
    assert dec.read_json_asset("ok.json") == {"a": 1}
    flags = {a["name"]: a["flags"] for a in dec.list_assets()}
    assert "json" in flags["ok.json"]
    assert "json" not in flags["ok.txt"]
    assert "json" not in flags["ok.bin"]


def test_add_asset_accepts_dict_and_list_payloads(tmp_path: Path) -> None:
    enc = BendlEncoder(tmp_path / "d.bendl", overwrite=True)
    enc.add_asset("scores.json", {"cut_edges": [10, 12]}, content_type="json")
    enc.add_asset("steps.json", [1, 2, 3], content_type="json")
    # dict/list payloads are JSON by definition; other content types are a caller mistake.
    with pytest.raises(ValueError, match="require content_type='json'"):
        enc.add_asset("bad.bin", {"a": 1}, content_type="binary")
    enc.close()

    dec = BendlDecoder(tmp_path / "d.bendl")
    assert dec.read_json_asset("scores.json") == {"cut_edges": [10, 12]}
    assert dec.read_json_asset("steps.json") == [1, 2, 3]


def test_add_asset_accepts_paths_and_file_likes(tmp_path: Path) -> None:
    import io

    blob = bytes(range(256))
    src = tmp_path / "geometry.gpkg"
    src.write_bytes(blob)

    enc = BendlEncoder(tmp_path / "f.bendl", overwrite=True)
    # pathlib.Path payload: the file at that path is read. (A plain str would be stored as
    # UTF-8 *content*, never treated as a path.)
    enc.add_asset("from_path.gpkg", src, content_type="binary")
    # File-like payloads are read; binary and text handles both work.
    enc.add_asset("from_filelike.gpkg", io.BytesIO(blob), content_type="binary")
    enc.add_asset("from_text_handle.txt", io.StringIO("hello"), content_type="text")
    enc.close()

    dec = BendlDecoder(tmp_path / "f.bendl")
    assert dec.read_asset_bytes("from_path.gpkg") == blob
    assert dec.read_asset_bytes("from_filelike.gpkg") == blob
    assert dec.read_asset_bytes("from_text_handle.txt") == b"hello"


def test_add_asset_file_content_type_reads_paths(tmp_path: Path) -> None:
    blob = bytes(range(256))
    src = tmp_path / "geometry.gpkg"
    src.write_bytes(blob)

    enc = BendlEncoder(tmp_path / "p.bendl", overwrite=True)
    # Under content_type="file", a plain str payload *is* a path — the explicit opt-in that
    # resolves the str-is-content default of every other content type.
    enc.add_asset("from_str_path.gpkg", str(src), content_type="file")
    enc.add_asset("from_pathlib.gpkg", src, content_type="file")
    with pytest.raises(TypeError, match="requires a str or os.PathLike"):
        enc.add_asset("bad", b"raw bytes are not a path", content_type="file")
    with pytest.raises(FileNotFoundError):
        enc.add_asset("missing", tmp_path / "nope.gpkg", content_type="file")
    enc.close()

    dec = BendlDecoder(tmp_path / "p.bendl")
    assert dec.read_asset_bytes("from_str_path.gpkg") == blob
    assert dec.read_asset_bytes("from_pathlib.gpkg") == blob


def test_binary_asset_round_trips_arbitrary_bytes(tmp_path: Path) -> None:
    # A blob that is deliberately not valid UTF-8 and not JSON — the shape of a zipped
    # shapefile or GeoPackage — must round-trip byte-exactly under CRC protection.
    blob = bytes(range(256)) * 5
    enc = BendlEncoder(tmp_path / "blob.bendl", overwrite=True)
    enc.add_asset("tracts.gpkg", blob, content_type="binary")
    enc.close()

    dec = BendlDecoder(tmp_path / "blob.bendl")
    assert dec.read_asset_bytes("tracts.gpkg") == blob


def test_large_assets_compress_transparently(tmp_path: Path) -> None:
    # Payloads at or above the writer's 1 KiB threshold are xz-compressed on disk by default;
    # the read side decompresses transparently, so round-trips are unaffected.
    big_json = json.dumps({"scores": list(range(2000))})
    assert len(big_json) >= 1024
    enc = BendlEncoder(tmp_path / "big.bendl", overwrite=True)
    enc.add_asset("scores.json", big_json, content_type="json")
    enc.close()

    dec = BendlDecoder(tmp_path / "big.bendl")
    assert dec.read_json_asset("scores.json") == {"scores": list(range(2000))}


# ---------------------------------------------------------------------------
# add_graph reorder / raw / validation
# ---------------------------------------------------------------------------


def test_add_graph_reorder_emits_graph_and_permutation_map(tmp_path: Path) -> None:
    n = _n()
    path = tmp_path / "reord.bendl"
    enc = BendlEncoder(path, overwrite=True)
    reordered = enc.add_graph(_graph(), sort="rcm")
    with enc.stream() as s:
        s.write([1] * n)
    enc.close()

    dec = BendlDecoder(path)
    assert dec.asset_names() == ["graph.json", "node_permutation_map.json"]
    # add_graph and read_graph both hand back live NetworkX graphs with matching
    # nodes in the same (reordered) order.
    assert list(reordered.nodes) == list(dec.read_graph().nodes)
    assert reordered.number_of_nodes() == n
    pmap = dec.read_node_permutation_map()
    mapping = pmap["node_permutation_old_to_new"]
    assert len(mapping) == n
    # old->new is a bijection over [0, n).
    assert sorted(int(k) for k in mapping) == list(range(n))
    assert sorted(mapping.values()) == list(range(n))


def test_add_graph_none_stores_raw_without_permutation_map(tmp_path: Path) -> None:
    path = tmp_path / "raw.bendl"
    enc = BendlEncoder(path, overwrite=True)
    enc.add_graph(_graph(), sort=None)
    enc.close()
    dec = BendlDecoder(path)
    assert dec.asset_names() == ["graph.json"]
    assert dec.read_node_permutation_map() is None


def test_add_graph_defaults_to_mlc_reorder(tmp_path: Path) -> None:
    # With no sort given, add_graph reorders via MLC (the default) and stores a map.
    path = tmp_path / "default.bendl"
    enc = BendlEncoder(path, overwrite=True)
    returned = enc.add_graph(_graph())
    enc.close()
    assert returned.number_of_nodes() == _n()
    dec = BendlDecoder(path)
    assert dec.asset_names() == ["graph.json", "node_permutation_map.json"]
    assert dec.read_node_permutation_map()["ordering_method"] == "multi-level-cluster"


def test_add_graph_node_count_mismatch_raises(tmp_path: Path) -> None:
    n = _n()
    enc = BendlEncoder(tmp_path / "nc.bendl", overwrite=True)
    enc.add_graph(_graph(), sort=None)
    with enc.stream() as s:
        s.write([1] * n)  # correct
        with pytest.raises(ValueError, match="does not match graph node count"):
            s.write([1] * (n - 1))


def test_reorder_add_graph_after_stream_raises_but_raw_succeeds(tmp_path: Path) -> None:
    n = _n()
    path = tmp_path / "after.bendl"
    enc = BendlEncoder(path, overwrite=True)
    with enc.stream() as s:
        s.write([1] * n)
    with pytest.raises(Exception, match="only allowed before"):
        enc.add_graph(_graph(), sort="rcm")
    # A raw graph attaches fine post-stream.
    enc.add_graph(_graph(), sort=None)
    enc.close()
    assert BendlDecoder(path).asset_names() == ["graph.json"]


def test_duplicate_graph_raises(tmp_path: Path) -> None:
    enc = BendlEncoder(tmp_path / "dup.bendl", overwrite=True)
    enc.add_graph(_graph(), sort=None)
    with pytest.raises(Exception, match="duplicate singleton"):
        enc.add_graph(_graph(), sort=None)


# ---------------------------------------------------------------------------
# Stream-signature and second-stream guards
# ---------------------------------------------------------------------------


def test_stream_takes_no_positional_arguments(tmp_path: Path) -> None:
    # The embedded stream is always BEN at write time (XBEN comes from compress_stream), so
    # stream() has no format parameter and variant is keyword-only — a stale positional call
    # must fail loudly, not bind to variant.
    enc = BendlEncoder(tmp_path / "fmt.bendl", overwrite=True)
    with pytest.raises(TypeError):
        enc.stream("ben")  # type: ignore


def test_stream_rejects_unknown_variant(tmp_path: Path) -> None:
    enc = BendlEncoder(tmp_path / "var.bendl", overwrite=True)
    with pytest.raises(ValueError, match="Unknown variant"):
        enc.stream(variant="xben")


def test_second_stream_refused(tmp_path: Path) -> None:
    path = tmp_path / "two.bendl"
    enc = BendlEncoder(path, overwrite=True)
    with enc.stream() as s:
        s.write([1, 2])
    with pytest.raises(Exception, match="already been written"):
        enc.stream()


# ---------------------------------------------------------------------------
# Append mode
# ---------------------------------------------------------------------------


def test_append_mode_adds_assets(tmp_path: Path) -> None:
    path = tmp_path / "app.bendl"
    with BendlEncoder(path, overwrite=True) as enc:
        with enc.stream() as s:
            s.write([1, 2, 3])

    ap = BendlEncoder.append(path)
    ap.add_metadata({"appended": True})
    ap.add_asset("late.txt", "added later", content_type="text")
    ap.close()

    dec = BendlDecoder(path)
    assert dec.read_metadata() == {"appended": True}
    assert dec.read_asset_bytes("late.txt") == b"added later"
    assert list(dec) == [[1, 2, 3]]


def test_append_mode_disallows_stream(tmp_path: Path) -> None:
    path = tmp_path / "app2.bendl"
    with BendlEncoder(path, overwrite=True) as enc:
        with enc.stream() as s:
            s.write([1])
    ap = BendlEncoder.append(path)
    with pytest.raises(Exception, match="append mode"):
        ap.stream()


def test_append_mode_reorder_graph_raises(tmp_path: Path) -> None:
    path = tmp_path / "app3.bendl"
    with BendlEncoder(path, overwrite=True) as enc:
        with enc.stream() as s:
            s.write([1] * _n())
    ap = BendlEncoder.append(path)
    with pytest.raises(Exception, match="only allowed before"):
        ap.add_graph(_graph(), sort="rcm")
    # Raw graph append works.
    ap.add_graph(_graph(), sort=None)
    ap.close()
    assert "graph.json" in BendlDecoder(path).asset_names()


def test_append_on_unfinalized_bundle_raises(tmp_path: Path) -> None:
    path = tmp_path / "unfin.bendl"
    with pytest.raises(RuntimeError):
        with BendlEncoder(path, overwrite=True) as enc:
            with enc.stream() as s:
                s.write([1, 2])
                raise RuntimeError("stop")
    with pytest.raises(Exception):
        BendlEncoder.append(path)


# ---------------------------------------------------------------------------
# Live NetworkX graph inputs
# ---------------------------------------------------------------------------


def test_add_graph_accepts_live_networkx_graph(tmp_path: Path) -> None:
    import networkx as nx

    live = nx.readwrite.json_graph.adjacency_graph(_graph())
    n = live.number_of_nodes()
    samples = [[(i + j) % 4 + 1 for j in range(n)] for i in range(4)]

    path = tmp_path / "live.bendl"
    enc = BendlEncoder(path, overwrite=True)
    stored = enc.add_graph(live, sort=None)
    # A raw (sort=None) embed of a live graph preserves its node iteration order.
    assert list(stored.nodes) == list(live.nodes)
    with enc.stream() as stream:
        for a in samples:
            stream.write(a)

    dec = BendlDecoder(path)
    assert list(dec) == samples
    assert list(dec.read_graph().nodes) == list(live.nodes)


def test_add_graph_accepts_networkx_graph_subclass(tmp_path: Path) -> None:
    # gerrychain.Graph is an nx.Graph subclass; pin that subclasses are accepted.
    import networkx as nx

    class SubGraph(nx.Graph):
        pass

    live = SubGraph(nx.readwrite.json_graph.adjacency_graph(_graph()))
    enc = BendlEncoder(tmp_path / "sub.bendl", overwrite=True)
    stored = enc.add_graph(live, sort=None)
    assert stored.number_of_nodes() == live.number_of_nodes()
    enc.close()


def test_add_metadata_rejects_networkx_graph(tmp_path: Path) -> None:
    # Graphs are graph.json material, not metadata; the metadata path must not
    # silently serialize one.
    import networkx as nx

    enc = BendlEncoder(tmp_path / "meta.bendl", overwrite=True)
    with pytest.raises(ValueError, match="metadata must be"):
        enc.add_metadata(nx.Graph())


# ---------------------------------------------------------------------------
# Stream size (header-recorded, no decoding)
# ---------------------------------------------------------------------------


def test_stream_size_matches_extracted_bytes(tmp_path: Path) -> None:
    n = _n()
    path = tmp_path / "s.bendl"
    enc = BendlEncoder(path, overwrite=True)
    enc.add_graph(_graph(), sort=None)
    with enc.stream() as s:
        for i in range(5):
            s.write([(i + j) % 3 + 1 for j in range(n)])

    # stream_size comes straight from the header and must equal the byte count
    # extract_stream copies out.
    dec = BendlDecoder(path)
    out = tmp_path / "out.ben"
    dec.extract_stream(out)
    assert dec.stream_size() > 0
    assert dec.stream_size() == out.stat().st_size


def test_stream_size_zero_for_assets_only(tmp_path: Path) -> None:
    path = tmp_path / "a.bendl"
    with BendlEncoder(path, overwrite=True) as enc:
        enc.add_metadata({"x": 1})
    assert BendlDecoder(path).stream_size() == 0


def test_asset_size_matches_directory_and_distinguishes_stored_from_decoded(
    tmp_path: Path,
) -> None:
    path = tmp_path / "sizes.bendl"
    with BendlEncoder(path, overwrite=True) as enc:
        enc.add_asset("small.txt", "tiny", content_type="text")
        # 5000 highly compressible bytes: above the 1 KiB threshold, so stored xz-compressed.
        enc.add_asset("big.txt", "x" * 5000, content_type="text")

    dec = BendlDecoder(path)
    # asset_size is the directory's stored length, for every entry.
    for entry in dec.list_assets():
        assert dec.asset_size(entry["name"]) == entry["len"]

    # Sub-threshold assets are stored raw: stored size == decoded size.
    assert dec.asset_size("small.txt") == len(dec.read_asset_bytes("small.txt")) == 4
    # Compressed assets: stored size is the xz size, smaller than the decoded payload.
    flags = {a["name"]: a["flags"] for a in dec.list_assets()}
    assert "xz" in flags["big.txt"]
    assert dec.asset_size("big.txt") < len(dec.read_asset_bytes("big.txt")) == 5000

    with pytest.raises(KeyError, match="no asset named"):
        dec.asset_size("missing.bin")


# ---------------------------------------------------------------------------
# Asset removal
# ---------------------------------------------------------------------------


def test_remove_asset_drops_entry_and_preserves_everything_else(tmp_path: Path) -> None:
    n = _n()
    samples = [[(i + j) % 4 + 1 for j in range(n)] for i in range(4)]
    path = tmp_path / "rm.bendl"
    enc = BendlEncoder(path, overwrite=True)
    enc.add_graph(_graph(), sort=None)
    with enc.stream() as s:
        for a in samples:
            s.write(a)
    enc.add_asset("notes.txt", "scratch notes", content_type="text")
    enc.add_asset("keep.json", {"keep": True}, content_type="json")

    appender = BendlEncoder.append(path)
    appender.remove_asset("notes.txt")

    dec = BendlDecoder(path)
    assert "notes.txt" not in dec.asset_names()
    with pytest.raises(KeyError, match="no asset named"):
        dec.read_asset_bytes("notes.txt")
    # Everything else is untouched: assets, stream, and every remaining checksum.
    assert dec.read_json_asset("keep.json") == {"keep": True}
    assert list(dec) == samples
    dec.verify()


def test_remove_then_add_replaces_an_asset(tmp_path: Path) -> None:
    path = tmp_path / "update.bendl"
    with BendlEncoder(path, overwrite=True) as enc:
        enc.add_metadata({"seed": 1})

    appender = BendlEncoder.append(path)
    # metadata.json is a singleton, so a bare re-add is refused...
    with pytest.raises(Exception, match="duplicate"):
        appender.add_metadata({"seed": 2})
    # ...but remove-then-add is the update idiom.
    appender.remove_asset("metadata.json")
    appender.add_metadata({"seed": 2})
    assert BendlDecoder(path).read_metadata() == {"seed": 2}


def test_remove_asset_reclaims_bytes_automatically(tmp_path: Path) -> None:
    path = tmp_path / "reclaim.bendl"
    enc = BendlEncoder(path, overwrite=True)
    with enc.stream() as s:
        s.write([1, 2, 3])
    import random

    blob = random.Random(0).randbytes(64 * 1024)  # incompressible, so it really occupies bytes
    enc.add_asset("bloat.bin", blob, content_type="binary")
    bloated = path.stat().st_size
    assert bloated > 64 * 1024

    # The facade's removal compacts in place: the payload bytes are really gone,
    # not just unreferenced.
    enc.remove_asset("bloat.bin")
    assert path.stat().st_size < bloated - 60_000
    dec = BendlDecoder(path)
    assert dec.asset_names() == []
    assert list(dec) == [[1, 2, 3]]
    dec.verify()


def test_remove_asset_guards(tmp_path: Path) -> None:
    path = tmp_path / "guards.bendl"
    enc = BendlEncoder(path, overwrite=True)
    enc.add_asset("a.txt", "a", content_type="text")
    # Pre-stream create mode: nothing is committed yet, so there is nothing to remove.
    with pytest.raises(Exception, match="not finalized"):
        enc.remove_asset("a.txt")
    with enc.stream() as s:
        s.write([1, 2])
    # Post-stream (finalized) the same encoder can remove, and unknown names are KeyErrors.
    with pytest.raises(KeyError, match="no asset named"):
        enc.remove_asset("missing.txt")
    enc.remove_asset("a.txt")
    assert BendlDecoder(path).asset_names() == []


@pytest.mark.skipif(os.name != "posix", reason="needs RLIMIT_FSIZE / SIGXFSZ")
def test_failed_stream_finalize_poisons_the_encoder(tmp_path: Path) -> None:
    """A finalize that fails (EFBIG from RLIMIT_FSIZE while flushing the stream tail and
    directory) must poison the encoder. The failure used to leave the state machine stuck in
    'streaming': a retried close() silently returned success on an unfinalized bundle, and
    add_asset advised 'close it before adding assets' — advice that did nothing."""
    import resource
    import signal

    path = tmp_path / "limited.bendl"
    enc = BendlEncoder(path, overwrite=True)
    s = enc.stream(variant="standard")
    for _ in range(64):
        s.write([1, 2, 3, 4] * 64)

    old_limit = resource.getrlimit(resource.RLIMIT_FSIZE)
    old_handler = signal.signal(signal.SIGXFSZ, signal.SIG_IGN)
    try:
        # Cap the file just above its current size: the buffered stream tail and the directory
        # that close() flushes cannot fit, so the finalize fails mid-write.
        resource.setrlimit(resource.RLIMIT_FSIZE, (path.stat().st_size + 1, old_limit[1]))
        with pytest.raises(OSError):
            s.close()
    finally:
        resource.setrlimit(resource.RLIMIT_FSIZE, old_limit)
        signal.signal(signal.SIGXFSZ, old_handler)

    # A retried close keeps reporting the failure instead of claiming success.
    with pytest.raises(Exception, match="previous stream failed"):
        s.close()
    # Encoder-level calls explain the failure accurately.
    with pytest.raises(Exception, match="previous stream failed"):
        enc.add_asset("notes.txt", "x", content_type="text")
    with pytest.raises(Exception, match="previous stream failed"):
        enc.stream()
    # And the bundle on disk is, truthfully, unfinalized.
    assert not BendlDecoder(path).is_complete()


def test_decoder_refuses_file_replaced_under_it(tmp_path: Path) -> None:
    """A decoder is a snapshot: after an in-place transform swaps a rewritten file over the
    path, every data read must refuse. The old behavior was split-brain — asset reads and
    verify() served the OLD file through the held handle (verify passed, read_graph returned
    the just-replaced graph) while iteration reopened the NEW file at stale offsets — which
    could silently pair the old graph with relabeled assignments."""
    from binary_ensemble.bundle import relabel_bundle

    path = tmp_path / "replaced.bendl"
    enc = BendlEncoder(path, overwrite=True)
    enc.add_graph(_graph(), sort=None)
    with enc.stream() as s:
        s.write([1] * _n())
        s.write([2] * _n())

    dec = BendlDecoder(path)
    assert dec.read_graph() is not None  # the snapshot works while the file is unchanged
    relabel_bundle(path)  # in place: a rewritten file is swapped over the path

    with pytest.raises(Exception, match="changed on disk"):
        dec.read_graph()
    with pytest.raises(Exception, match="changed on disk"):
        dec.verify()
    with pytest.raises(Exception, match="changed on disk"):
        list(dec)
    with pytest.raises(Exception, match="changed on disk"):
        dec.extract_stream(tmp_path / "out.ben")

    # A fresh decoder reads the current file fine.
    fresh = BendlDecoder(path)
    fresh.verify()
    assert len(list(fresh)) == 2


def test_decoder_refuses_after_append(tmp_path: Path) -> None:
    """Appends rewrite the directory in place; a decoder opened before one holds a stale
    directory (it would not even list the new asset), so its data reads refuse too."""
    path = tmp_path / "appended.bendl"
    enc = BendlEncoder(path, overwrite=True)
    with enc.stream() as s:
        s.write([1, 2, 3])
    enc.add_asset("a.txt", "alpha", content_type="text")

    dec = BendlDecoder(path)
    assert dec.read_asset_bytes("a.txt") == b"alpha"

    appender = BendlEncoder.append(path)
    appender.add_asset("b.txt", "beta", content_type="text")

    with pytest.raises(Exception, match="changed on disk"):
        dec.read_asset_bytes("a.txt")
    with pytest.raises(Exception, match="changed on disk"):
        list(dec)
    assert BendlDecoder(path).read_asset_bytes("b.txt") == b"beta"


def test_generic_add_asset_refuses_canonical_names(tmp_path: Path) -> None:
    """A custom asset stored under a standardized name would be invisible to the type-keyed
    readers (read_metadata() returned None while asset_names() listed 'metadata.json' and
    verify() passed) — the silent failure mode of doing the replace flow through the generic
    add_asset. The writer now refuses with guidance, and the typed re-add works."""
    path = tmp_path / "reserved.bendl"
    with BendlEncoder(path, overwrite=True) as enc:
        enc.add_metadata({"seed": 1})

    appender = BendlEncoder.append(path)
    appender.remove_asset("metadata.json")
    # The footgun: generic add under the canonical name is refused, pointing at the typed add.
    with pytest.raises(Exception, match="reserved.*add_metadata"):
        appender.add_asset("metadata.json", {"seed": 2}, content_type="json")
    # The refusal reserved nothing; the typed replace works and the reader finds it by type.
    appender.add_metadata({"seed": 2})
    assert BendlDecoder(path).read_metadata() == {"seed": 2}

    # Same protection on the create path, for all three canonical names.
    enc = BendlEncoder(tmp_path / "fresh.bendl", overwrite=True)
    for name in ("metadata.json", "graph.json", "node_permutation_map.json"):
        with pytest.raises(Exception, match="reserved"):
            enc.add_asset(name, b"{}", content_type="json")
