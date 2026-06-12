"""Pipeline, plain-stream encoder/decoder, and subsampling tests.

Covers the whole-file ``codec`` transforms and the stream-only ``BenEncoder`` /
``BenDecoder``. Bundle (``.bendl``) behavior lives in ``test_bundle.py`` and
``test_bundle_api.py``.
"""

import json
import random
from pathlib import Path
from typing import Iterable, List

import pytest

from binary_ensemble import (
    BenDecoder,
    BenEncoder,
    decode_ben_to_jsonl,
    decode_xben_to_ben,
    decode_xben_to_jsonl,
    encode_ben_to_xben,
    encode_jsonl_to_ben,
    encode_jsonl_to_xben,
)

# ---------- Helpers ----------


def expand_rle(rle: Iterable[tuple[int, int]], cap: int) -> list[int]:
    out: List[int] = []
    for val, length in rle:
        take = min(length, max(0, cap - len(out)))
        if take <= 0:
            break
        out.extend([val] * take)
    return out


def gen_assignment(
    rng: random.Random, max_val: int, max_run: int, max_len: int
) -> list[int]:
    rle = []
    n_runs = rng.randint(10, 50)
    for _ in range(n_runs):
        val = rng.randint(1, max_val)
        length = rng.randint(1, max_run)
        rle.append((val, length))
    v = expand_rle(rle, max_len)
    return v or [1]


def gen_sequence_standard(
    rng: random.Random, n_samples: int, *, max_val=50, max_run=300, max_len=1500
) -> list[list[int]]:
    return [gen_assignment(rng, max_val, max_run, max_len) for _ in range(n_samples)]


def gen_sequence_mkv(
    rng: random.Random, n_samples: int, *, max_val=50, max_run=300, max_len=1500
) -> list[list[int]]:
    seq: list[list[int]] = []
    while len(seq) < n_samples:
        base = gen_assignment(rng, max_val, max_run, max_len)
        reps = min(rng.randint(1, 10), n_samples - len(seq))
        seq.extend([base] * reps)
    return seq


def write_jsonl(samples: list[list[int]], path: Path) -> None:
    with path.open("w", encoding="utf-8") as f:
        for i, a in enumerate(samples, start=1):
            json.dump({"assignment": a, "sample": i}, f, separators=(",", ":"))
            f.write("\n")


def read_jsonl_assignments(path: Path) -> list[list[int]]:
    out: list[list[int]] = []
    with path.open("r", encoding="utf-8") as f:
        for line in f:
            if not line.strip():
                continue
            obj = json.loads(line)
            out.append(list(map(int, obj["assignment"])))
    return out


# ---------- Codec pipelines ----------


def test_ben_pipeline(tmp_path: Path) -> None:
    rng = random.Random(129530786)
    seq = gen_sequence_standard(rng, 100)
    src = tmp_path / "src.jsonl"
    write_jsonl(seq, src)
    ben = tmp_path / "out.ben"
    out_jsonl = tmp_path / "round.jsonl"
    encode_jsonl_to_ben(src, ben, overwrite=True, variant="standard")
    decode_ben_to_jsonl(ben, out_jsonl, overwrite=True)
    assert src.read_bytes() == out_jsonl.read_bytes()


def test_mkvben_pipeline(tmp_path: Path) -> None:
    rng = random.Random(129530786)
    seq = gen_sequence_mkv(rng, 100)
    src = tmp_path / "src.jsonl"
    write_jsonl(seq, src)
    ben = tmp_path / "out_mkv.ben"
    out_jsonl = tmp_path / "round_mkv.jsonl"
    encode_jsonl_to_ben(src, ben, overwrite=True, variant="mkv_chain")
    decode_ben_to_jsonl(ben, out_jsonl, overwrite=True)
    assert src.read_bytes() == out_jsonl.read_bytes()


def test_xben_pipeline(tmp_path: Path) -> None:
    rng = random.Random(129530786)
    seq = gen_sequence_standard(rng, 50)
    src = tmp_path / "src.jsonl"
    write_jsonl(seq, src)
    xben = tmp_path / "out.xben"
    ben = tmp_path / "out.ben"
    round_jsonl = tmp_path / "round.jsonl"
    encode_jsonl_to_xben(
        src, xben, overwrite=True, variant="standard", n_threads=1, compression_level=1
    )
    decode_xben_to_ben(xben, ben, overwrite=True)
    decode_ben_to_jsonl(ben, round_jsonl, overwrite=True)
    assert src.read_bytes() == round_jsonl.read_bytes()


