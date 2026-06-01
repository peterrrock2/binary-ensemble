import io
import json
import random
from pathlib import Path
from typing import Iterable, List

import pytest

import binary_ensemble
from binary_ensemble import (
    BenDecoder,
    BenEncoder,
    encode_ben_to_xben,
    encode_jsonl_to_ben,
    encode_jsonl_to_xben,
    decode_ben_to_jsonl,
    decode_xben_to_ben,
    decode_xben_to_jsonl,
)

# ---------- Helpers ----------


def expand_rle(rle: Iterable[tuple[int, int]], cap: int) -> list[int]:
    """Expand RLE pairs into a flat assignment vector, capped at cap."""
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
    """Generate one assignment by RLE with bounded length."""
    rle = []
    # Keep it small/fast: up to ~50 runs
    n_runs = rng.randint(10, 50)
    for _ in range(n_runs):
        val = rng.randint(1, max_val)
        length = rng.randint(1, max_run)
        rle.append((val, length))
    v = expand_rle(rle, max_len)
    # Ensure non-empty
    return v or [1]


def gen_sequence_standard(
    rng: random.Random, n_samples: int, *, max_val=50, max_run=300, max_len=1500
) -> list[list[int]]:
    return [gen_assignment(rng, max_val, max_run, max_len) for _ in range(n_samples)]


def gen_sequence_mkv(
    rng: random.Random, n_samples: int, *, max_val=50, max_run=300, max_len=1500
) -> list[list[int]]:
    """
    Like Rust test: inject duplicate exact assignments periodically to
    exercise MKV grouping. Ensures total length n_samples.
    """
    seq: list[list[int]] = []
    while len(seq) < n_samples:
        base = gen_assignment(rng, max_val, max_run, max_len)
        # repeat this assignment 1..10 times (but don’t exceed n_samples)
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


# ---------- Tests mirroring Rust ----------


def test_ben_pipeline(tmp_path: Path) -> None:
    rng = random.Random(129530786)
    n_samples = 100
    seq = gen_sequence_standard(rng, n_samples)

    src = tmp_path / "src.jsonl"
    write_jsonl(seq, src)

    ben = tmp_path / "out.ben"
    out_jsonl = tmp_path / "round.jsonl"

    encode_jsonl_to_ben(src, ben, overwrite=True, variant="standard")
    decode_ben_to_jsonl(ben, out_jsonl, overwrite=True)

    assert src.read_bytes() == out_jsonl.read_bytes()


def test_mkvben_pipeline(tmp_path: Path) -> None:
    rng = random.Random(129530786)
    n_samples = 100
    seq = gen_sequence_mkv(rng, n_samples)

    src = tmp_path / "src.jsonl"
    write_jsonl(seq, src)

    ben = tmp_path / "out_mkv.ben"
    out_jsonl = tmp_path / "round_mkv.jsonl"

    encode_jsonl_to_ben(src, ben, overwrite=True, variant="mkv_chain")
    decode_ben_to_jsonl(ben, out_jsonl, overwrite=True)

    assert src.read_bytes() == out_jsonl.read_bytes()


def test_xben_pipeline(tmp_path: Path) -> None:
    rng = random.Random(129530786)
    n_samples = 50
    seq = gen_sequence_standard(rng, n_samples)

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


def test_xmkvben_pipeline(tmp_path: Path) -> None:
    rng = random.Random(129530786)
    n_samples = 50
    seq = gen_sequence_mkv(rng, n_samples)

    src = tmp_path / "src.jsonl"
    write_jsonl(seq, src)

    xben = tmp_path / "out_mkv.xben"
    ben = tmp_path / "out_mkv.ben"
    round_jsonl = tmp_path / "round_mkv.jsonl"

    encode_jsonl_to_xben(
        src, xben, overwrite=True, variant="mkv_chain", n_threads=1, compression_level=1
    )
    decode_xben_to_ben(xben, ben, overwrite=True)
    decode_ben_to_jsonl(ben, round_jsonl, overwrite=True)

    assert src.read_bytes() == round_jsonl.read_bytes()


# ---------- Iterator/decoder parity with JSONL ----------


def test_decoder_iterator_matches_jsonl_ben(tmp_path: Path) -> None:
    rng = random.Random(129530786)
    n_samples = 120
    seq = gen_sequence_standard(rng, n_samples)

    src = tmp_path / "src.jsonl"
    write_jsonl(seq, src)

    ben = tmp_path / "out.ben"
    encode_jsonl_to_ben(src, ben, overwrite=True, variant="standard")

    # Baseline: assignments from JSONL
    baseline = read_jsonl_assignments(src)

    # BenDecoder over BEN
    got: list[list[int]] = []
    dec = BenDecoder(ben, mode="ben")
    for a in dec:
        got.append(a)

    assert got == baseline


def test_decoder_iterator_matches_jsonl_xben(tmp_path: Path) -> None:
    rng = random.Random(129530786)
    n_samples = 120
    seq = gen_sequence_mkv(rng, n_samples)

    src = tmp_path / "src.jsonl"
    write_jsonl(seq, src)

    xben = tmp_path / "out.xben"
    encode_jsonl_to_xben(
        src, xben, overwrite=True, variant="mkv_chain", n_threads=1, compression_level=1
    )

    # Baseline via full decompression
    roundtrip = tmp_path / "direct.jsonl"
    decode_xben_to_jsonl(xben, roundtrip, overwrite=True)
    baseline = read_jsonl_assignments(roundtrip)

    # Iterator directly over XBEN
    got: list[list[int]] = []
    dec = BenDecoder(xben, mode="xben")
    for a in dec:
        got.append(a)

    assert got == baseline


# ---------- Subsampling tests ----------


