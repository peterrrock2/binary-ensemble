"""Lifecycle tests for the ``BendlEncoder`` authoring facade.

Covers create vs append mode, the single-use stream session, asset/graph/metadata
adds before and after the stream, content-type validation, graph↔chain node-count
validation, the assets-only (stream-less) bundle, and the unfinalized-on-exception
recovery path.
"""

from __future__ import annotations

import json
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
        with enc.stream("ben") as stream:
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
    with enc.stream("ben") as s:
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
    with enc.stream("ben") as s:
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
            with enc.stream("ben") as s:
                s.write([1, 2, 3])
                raise RuntimeError("boom")
    dec = BendlDecoder(path)
    assert dec.is_complete() is False
    # Verified extraction refuses an unfinalized stream...
    with pytest.raises(Exception, match="unfinalized"):
        dec.extract_stream(tmp_path / "recovered.ben")
    # ...but the partial write is recoverable.
    dec.extract_stream(
        tmp_path / "recovered.ben", overwrite=True, allow_unfinalized=True
    )
    assert (tmp_path / "recovered.ben").stat().st_size > 0


# ---------------------------------------------------------------------------
# content_type validation
# ---------------------------------------------------------------------------


def test_add_asset_content_type_validation(tmp_path: Path) -> None:
    enc = BendlEncoder(tmp_path / "v.bendl", overwrite=True)
    with pytest.raises(ValueError, match="must be 'json' or 'text'"):
        enc.add_asset("x", b"data", content_type="binary")
    with pytest.raises(ValueError, match="valid UTF-8 JSON"):
        enc.add_asset("bad.json", "not json", content_type="json")
    with pytest.raises(ValueError, match="valid UTF-8"):
        enc.add_asset("bad.txt", b"\xff\xfe", content_type="text")
    # Valid forms succeed.
    enc.add_asset("ok.json", '{"a":1}', content_type="json")
    enc.add_asset("ok.txt", "fine", content_type="text")
    enc.close()
    dec = BendlDecoder(tmp_path / "v.bendl")
    assert dec.read_json_asset("ok.json") == {"a": 1}
    flags = {a["name"]: a["flags"] for a in dec.list_assets()}
    assert "json" in flags["ok.json"]
    assert "json" not in flags["ok.txt"]


# ---------------------------------------------------------------------------
# add_graph reorder / raw / validation
# ---------------------------------------------------------------------------


def test_add_graph_reorder_emits_graph_and_permutation_map(tmp_path: Path) -> None:
    n = _n()
    path = tmp_path / "reord.bendl"
    enc = BendlEncoder(path, overwrite=True)
    reordered = enc.add_graph(_graph(), sort="rcm")
    with enc.stream("ben") as s:
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
    with enc.stream("ben") as s:
        s.write([1] * n)  # correct
        with pytest.raises(ValueError, match="does not match graph node count"):
            s.write([1] * (n - 1))


def test_reorder_add_graph_after_stream_raises_but_raw_succeeds(tmp_path: Path) -> None:
    n = _n()
    path = tmp_path / "after.bendl"
    enc = BendlEncoder(path, overwrite=True)
    with enc.stream("ben") as s:
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
# Stream-format and second-stream guards
# ---------------------------------------------------------------------------


def test_stream_rejects_non_ben_format(tmp_path: Path) -> None:
    enc = BendlEncoder(tmp_path / "fmt.bendl", overwrite=True)
    with pytest.raises(ValueError, match="must be 'ben'"):
        enc.stream("xben")


def test_second_stream_refused(tmp_path: Path) -> None:
    path = tmp_path / "two.bendl"
    enc = BendlEncoder(path, overwrite=True)
    with enc.stream("ben") as s:
        s.write([1, 2])
    with pytest.raises(Exception, match="already been written"):
        enc.stream("ben")


# ---------------------------------------------------------------------------
# Append mode
# ---------------------------------------------------------------------------


def test_append_mode_adds_assets(tmp_path: Path) -> None:
    path = tmp_path / "app.bendl"
    with BendlEncoder(path, overwrite=True) as enc:
        with enc.stream("ben") as s:
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
        with enc.stream("ben") as s:
            s.write([1])
    ap = BendlEncoder.append(path)
    with pytest.raises(Exception, match="append mode"):
        ap.stream("ben")


def test_append_mode_reorder_graph_raises(tmp_path: Path) -> None:
    path = tmp_path / "app3.bendl"
    with BendlEncoder(path, overwrite=True) as enc:
        with enc.stream("ben") as s:
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
            with enc.stream("ben") as s:
                s.write([1, 2])
                raise RuntimeError("stop")
    with pytest.raises(Exception):
        BendlEncoder.append(path)