def test_ben_to_xben_and_back(tmp_path: Path) -> None:
    rng = random.Random(314159)
    seq = gen_sequence_mkv(rng, 80)
    src = tmp_path / "src.jsonl"
    write_jsonl(seq, src)
    ben = tmp_path / "in.ben"
    xben = tmp_path / "roundtrip.xben"
    ben2 = tmp_path / "out.ben"
    out_jsonl = tmp_path / "out.jsonl"
    encode_jsonl_to_ben(src, ben, overwrite=True, variant="mkv_chain")
    encode_ben_to_xben(ben, xben, overwrite=True, n_threads=1, compression_level=1)
    decode_xben_to_ben(xben, ben2, overwrite=True)
    decode_ben_to_jsonl(ben2, out_jsonl, overwrite=True)
    assert src.read_bytes() == out_jsonl.read_bytes()


# ---------- Iterator parity ----------


def test_decoder_iterator_matches_jsonl_ben(tmp_path: Path) -> None:
    rng = random.Random(129530786)
    seq = gen_sequence_standard(rng, 120)
    src = tmp_path / "src.jsonl"
    write_jsonl(seq, src)
    ben = tmp_path / "out.ben"
    encode_jsonl_to_ben(src, ben, overwrite=True, variant="standard")
    baseline = read_jsonl_assignments(src)
    assert list(BenDecoder(ben, mode="ben")) == baseline


def test_decoder_iterator_matches_jsonl_xben(tmp_path: Path) -> None:
    rng = random.Random(129530786)
    seq = gen_sequence_mkv(rng, 120)
    src = tmp_path / "src.jsonl"
    write_jsonl(seq, src)
    xben = tmp_path / "out.xben"
    encode_jsonl_to_xben(
        src, xben, overwrite=True, variant="mkv_chain", n_threads=1, compression_level=1
    )
    roundtrip = tmp_path / "direct.jsonl"
    decode_xben_to_jsonl(xben, roundtrip, overwrite=True)
    baseline = read_jsonl_assignments(roundtrip)
    assert list(BenDecoder(xben, mode="xben")) == baseline


# ---------- Subsampling on plain streams ----------


def test_subsample_indices(tmp_path: Path) -> None:
    rng = random.Random(2_022_11_11)
    seq = gen_sequence_mkv(rng, 200)
    src = tmp_path / "src.jsonl"
    write_jsonl(seq, src)
    xben = tmp_path / "out.xben"
    encode_jsonl_to_xben(
        src, xben, overwrite=True, variant="mkv_chain", n_threads=1, compression_level=1
    )
    want = list(range(1, 201, 3))
    baseline = [seq[i - 1] for i in want]
    assert list(BenDecoder(xben, mode="xben").subsample_indices(want)) == baseline


def test_subsample_range(tmp_path: Path) -> None:
    rng = random.Random(42)
    seq = gen_sequence_mkv(rng, 150)
    src = tmp_path / "src.jsonl"
    write_jsonl(seq, src)
    ben = tmp_path / "out.ben"
    encode_jsonl_to_ben(src, ben, overwrite=True, variant="mkv_chain")
    assert list(BenDecoder(ben, mode="ben").subsample_range(11, 77)) == seq[10:77]


def test_subsample_every(tmp_path: Path) -> None:
    rng = random.Random(1337)
    seq = gen_sequence_mkv(rng, 180)
    src = tmp_path / "src.jsonl"
    write_jsonl(seq, src)
    xben = tmp_path / "out.xben"
    encode_jsonl_to_xben(
        src, xben, overwrite=True, variant="mkv_chain", n_threads=1, compression_level=1
    )
    baseline = [seq[i - 1] for i in range(2, 181, 5)]
    assert list(BenDecoder(xben, mode="xben").subsample_every(5, 2)) == baseline