def test_subsample_indices(tmp_path: Path) -> None:
    rng = random.Random(2_022_11_11)
    n_samples = 200
    seq = gen_sequence_mkv(rng, n_samples)

    src = tmp_path / "src.jsonl"
    write_jsonl(seq, src)

    xben = tmp_path / "out.xben"
    encode_jsonl_to_xben(
        src, xben, overwrite=True, variant="mkv_chain", n_threads=1, compression_level=1
    )

    # choose indices: 1,4,7,…
    want = list(range(1, n_samples + 1, 3))
    baseline = [seq[i - 1] for i in want]

    got: list[list[int]] = []
    dec = BenDecoder(xben, mode="xben").subsample_indices(want)
    for a in dec:
        got.append(a)

    assert got == baseline


def test_subsample_range(tmp_path: Path) -> None:
    rng = random.Random(42)
    n_samples = 150
    seq = gen_sequence_mkv(rng, n_samples)

    src = tmp_path / "src.jsonl"
    write_jsonl(seq, src)

    ben = tmp_path / "out.ben"
    encode_jsonl_to_ben(src, ben, overwrite=True, variant="mkv_chain")

    start, end = 11, 77
    baseline = seq[start - 1 : end]

    got: list[list[int]] = []
    dec = BenDecoder(ben, mode="ben").subsample_range(start, end)
    for a in dec:
        got.append(a)

    assert got == baseline


def test_subsample_every(tmp_path: Path) -> None:
    rng = random.Random(1337)
    n_samples = 180
    seq = gen_sequence_mkv(rng, n_samples)

    src = tmp_path / "src.jsonl"
    write_jsonl(seq, src)

    xben = tmp_path / "out.xben"
    encode_jsonl_to_xben(
        src, xben, overwrite=True, variant="mkv_chain", n_threads=1, compression_level=1
    )

    step, offset = 5, 2  # keep 2,7,12,…
    baseline = [seq[i - 1] for i in range(offset, n_samples + 1, step)]

    got: list[list[int]] = []
    dec = BenDecoder(xben, mode="xben").subsample_every(step, offset)
    for a in dec:
        got.append(a)

    assert got == baseline


# ---------- Encoder surface (context manager & write) ----------


def test_benencoder_roundtrip(tmp_path: Path) -> None:
    rng = random.Random(777)
    n_samples = 60
    seq = gen_sequence_standard(rng, n_samples)

    ben = tmp_path / "out.ben"
    with BenEncoder(ben, overwrite=True, variant="standard", ben_file_only=True) as enc:
        for a in seq:
            enc.write(a)

    # Use decoder to read back
    got = list(BenDecoder(ben, mode="ben"))
    assert got == seq


# ---------- BEN -> XBEN convenience conversion ----------


def test_ben_to_xben_and_back(tmp_path: Path) -> None:
    rng = random.Random(314159)
    n_samples = 80
    seq = gen_sequence_mkv(rng, n_samples)

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


def test_decoder_subsample_indices_rejects_empty_input(tmp_path: Path) -> None:
    rng = random.Random(123)
    seq = gen_sequence_standard(rng, 10)

    src = tmp_path / "src.jsonl"
    write_jsonl(seq, src)

    ben = tmp_path / "out.ben"
    encode_jsonl_to_ben(src, ben, overwrite=True, variant="standard")

    dec = BenDecoder(ben, mode="ben")
    with pytest.raises(Exception, match="indices must not be empty"):
        dec.subsample_indices([])


def test_decoder_subsample_every_rejects_offset_past_end(tmp_path: Path) -> None:
    rng = random.Random(456)
    seq = gen_sequence_standard(rng, 10)

    src = tmp_path / "src.jsonl"
    write_jsonl(seq, src)

    ben = tmp_path / "out.ben"
    encode_jsonl_to_ben(src, ben, overwrite=True, variant="standard")

    dec = BenDecoder(ben, mode="ben")
    with pytest.raises(Exception, match="offset must be <="):
        dec.subsample_every(2, 99)


def test_compress_helpers_reject_unknown_variants(tmp_path: Path) -> None:
    rng = random.Random(789)
    seq = gen_sequence_standard(rng, 5)

    src = tmp_path / "src.jsonl"
    write_jsonl(seq, src)

    with pytest.raises(ValueError, match="Unknown variant"):
        encode_jsonl_to_ben(src, tmp_path / "out.ben", overwrite=True, variant="weird")

    with pytest.raises(ValueError, match="Unknown variant"):
        encode_jsonl_to_xben(
            src, tmp_path / "out.xben", overwrite=True, variant="weird"
        )


def test_module_exports_are_exposed() -> None:
    expected = {
        "BenDecoder",
        "BenEncoder",
        "encode_jsonl_to_ben",
        "encode_ben_to_xben",
        "encode_jsonl_to_xben",
        "decode_ben_to_jsonl",
        "decode_xben_to_jsonl",
        "decode_xben_to_ben",
    }
    assert expected.issubset(set(binary_ensemble.__all__))
    for name in expected:
        assert hasattr(binary_ensemble, name)
    assert hasattr(binary_ensemble, "_core")


def test_benencoder_defaults_and_markov_alias_work(tmp_path: Path) -> None:
    samples = [[1, 1, 2], [1, 1, 2], [2, 3, 3]]

    default_ben = tmp_path / "default.ben"
    with BenEncoder(default_ben, overwrite=True, ben_file_only=True) as enc:
        for sample in samples:
            enc.write(sample)
    assert list(BenDecoder(default_ben, mode="ben")) == samples

    src = tmp_path / "src.jsonl"
    write_jsonl(samples, src)

    alias_ben = tmp_path / "alias.ben"
    alias_xben = tmp_path / "alias.xben"
    encode_jsonl_to_ben(src, alias_ben, overwrite=True, variant="markov")
    encode_jsonl_to_xben(
        src,
        alias_xben,
        overwrite=True,
        variant="markov",
        n_threads=1,
        compression_level=1,
    )
    assert list(BenDecoder(alias_ben, mode="ben")) == samples
    assert list(BenDecoder(alias_xben, mode="xben")) == samples


def test_benencoder_close_and_write_error_paths(tmp_path: Path) -> None:
    out = tmp_path / "out.ben"
    enc = BenEncoder(out, overwrite=True, variant="standard", ben_file_only=True)
    enc.write([1, 2, 3])
    enc.close()
    enc.close()
    with pytest.raises(OSError, match="already been closed"):
        enc.write([1, 2, 3])

    ctx_path = tmp_path / "ctx.ben"
    with BenEncoder(
        ctx_path, overwrite=True, variant="standard", ben_file_only=True
    ) as ctx_enc:
        ctx_enc.write([4, 5, 6])
    assert list(BenDecoder(ctx_path, mode="ben")) == [[4, 5, 6]]

    invalid_path = tmp_path / "invalid_assignment.ben"
    with BenEncoder(
        invalid_path, overwrite=True, variant="standard", ben_file_only=True
    ) as invalid_enc:
        with pytest.raises(Exception):
            invalid_enc.write([-1])
        with pytest.raises(Exception):
            invalid_enc.write([65536])


def test_benencoder_rejects_overwrite_and_unknown_variant(tmp_path: Path) -> None:
    out = tmp_path / "out.ben"
    out.write_bytes(b"existing")

    with pytest.raises(ValueError, match="Unknown variant"):
        BenEncoder(tmp_path / "bad.ben", overwrite=False, variant="weird")

    with pytest.raises(OSError, match="already exists"):
        BenEncoder(out, overwrite=False, variant="standard")

    with pytest.raises(OSError, match="Failed to create"):
        BenEncoder(
            tmp_path / "missing-dir" / "out.ben",
            overwrite=False,
            variant="standard",
        )


def test_compress_helpers_reject_same_path_missing_input_and_bad_json(
    tmp_path: Path,
) -> None:
    src = tmp_path / "src.jsonl"
    write_jsonl([[1, 1, 2]], src)

    with pytest.raises(OSError, match="must differ"):
        encode_jsonl_to_ben(src, src, overwrite=True, variant="standard")

    with pytest.raises(OSError, match="does not exist"):
        encode_jsonl_to_ben(
            tmp_path / "missing.jsonl",
            tmp_path / "out.ben",
            overwrite=True,
            variant="standard",
        )

    bad_json = tmp_path / "bad.jsonl"
    bad_json.write_text("not json\n", encoding="utf-8")
    with pytest.raises(OSError, match="Failed to convert JSONL to BEN"):
        encode_jsonl_to_ben(
            bad_json,
            tmp_path / "bad.ben",
            overwrite=True,
            variant="standard",
        )

    bad_assign = tmp_path / "bad_assign.jsonl"
    bad_assign.write_text('{"assignment":"bad","sample":1}\n', encoding="utf-8")
    with pytest.raises(OSError, match="Failed to convert JSONL to XBEN"):
        encode_jsonl_to_xben(
            bad_assign,
            tmp_path / "bad.xben",
            overwrite=True,
            variant="standard",
            n_threads=1,
            compression_level=1,
        )

    with pytest.raises(OSError, match="Failed to create"):
        encode_jsonl_to_ben(
            src,
            tmp_path / "missing-dir" / "out.ben",
            overwrite=True,
            variant="standard",
        )


def test_encode_ben_to_xben_rejects_same_path_missing_input_invalid_header_and_existing_output(
    tmp_path: Path,
) -> None:
    with pytest.raises(OSError, match="does not exist"):
        encode_ben_to_xben(
            tmp_path / "missing.ben",
            tmp_path / "out.xben",
            overwrite=True,
            n_threads=1,
            compression_level=1,
        )

    bad_ben = tmp_path / "bad.ben"
    bad_ben.write_bytes(b"garbage")

    with pytest.raises(OSError, match="must differ"):
        encode_ben_to_xben(
            bad_ben,
            bad_ben,
            overwrite=True,
            n_threads=1,
            compression_level=1,
        )

    with pytest.raises(OSError, match="Failed to convert BEN to XBEN"):
        encode_ben_to_xben(
            bad_ben,
            tmp_path / "out.xben",
            overwrite=True,
            n_threads=1,
            compression_level=1,
        )

    src = tmp_path / "src.jsonl"
    write_jsonl([[1, 2, 3]], src)
    ben = tmp_path / "good.ben"
    encode_jsonl_to_ben(src, ben, overwrite=True, variant="standard")
    out = tmp_path / "exists.xben"
    out.write_bytes(b"exists")
    with pytest.raises(OSError, match="already exists"):
        encode_ben_to_xben(
            ben,
            out,
            overwrite=False,
            n_threads=1,
            compression_level=1,
        )


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


def test_decoder_len_and_count_samples_are_lazy_and_cached(tmp_path: Path) -> None:
    samples = [[1, 1, 2], [1, 1, 2], [2, 3, 3], [4]]
    src = tmp_path / "src.jsonl"
    write_jsonl(samples, src)

    ben = tmp_path / "out.ben"
    encode_jsonl_to_ben(src, ben, overwrite=True, variant="mkv_chain")

    dec = BenDecoder(ben, mode="ben")
    assert len(dec) == len(samples)
    assert dec.count_samples() == len(samples)
    assert list(dec) == samples

    gone = BenDecoder(ben, mode="ben")
    assert len(gone) == len(samples)
    ben.unlink()
    with pytest.raises(Exception, match="Failed to create frame iterator"):
        gone.subsample_range(1, 2)


def test_decoder_xben_len_count_and_warning(tmp_path: Path) -> None:
    samples = [[1, 1], [1, 1], [2, 2], [3, 3]]
    src = tmp_path / "src.jsonl"
    write_jsonl(samples, src)

    xben = tmp_path / "out.xben"
    encode_jsonl_to_xben(
        src, xben, overwrite=True, variant="mkv_chain", n_threads=1, compression_level=1
    )

    with pytest.warns(UserWarning, match="XBEN may take a second"):
        dec = BenDecoder(xben, mode="xben")
    assert len(dec) == len(samples)
    assert dec.count_samples() == len(samples)
    assert list(dec) == samples