def test_plain_stream_iteration_restart(tmp_path: Path) -> None:
    samples = [[1, 2, 3], [4, 5, 6], [7, 8, 9]]
    path = tmp_path / "twice.ben"
    with BenEncoder(path, overwrite=True, variant="standard") as enc:
        for a in samples:
            enc.write(a)
    dec = BenDecoder(path)
    assert list(dec) == samples
    assert list(dec) == samples


def test_plain_stream_subsample_survives_reiteration(tmp_path: Path) -> None:
    samples = [[i] for i in range(1, 8)]
    path = tmp_path / "re.ben"
    with BenEncoder(path, overwrite=True, variant="standard") as enc:
        for a in samples:
            enc.write(a)
    dec = BenDecoder(path).subsample_every(2, offset=1)
    expected = [[1], [3], [5], [7]]
    assert list(dec) == expected
    assert list(dec) == expected


# ---------- Encoder surface ----------


def test_benencoder_roundtrip(tmp_path: Path) -> None:
    rng = random.Random(777)
    seq = gen_sequence_standard(rng, 60)
    ben = tmp_path / "out.ben"
    with BenEncoder(ben, overwrite=True, variant="standard") as enc:
        for a in seq:
            enc.write(a)
    assert list(BenDecoder(ben, mode="ben")) == seq


def test_benencoder_default_and_markov_alias(tmp_path: Path) -> None:
    samples = [[1, 1, 2], [1, 1, 2], [2, 3, 3]]
    default_ben = tmp_path / "default.ben"
    with BenEncoder(default_ben, overwrite=True) as enc:
        for s in samples:
            enc.write(s)
    assert list(BenDecoder(default_ben, mode="ben")) == samples

    src = tmp_path / "src.jsonl"
    write_jsonl(samples, src)
    alias_ben = tmp_path / "alias.ben"
    encode_jsonl_to_ben(src, alias_ben, overwrite=True, variant="markov")
    assert list(BenDecoder(alias_ben, mode="ben")) == samples


def test_benencoder_produces_plain_stream_not_bundle(tmp_path: Path) -> None:
    # BenEncoder must never emit BENDL framing.
    out = tmp_path / "plain.ben"
    with BenEncoder(out, overwrite=True, variant="standard") as enc:
        enc.write([1, 2, 3])
    assert not out.read_bytes().startswith(b"BENDL")
    assert list(BenDecoder(out, mode="ben")) == [[1, 2, 3]]


def test_benencoder_close_and_write_error_paths(tmp_path: Path) -> None:
    out = tmp_path / "out.ben"
    enc = BenEncoder(out, overwrite=True, variant="standard")
    enc.write([1, 2, 3])
    enc.close()
    enc.close()  # idempotent
    with pytest.raises(OSError, match="already been closed"):
        enc.write([1, 2, 3])

    invalid_path = tmp_path / "invalid.ben"
    with BenEncoder(invalid_path, overwrite=True, variant="standard") as inv:
        with pytest.raises(Exception):
            inv.write([-1])
        with pytest.raises(Exception):
            inv.write([65536])


def test_benencoder_rejects_overwrite_and_unknown_variant(tmp_path: Path) -> None:
    out = tmp_path / "out.ben"
    out.write_bytes(b"existing")
    with pytest.raises(ValueError, match="Unknown variant"):
        BenEncoder(tmp_path / "bad.ben", overwrite=False, variant="weird")
    with pytest.raises(OSError, match="already exists"):
        BenEncoder(out, overwrite=False, variant="standard")
    with pytest.raises(OSError, match="Failed to create"):
        BenEncoder(
            tmp_path / "missing-dir" / "out.ben", overwrite=False, variant="standard"
        )


# ---------- Decoder error / laziness paths ----------


def test_decoder_constructor_and_mode_errors(tmp_path: Path) -> None:
    with pytest.raises(Exception, match="Unknown mode"):
        BenDecoder(tmp_path / "missing.ben", mode="weird")
    with pytest.raises(OSError, match="Failed to open"):
        BenDecoder(tmp_path / "missing.ben", mode="ben")
    bad_ben = tmp_path / "bad.ben"
    bad_ben.write_bytes(b"garbage")
    with pytest.raises(Exception, match="Failed to create BenDecoder"):
        BenDecoder(bad_ben, mode="ben")
    bad_xben = tmp_path / "bad.xben"
    bad_xben.write_bytes(b"garbage")
    with pytest.warns(UserWarning, match="XBEN may take a second"):
        with pytest.raises(Exception, match="Failed to create XBenDecoder"):
            BenDecoder(bad_xben, mode="xben")