def test_decoder_subsample_validations_and_warning_paths(tmp_path: Path) -> None:
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

    with pytest.raises(Exception):
        BenDecoder(ben, mode="ben").subsample_indices([-1])

    with pytest.raises(Exception, match="indices must be <="):
        BenDecoder(ben, mode="ben").subsample_indices([6])

    with pytest.raises(Exception, match="range must be 1-based"):
        BenDecoder(ben, mode="ben").subsample_range(0, 2)

    with pytest.raises(Exception):
        BenDecoder(ben, mode="ben").subsample_range(-1, 2)

    with pytest.raises(Exception, match="end must be <="):
        BenDecoder(ben, mode="ben").subsample_range(1, 99)

    with pytest.raises(Exception, match="step and offset must be >= 1"):
        BenDecoder(ben, mode="ben").subsample_every(0, 1)

    with pytest.raises(Exception, match="offset must be <="):
        BenDecoder(ben, mode="ben").subsample_every(2, 99)

    assert list(BenDecoder(ben, mode="ben").subsample_range(2, 4)) == samples[1:4]
    assert list(BenDecoder(ben, mode="ben").subsample_every(2, 2)) == samples[1::2]


def test_decoder_count_and_subsample_fail_cleanly_if_source_disappears(
    tmp_path: Path,
) -> None:
    src = tmp_path / "src.jsonl"
    write_jsonl([[1], [2], [3]], src)

    ben = tmp_path / "out.ben"
    encode_jsonl_to_ben(src, ben, overwrite=True, variant="standard")

    dec = BenDecoder(ben, mode="ben")
    ben.unlink()

    with pytest.raises(Exception, match="Failed to count samples"):
        dec.count_samples()


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
    dec = BenDecoder(bad_ben, mode="ben")
    with pytest.raises(Exception, match="Error decoding next item"):
        next(iter(dec))


def test_decode_helpers_reject_same_paths_missing_inputs_existing_output_and_invalid_headers(
    tmp_path: Path,
) -> None:
    with pytest.raises(OSError, match="does not exist"):
        decode_ben_to_jsonl(
            tmp_path / "missing.ben",
            tmp_path / "out.jsonl",
            overwrite=True,
        )

    bad_ben = tmp_path / "bad.ben"
    bad_ben.write_bytes(b"garbage")
    with pytest.raises(OSError, match="Failed to convert BEN to JSONL"):
        decode_ben_to_jsonl(
            bad_ben,
            tmp_path / "out.jsonl",
            overwrite=True,
        )

    bad_xben = tmp_path / "bad.xben"
    bad_xben.write_bytes(b"garbage")
    with pytest.raises(OSError, match="Failed to convert XBEN to BEN"):
        decode_xben_to_ben(
            bad_xben,
            tmp_path / "out.ben",
            overwrite=True,
        )

    with pytest.raises(OSError, match="must differ"):
        decode_xben_to_jsonl(
            bad_xben,
            bad_xben,
            overwrite=True,
        )

    src = tmp_path / "src.jsonl"
    write_jsonl([[1, 2, 3]], src)
    ben = tmp_path / "good.ben"
    xben = tmp_path / "good.xben"
    encode_jsonl_to_ben(src, ben, overwrite=True, variant="standard")
    encode_ben_to_xben(ben, xben, overwrite=True, n_threads=1, compression_level=1)

    out = tmp_path / "exists.jsonl"
    out.write_text("exists\n", encoding="utf-8")
    with pytest.raises(OSError, match="already exists"):
        decode_ben_to_jsonl(ben, out, overwrite=False)


# ---------------------------------------------------------------------------
# Bundle inspection via BenDecoder
# ---------------------------------------------------------------------------


def test_decoder_bundle_round_trip_all_methods(tmp_path: Path) -> None:
    samples = [[1, 2, 3], [4, 5, 6], [7, 8, 9]]
    graph = {"nodes": [{"id": 0}, {"id": 1}], "links": [{"source": 0, "target": 1}]}
    path = tmp_path / "full.bendl"
    with BenEncoder(path, overwrite=True, variant="standard", graph=graph) as enc:
        for a in samples:
            enc.write(a)

    dec = BenDecoder(path)
    assert dec.is_bundle()
    assert dec.is_complete()
    assert dec.count_samples() == len(samples)
    assert dec.assignment_format() == "ben"
    v = dec.version()
    assert isinstance(v, tuple) and len(v) == 2

    names = dec.asset_names()
    assert "graph.json" in names

    assets = dec.list_assets()
    assert len(assets) >= 1
    for entry in assets:
        assert "name" in entry
        assert "type" in entry
        assert "flags" in entry

    raw = dec.read_asset_bytes("graph.json")
    assert isinstance(raw, bytes)

    parsed = dec.read_json_asset("graph.json")
    assert parsed["nodes"] == graph["nodes"]

    g = dec.read_graph()
    assert g is not None
    assert g["nodes"] == graph["nodes"]

    assert dec.read_metadata() is None
    assert dec.read_relabel_map() is None

    assert list(dec) == samples


def test_decoder_bundle_extract_stream_and_decode(tmp_path: Path) -> None:
    samples = [[10, 20], [30, 40]]
    path = tmp_path / "extract.bendl"
    with BenEncoder(path, overwrite=True, variant="standard") as enc:
        for a in samples:
            enc.write(a)

    dec = BenDecoder(path)
    out = tmp_path / "extracted.ben"
    dec.extract_stream(out)
    assert list(BenDecoder(out, mode="ben")) == samples


def test_decoder_bundle_extract_stream_overwrite_and_refuse(tmp_path: Path) -> None:
    samples = [[1]]
    path = tmp_path / "ow.bendl"
    with BenEncoder(path, overwrite=True, variant="standard") as enc:
        enc.write(samples[0])

    dec = BenDecoder(path)
    out = tmp_path / "out.ben"
    dec.extract_stream(out)
    with pytest.raises(OSError, match="already exists"):
        dec.extract_stream(out, overwrite=False)
    dec.extract_stream(out, overwrite=True)
    assert list(BenDecoder(out, mode="ben")) == samples


def test_decoder_bundle_missing_asset_raises_keyerror(tmp_path: Path) -> None:
    path = tmp_path / "no_asset.bendl"
    with BenEncoder(path, overwrite=True, variant="standard") as enc:
        enc.write([1, 2])

    dec = BenDecoder(path)
    with pytest.raises(KeyError, match="nope"):
        dec.read_asset_bytes("nope")
    with pytest.raises(KeyError, match="nope"):
        dec.read_json_asset("nope")


# ---------------------------------------------------------------------------
# BenEncoder bundle-mode coverage
# ---------------------------------------------------------------------------


def test_benencoder_bundle_without_graph(tmp_path: Path) -> None:
    samples = [[1, 2], [3, 4]]
    path = tmp_path / "no_graph.bendl"
    with BenEncoder(path, overwrite=True, variant="standard") as enc:
        for a in samples:
            enc.write(a)

    dec = BenDecoder(path)
    assert dec.is_bundle()
    assert dec.assignment_format() == "ben"
    assert dec.read_graph() is None
    assert list(dec) == samples


def test_benencoder_bundle_graph_from_dict(tmp_path: Path) -> None:
    graph = {"test": True}
    path = tmp_path / "dict_graph.bendl"
    with BenEncoder(path, overwrite=True, variant="standard", graph=graph) as enc:
        enc.write([1])
    dec = BenDecoder(path)
    assert dec.read_graph() == graph


def test_benencoder_bundle_graph_from_bytes(tmp_path: Path) -> None:
    graph = {"test": "bytes"}
    path = tmp_path / "bytes_graph.bendl"
    with BenEncoder(
        path, overwrite=True, variant="standard", graph=json.dumps(graph).encode()
    ) as enc:
        enc.write([1])
    assert BenDecoder(path).read_graph() == graph


def test_benencoder_bundle_graph_from_bytearray(tmp_path: Path) -> None:
    graph = {"test": "bytearray"}
    path = tmp_path / "ba_graph.bendl"
    with BenEncoder(
        path,
        overwrite=True,
        variant="standard",
        graph=bytearray(json.dumps(graph).encode()),
    ) as enc:
        enc.write([1])
    assert BenDecoder(path).read_graph() == graph


def test_benencoder_bundle_graph_from_file_path(tmp_path: Path) -> None:
    graph = {"test": "path"}
    gpath = tmp_path / "g.json"
    gpath.write_text(json.dumps(graph), encoding="utf-8")
    path = tmp_path / "path_graph.bendl"
    with BenEncoder(path, overwrite=True, variant="standard", graph=gpath) as enc:
        enc.write([1])
    assert BenDecoder(path).read_graph() == graph


def test_benencoder_bundle_graph_from_str_path(tmp_path: Path) -> None:
    graph = {"test": "str_path"}
    gpath = tmp_path / "g2.json"
    gpath.write_text(json.dumps(graph), encoding="utf-8")
    path = tmp_path / "str_path_graph.bendl"
    with BenEncoder(path, overwrite=True, variant="standard", graph=str(gpath)) as enc:
        enc.write([1])
    assert BenDecoder(path).read_graph() == graph


def test_benencoder_bundle_graph_from_bytesio(tmp_path: Path) -> None:
    graph = {"test": "bytesio"}
    path = tmp_path / "bio_graph.bendl"
    with BenEncoder(
        path,
        overwrite=True,
        variant="standard",
        graph=io.BytesIO(json.dumps(graph).encode()),
    ) as enc:
        enc.write([1])
    assert BenDecoder(path).read_graph() == graph


def test_benencoder_bundle_graph_from_stringio(tmp_path: Path) -> None:
    graph = {"test": "stringio"}
    path = tmp_path / "sio_graph.bendl"
    with BenEncoder(
        path,
        overwrite=True,
        variant="standard",
        graph=io.StringIO(json.dumps(graph)),
    ) as enc:
        enc.write([1])
    assert BenDecoder(path).read_graph() == graph


def test_benencoder_bundle_rejects_graph_with_ben_file_only(tmp_path: Path) -> None:
    with pytest.raises(ValueError, match="graph.*cannot be combined"):
        BenEncoder(
            tmp_path / "bad.ben",
            overwrite=True,
            variant="standard",
            graph={"a": 1},
            ben_file_only=True,
        )


def test_benencoder_bundle_rejects_invalid_graph_type(tmp_path: Path) -> None:
    with pytest.raises(ValueError, match="graph must be"):
        BenEncoder(
            tmp_path / "bad.bendl",
            overwrite=True,
            variant="standard",
            graph=42,
        )


def test_benencoder_bundle_close_is_idempotent(tmp_path: Path) -> None:
    path = tmp_path / "idempotent.bendl"
    enc = BenEncoder(path, overwrite=True, variant="standard")
    enc.write([1, 2])
    enc.close()
    enc.close()
    assert list(BenDecoder(path)) == [[1, 2]]


def test_benencoder_bundle_write_after_close_raises(tmp_path: Path) -> None:
    path = tmp_path / "closed.bendl"
    enc = BenEncoder(path, overwrite=True, variant="standard")
    enc.write([1])
    enc.close()
    with pytest.raises(OSError, match="already been closed"):
        enc.write([2])


# ---------------------------------------------------------------------------
# BenDecoder bundle-path coverage
# ---------------------------------------------------------------------------


def test_bendecoder_bundle_auto_detect_and_iterate(tmp_path: Path) -> None:
    samples = [[1, 2], [3, 4], [5, 6]]
    path = tmp_path / "auto.bendl"
    with BenEncoder(path, overwrite=True, variant="standard") as enc:
        for a in samples:
            enc.write(a)
    dec = BenDecoder(path)
    assert dec.is_bundle()
    assert list(dec) == samples