def test_decoder_len_and_count_are_lazy_and_cached(tmp_path: Path) -> None:
    samples = [[1, 1, 2], [1, 1, 2], [2, 3, 3], [4]]
    src = tmp_path / "src.jsonl"
    write_jsonl(samples, src)
    ben = tmp_path / "out.ben"
    encode_jsonl_to_ben(src, ben, overwrite=True, variant="mkv_chain")
    dec = BenDecoder(ben, mode="ben")
    assert len(dec) == len(samples)
    assert dec.count_samples() == len(samples)
    assert list(dec) == samples


def test_decoder_count_after_subsample_preserves_len(tmp_path: Path) -> None:
    samples = [[i] for i in range(1, 11)]
    path = tmp_path / "plain.ben"
    with BenEncoder(path, overwrite=True, variant="standard") as enc:
        for a in samples:
            enc.write(a)
    dec = BenDecoder(path).subsample_every(3, 1)
    expected = samples[::3]
    assert len(dec) == len(expected)
    assert dec.count_samples() == len(samples)
    assert len(dec) == len(expected)
    assert list(dec) == expected


def test_decoder_subsample_validations(tmp_path: Path) -> None:
    samples = [[1], [2], [3], [4], [5]]
    src = tmp_path / "src.jsonl"
    write_jsonl(samples, src)
    ben = tmp_path / "out.ben"
    encode_jsonl_to_ben(src, ben, overwrite=True, variant="standard")

    with pytest.warns(UserWarning, match="sorted and unique"):
        got = list(BenDecoder(ben, mode="ben").subsample_indices([5, 1, 1, 3]))
    assert got == [samples[0], samples[2], samples[4]]
    with pytest.raises(Exception, match="indices must be 1-based"):
        BenDecoder(ben, mode="ben").subsample_indices([0, 1])
    with pytest.raises(Exception, match="indices must not be empty"):
        BenDecoder(ben, mode="ben").subsample_indices([])
    with pytest.raises(Exception, match="indices must be <="):
        BenDecoder(ben, mode="ben").subsample_indices([6])
    with pytest.raises(Exception, match="range must be 1-based"):
        BenDecoder(ben, mode="ben").subsample_range(0, 2)
    with pytest.raises(Exception, match="end must be <="):
        BenDecoder(ben, mode="ben").subsample_range(1, 99)
    with pytest.raises(Exception, match="step and offset must be >= 1"):
        BenDecoder(ben, mode="ben").subsample_every(0, 1)
    with pytest.raises(Exception, match="offset must be <="):
        BenDecoder(ben, mode="ben").subsample_every(2, 99)


def test_decoder_reports_zero_count_and_bad_frame_errors(tmp_path: Path) -> None:
    src = tmp_path / "src.jsonl"
    write_jsonl([[1, 1, 2]], src)
    mkv_ben = tmp_path / "mkv.ben"
    encode_jsonl_to_ben(src, mkv_ben, overwrite=True, variant="mkv_chain")
    data = bytearray(mkv_ben.read_bytes())
    data[-2:] = b"\x00\x00"
    mkv_ben.write_bytes(data)
    with pytest.raises(Exception, match="count must be greater than zero"):
        next(iter(BenDecoder(mkv_ben, mode="ben")))

    standard_ben = tmp_path / "standard.ben"
    encode_jsonl_to_ben(src, standard_ben, overwrite=True, variant="standard")
    truncated = standard_ben.read_bytes()[:-1]
    bad_ben = tmp_path / "truncated.ben"
    bad_ben.write_bytes(truncated)
    with pytest.raises(Exception, match="Error decoding next item"):
        next(iter(BenDecoder(bad_ben, mode="ben")))


# ---------- Codec error paths ----------


def test_codec_helpers_reject_unknown_variants(tmp_path: Path) -> None:
    src = tmp_path / "src.jsonl"
    write_jsonl([[1, 1, 2]], src)
    with pytest.raises(ValueError, match="Unknown variant"):
        encode_jsonl_to_ben(src, tmp_path / "o.ben", overwrite=True, variant="weird")
    with pytest.raises(ValueError, match="Unknown variant"):
        encode_jsonl_to_xben(src, tmp_path / "o.xben", overwrite=True, variant="weird")