def test_bendecoder_bundle_toc_methods(tmp_path: Path) -> None:
    graph = {"g": 1}
    path = tmp_path / "toc.bendl"
    with BenEncoder(path, overwrite=True, variant="standard", graph=graph) as enc:
        enc.write([1, 2, 3])

    dec = BenDecoder(path)
    assert dec.is_bundle()
    assert dec.assignment_format() == "ben"
    v = dec.version()
    assert isinstance(v, tuple) and len(v) == 2
    assert dec.is_complete()

    names = dec.asset_names()
    assert "graph.json" in names

    assets = dec.list_assets()
    assert len(assets) >= 1
    for entry in assets:
        assert "name" in entry
        assert "type" in entry
        assert "flags" in entry

    raw = dec.read_asset_bytes("graph.json")
    assert isinstance(raw, bytes)

    parsed = dec.read_json_asset("graph.json")
    assert parsed == graph

    assert dec.read_graph() == graph
    assert dec.read_metadata() is None
    assert dec.read_relabel_map() is None


def test_bendecoder_bundle_subsample_all_modes(tmp_path: Path) -> None:
    samples = [[i] for i in range(1, 11)]
    path = tmp_path / "subsample.bendl"
    with BenEncoder(path, overwrite=True, variant="standard") as enc:
        for a in samples:
            enc.write(a)

    dec = BenDecoder(path)
    dec.subsample_range(2, 5)
    assert list(dec) == samples[1:5]

    dec2 = BenDecoder(path)
    dec2.subsample_indices([1, 3, 10])
    assert list(dec2) == [samples[0], samples[2], samples[9]]

    dec3 = BenDecoder(path)
    dec3.subsample_every(3, 2)
    assert list(dec3) == [samples[1], samples[4], samples[7]]


def test_bendecoder_bundle_len_and_count(tmp_path: Path) -> None:
    samples = [[1], [2], [3], [4], [5]]
    path = tmp_path / "len.bendl"
    with BenEncoder(path, overwrite=True, variant="standard") as enc:
        for a in samples:
            enc.write(a)

    dec = BenDecoder(path)
    assert len(dec) == len(samples)
    assert dec.count_samples() == len(samples)
    assert list(dec) == samples


def test_bendecoder_bundle_iteration_restart(tmp_path: Path) -> None:
    samples = [[1, 2], [3, 4]]
    path = tmp_path / "restart.bendl"
    with BenEncoder(path, overwrite=True, variant="standard") as enc:
        for a in samples:
            enc.write(a)

    dec = BenDecoder(path)
    assert list(dec) == samples
    assert list(dec) == samples


def test_bendecoder_bundle_subsample_survives_reiteration(tmp_path: Path) -> None:
    samples = [[i] for i in range(1, 8)]
    path = tmp_path / "re_sub.bendl"
    with BenEncoder(path, overwrite=True, variant="standard") as enc:
        for a in samples:
            enc.write(a)

    dec = BenDecoder(path)
    dec.subsample_range(2, 5)
    expected = samples[1:5]
    assert list(dec) == expected
    assert list(dec) == expected


def test_bendecoder_plain_rejects_bundle_methods(tmp_path: Path) -> None:
    path = tmp_path / "plain.ben"
    with BenEncoder(
        path, overwrite=True, variant="standard", ben_file_only=True
    ) as enc:
        enc.write([1, 2])

    dec = BenDecoder(path)
    assert not dec.is_bundle()
    assert dec.assignment_format() == "ben"

    for method, args in [
        ("version", ()),
        ("is_complete", ()),
        ("asset_names", ()),
        ("list_assets", ()),
        ("read_asset_bytes", ("x",)),
        ("read_json_asset", ("x",)),
        ("read_graph", ()),
        ("read_metadata", ()),
        ("read_relabel_map", ()),
    ]:
        with pytest.raises(Exception, match="only available on .bendl"):
            getattr(dec, method)(*args)


def test_bendecoder_bundle_count_samples_preserves_subsample_len(
    tmp_path: Path,
) -> None:
    samples = [[i] for i in range(1, 9)]
    path = tmp_path / "count_sub.bendl"
    with BenEncoder(path, overwrite=True, variant="standard") as enc:
        for a in samples:
            enc.write(a)

    dec = BenDecoder(path)
    dec.subsample_range(2, 5)
    assert len(dec) == 4
    assert dec.count_samples() == len(samples)
    assert len(dec) == 4


# ---------------------------------------------------------------------------
# BenDecoder XBEN bundle coverage
# ---------------------------------------------------------------------------


def test_bendecoder_xben_bundle_roundtrip(tmp_path: Path) -> None:
    samples = [[1, 2], [3, 4], [5, 6]]
    src = tmp_path / "src.jsonl"
    write_jsonl(samples, src)

    xben_path = tmp_path / "samples.xben"
    encode_jsonl_to_xben(
        src,
        xben_path,
        overwrite=True,
        variant="standard",
        n_threads=1,
        compression_level=1,
    )

    bendl_path = tmp_path / "xben_bundle.bendl"
    with BenEncoder(bendl_path, overwrite=True, variant="standard") as enc:
        for a in samples:
            enc.write(a)

    dec = BenDecoder(bendl_path)
    assert dec.is_bundle()
    assert list(dec) == samples


def test_bendecoder_xben_plain_stream(tmp_path: Path) -> None:
    samples = [[1, 2], [3, 4]]
    src = tmp_path / "src.jsonl"
    write_jsonl(samples, src)

    xben_path = tmp_path / "plain.xben"
    encode_jsonl_to_xben(
        src,
        xben_path,
        overwrite=True,
        variant="standard",
        n_threads=1,
        compression_level=1,
    )

    dec = BenDecoder(xben_path, mode="xben")
    assert not dec.is_bundle()
    assert dec.assignment_format() == "xben"
    assert list(dec) == samples