def test_codec_helpers_reject_same_path_missing_input_and_bad_json(
    tmp_path: Path,
) -> None:
    src = tmp_path / "src.jsonl"
    write_jsonl([[1, 1, 2]], src)
    with pytest.raises(OSError, match="must differ"):
        encode_jsonl_to_ben(src, src, overwrite=True, variant="standard")
    with pytest.raises(OSError, match="does not exist"):
        encode_jsonl_to_ben(
            tmp_path / "missing.jsonl", tmp_path / "o.ben", overwrite=True
        )
    bad_json = tmp_path / "bad.jsonl"
    bad_json.write_text("not json\n", encoding="utf-8")
    with pytest.raises(OSError, match="Failed to convert JSONL to BEN"):
        encode_jsonl_to_ben(bad_json, tmp_path / "bad.ben", overwrite=True)


def test_encode_ben_to_xben_error_paths(tmp_path: Path) -> None:
    with pytest.raises(OSError, match="does not exist"):
        encode_ben_to_xben(
            tmp_path / "missing.ben", tmp_path / "o.xben", overwrite=True
        )
    bad_ben = tmp_path / "bad.ben"
    bad_ben.write_bytes(b"garbage")
    with pytest.raises(OSError, match="must differ"):
        encode_ben_to_xben(bad_ben, bad_ben, overwrite=True)
    with pytest.raises(OSError, match="Failed to convert BEN to XBEN"):
        encode_ben_to_xben(bad_ben, tmp_path / "o.xben", overwrite=True)


def test_decode_helpers_error_paths(tmp_path: Path) -> None:
    with pytest.raises(OSError, match="does not exist"):
        decode_ben_to_jsonl(
            tmp_path / "missing.ben", tmp_path / "o.jsonl", overwrite=True
        )
    bad_ben = tmp_path / "bad.ben"
    bad_ben.write_bytes(b"garbage")
    with pytest.raises(OSError, match="Failed to convert BEN to JSONL"):
        decode_ben_to_jsonl(bad_ben, tmp_path / "o.jsonl", overwrite=True)
    bad_xben = tmp_path / "bad.xben"
    bad_xben.write_bytes(b"garbage")
    with pytest.raises(OSError, match="Failed to convert XBEN to BEN"):
        decode_xben_to_ben(bad_xben, tmp_path / "o.ben", overwrite=True)
    with pytest.raises(OSError, match="must differ"):
        decode_xben_to_jsonl(bad_xben, bad_xben, overwrite=True)
    src = tmp_path / "src.jsonl"
    write_jsonl([[1, 2, 3]], src)
    ben = tmp_path / "good.ben"
    encode_jsonl_to_ben(src, ben, overwrite=True, variant="standard")
    out = tmp_path / "exists.jsonl"
    out.write_text("exists\n", encoding="utf-8")
    with pytest.raises(OSError, match="already exists"):
        decode_ben_to_jsonl(ben, out, overwrite=False)


def test_decoder_surfaces_truncated_streams_as_clean_exceptions(tmp_path: Path) -> None:
    # The Rust core guarantees corrupt input errors rather than panics; this pins the Python
    # half of that contract — a truncated stream raises an ordinary exception from iteration.
    samples = [[1, 1, 2, 2], [2, 2, 1, 1], [1, 2, 1, 2]]

    ben = tmp_path / "trunc.ben"
    with BenEncoder(ben, overwrite=True, variant="standard") as enc:
        for a in samples:
            enc.write(a)
    ben.write_bytes(ben.read_bytes()[:-3])
    with pytest.raises(Exception, match="."):
        list(BenDecoder(ben, mode="ben"))

    xben = tmp_path / "trunc.xben"
    src = tmp_path / "src.jsonl"
    write_jsonl(samples, src)
    encode_jsonl_to_xben(src, xben, overwrite=True, variant="standard")
    xben.write_bytes(xben.read_bytes()[:-3])
    with pytest.raises(Exception, match="."):
        list(BenDecoder(xben, mode="xben"))