# ---------------------------------------------------------------------------
# BenDecoder subsample validation errors
# ---------------------------------------------------------------------------


def test_bendecoder_subsample_indices_empty_raises(tmp_path: Path) -> None:
    samples = [[1], [2]]
    path = tmp_path / "empty_idx.bendl"
    with BenEncoder(path, overwrite=True, variant="standard") as enc:
        for a in samples:
            enc.write(a)

    dec = BenDecoder(path)
    with pytest.raises(Exception):
        dec.subsample_indices([])


def test_bendecoder_subsample_indices_zero_raises(tmp_path: Path) -> None:
    samples = [[1], [2]]
    path = tmp_path / "zero_idx.bendl"
    with BenEncoder(path, overwrite=True, variant="standard") as enc:
        for a in samples:
            enc.write(a)

    dec = BenDecoder(path)
    with pytest.raises(Exception):
        dec.subsample_indices([0, 1, 2])


def test_bendecoder_subsample_range_zero_start_raises(tmp_path: Path) -> None:
    samples = [[1], [2]]
    path = tmp_path / "zero_start.bendl"
    with BenEncoder(path, overwrite=True, variant="standard") as enc:
        for a in samples:
            enc.write(a)

    dec = BenDecoder(path)
    with pytest.raises(Exception):
        dec.subsample_range(0, 2)


def test_bendecoder_subsample_range_end_lt_start_raises(tmp_path: Path) -> None:
    samples = [[1], [2]]
    path = tmp_path / "bad_range.bendl"
    with BenEncoder(path, overwrite=True, variant="standard") as enc:
        for a in samples:
            enc.write(a)

    dec = BenDecoder(path)
    with pytest.raises(Exception):
        dec.subsample_range(5, 2)


def test_bendecoder_subsample_every_zero_step_raises(tmp_path: Path) -> None:
    samples = [[1], [2]]
    path = tmp_path / "zero_step.bendl"
    with BenEncoder(path, overwrite=True, variant="standard") as enc:
        for a in samples:
            enc.write(a)

    dec = BenDecoder(path)
    with pytest.raises(Exception):
        dec.subsample_every(0)


def test_bendecoder_subsample_every_zero_offset_raises(tmp_path: Path) -> None:
    samples = [[1], [2]]
    path = tmp_path / "zero_off.bendl"
    with BenEncoder(path, overwrite=True, variant="standard") as enc:
        for a in samples:
            enc.write(a)

    dec = BenDecoder(path)
    with pytest.raises(Exception):
        dec.subsample_every(1, offset=0)


# ---------------------------------------------------------------------------
# BenDecoder subsample on plain streams
# ---------------------------------------------------------------------------


def test_bendecoder_plain_subsample_indices(tmp_path: Path) -> None:
    samples = [[1], [2], [3], [4], [5]]
    path = tmp_path / "plain_sub.ben"
    with BenEncoder(
        path, overwrite=True, variant="standard", ben_file_only=True
    ) as enc:
        for a in samples:
            enc.write(a)

    dec = BenDecoder(path)
    dec.subsample_indices([1, 3, 5])
    assert list(dec) == [[1], [3], [5]]


def test_bendecoder_plain_subsample_range(tmp_path: Path) -> None:
    samples = [[1], [2], [3], [4], [5]]
    path = tmp_path / "plain_range.ben"
    with BenEncoder(
        path, overwrite=True, variant="standard", ben_file_only=True
    ) as enc:
        for a in samples:
            enc.write(a)

    dec = BenDecoder(path)
    dec.subsample_range(2, 4)
    assert list(dec) == [[2], [3], [4]]


def test_bendecoder_plain_subsample_every(tmp_path: Path) -> None:
    samples = [[1], [2], [3], [4], [5], [6]]
    path = tmp_path / "plain_every.ben"
    with BenEncoder(
        path, overwrite=True, variant="standard", ben_file_only=True
    ) as enc:
        for a in samples:
            enc.write(a)

    dec = BenDecoder(path)
    dec.subsample_every(2, offset=1)
    assert list(dec) == [[1], [3], [5]]


# ---------------------------------------------------------------------------
# BenDecoder len/count on plain streams
# ---------------------------------------------------------------------------


def test_bendecoder_plain_len_and_count(tmp_path: Path) -> None:
    samples = [[1], [2], [3]]
    path = tmp_path / "plain_len.ben"
    with BenEncoder(
        path, overwrite=True, variant="standard", ben_file_only=True
    ) as enc:
        for a in samples:
            enc.write(a)

    dec = BenDecoder(path)
    assert dec.count_samples() == 3
    assert len(dec) == 3


def test_bendecoder_plain_len_after_subsample(tmp_path: Path) -> None:
    samples = [[1], [2], [3], [4], [5]]
    path = tmp_path / "plain_sub_len.ben"
    with BenEncoder(
        path, overwrite=True, variant="standard", ben_file_only=True
    ) as enc:
        for a in samples:
            enc.write(a)

    dec = BenDecoder(path)
    dec.subsample_range(2, 4)
    assert len(dec) == 3
    assert dec.count_samples() == 5
    assert len(dec) == 3


# ---------------------------------------------------------------------------
# BenDecoder multiple iteration passes
# ---------------------------------------------------------------------------


def test_bendecoder_plain_multiple_iterations(tmp_path: Path) -> None:
    samples = [[1, 2], [3, 4]]
    path = tmp_path / "multi_iter.ben"
    with BenEncoder(
        path, overwrite=True, variant="standard", ben_file_only=True
    ) as enc:
        for a in samples:
            enc.write(a)

    dec = BenDecoder(path)
    assert list(dec) == samples
    assert list(dec) == samples
    assert list(dec) == samples


def test_bendecoder_plain_subsample_survives_reiteration(tmp_path: Path) -> None:
    samples = [[i] for i in range(1, 8)]
    path = tmp_path / "plain_re_sub.ben"
    with BenEncoder(
        path, overwrite=True, variant="standard", ben_file_only=True
    ) as enc:
        for a in samples:
            enc.write(a)

    dec = BenDecoder(path)
    dec.subsample_every(2, offset=1)
    expected = [[1], [3], [5], [7]]
    assert list(dec) == expected
    assert list(dec) == expected


# ---------------------------------------------------------------------------
# BenEncoder ben_file_only mode coverage
# ---------------------------------------------------------------------------


def test_benencoder_ben_file_only_roundtrip(tmp_path: Path) -> None:
    samples = [[10, 20, 30], [40, 50, 60]]
    path = tmp_path / "ben_only.ben"
    with BenEncoder(
        path, overwrite=True, variant="standard", ben_file_only=True
    ) as enc:
        for a in samples:
            enc.write(a)

    dec = BenDecoder(path, mode="ben")
    assert not dec.is_bundle()
    assert list(dec) == samples


def test_benencoder_ben_file_only_mkv(tmp_path: Path) -> None:
    samples = [[1, 2], [1, 2], [3, 4]]
    path = tmp_path / "ben_mkv.ben"
    with BenEncoder(
        path, overwrite=True, variant="mkv_chain", ben_file_only=True
    ) as enc:
        for a in samples:
            enc.write(a)

    dec = BenDecoder(path, mode="ben")
    assert list(dec) == samples


def test_benencoder_ben_file_only_close_and_reopen(tmp_path: Path) -> None:
    samples = [[5, 6]]
    path = tmp_path / "close_reopen.ben"
    enc = BenEncoder(path, overwrite=True, variant="standard", ben_file_only=True)
    enc.write(samples[0])
    enc.close()

    dec = BenDecoder(path, mode="ben")
    assert list(dec) == samples


# ---------------------------------------------------------------------------
# BenEncoder bundle with metadata
# ---------------------------------------------------------------------------


def test_benencoder_bundle_with_metadata(tmp_path: Path) -> None:
    samples = [[1, 2]]
    graph = {"nodes": [{"id": 0}], "adjacency": [[]]}
    path = tmp_path / "with_meta.bendl"
    with BenEncoder(path, overwrite=True, variant="standard", graph=graph) as enc:
        enc.write(samples[0])

    dec = BenDecoder(path)
    assert dec.read_graph() == graph
    assert list(dec) == samples


# ---------------------------------------------------------------------------
# BenDecoder extract_stream on plain stream raises
# ---------------------------------------------------------------------------


def test_bendecoder_extract_stream_on_plain_raises(tmp_path: Path) -> None:
    path = tmp_path / "plain_extract.ben"
    with BenEncoder(
        path, overwrite=True, variant="standard", ben_file_only=True
    ) as enc:
        enc.write([1, 2])

    dec = BenDecoder(path, mode="ben")
    with pytest.raises(Exception, match="only available on .bendl"):
        dec.extract_stream(tmp_path / "out.ben")


# ---------------------------------------------------------------------------
# decode_ben_to_jsonl and decode_xben_to_jsonl coverage
# ---------------------------------------------------------------------------


def test_decode_ben_to_jsonl_roundtrip(tmp_path: Path) -> None:
    samples = [[1, 2, 3], [4, 5, 6]]
    src = tmp_path / "src.jsonl"
    write_jsonl(samples, src)

    ben = tmp_path / "out.ben"
    encode_jsonl_to_ben(src, ben, overwrite=True, variant="standard")

    out = tmp_path / "round.jsonl"
    decode_ben_to_jsonl(ben, out, overwrite=True)

    restored = read_jsonl_assignments(out)
    assert restored == samples


def test_decode_xben_to_jsonl_roundtrip(tmp_path: Path) -> None:
    samples = [[1, 2, 3], [4, 5, 6]]
    src = tmp_path / "src.jsonl"
    write_jsonl(samples, src)

    xben = tmp_path / "out.xben"
    encode_jsonl_to_xben(
        src,
        xben,
        overwrite=True,
        variant="standard",
        n_threads=1,
        compression_level=1,
    )

    out = tmp_path / "round.jsonl"
    decode_xben_to_jsonl(xben, out, overwrite=True)

    restored = read_jsonl_assignments(out)
    assert restored == samples


# ---------------------------------------------------------------------------
# encode_ben_to_xben coverage
# ---------------------------------------------------------------------------


def test_encode_ben_to_xben_roundtrip(tmp_path: Path) -> None:
    samples = [[1, 2], [3, 4], [5, 6]]
    src = tmp_path / "src.jsonl"
    write_jsonl(samples, src)

    ben = tmp_path / "out.ben"
    encode_jsonl_to_ben(src, ben, overwrite=True, variant="standard")

    xben = tmp_path / "from_ben.xben"
    encode_ben_to_xben(ben, xben, overwrite=True, n_threads=1, compression_level=1)

    out = tmp_path / "round.jsonl"
    decode_xben_to_jsonl(xben, out, overwrite=True)

    restored = read_jsonl_assignments(out)
    assert restored == samples


# ---------------------------------------------------------------------------
# BenDecoder unknown mode error
# ---------------------------------------------------------------------------


def test_bendecoder_unknown_mode_raises(tmp_path: Path) -> None:
    path = tmp_path / "dummy.ben"
    path.write_bytes(b"\x00" * 100)
    with pytest.raises(Exception):
        BenDecoder(path, mode="bogus")


# ---------------------------------------------------------------------------
# BenDecoder MkvChain plain stream
# ---------------------------------------------------------------------------


def test_bendecoder_mkv_plain_stream(tmp_path: Path) -> None:
    samples = [[1, 2], [1, 2], [3, 4]]
    src = tmp_path / "mkv_src.jsonl"
    write_jsonl(samples, src)

    ben = tmp_path / "mkv.ben"
    encode_jsonl_to_ben(src, ben, overwrite=True, variant="mkv_chain")

    dec = BenDecoder(ben, mode="ben")
    assert list(dec) == samples
    assert dec.count_samples() == 3
