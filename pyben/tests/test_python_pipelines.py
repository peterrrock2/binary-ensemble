import json
import random
from pathlib import Path
from typing import Iterable, List

import pytest

import binary_ensemble
from binary_ensemble import (
    PyBenDecoder,
    PyBenEncoder,
    compress_ben_to_xben,
    compress_jsonl_to_ben,
    compress_jsonl_to_xben,
    decompress_ben_to_jsonl,
    decompress_xben_to_ben,
    decompress_xben_to_jsonl,
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

    compress_jsonl_to_ben(src, ben, overwrite=True, variant="standard")
    decompress_ben_to_jsonl(ben, out_jsonl, overwrite=True)

    assert src.read_bytes() == out_jsonl.read_bytes()


def test_mkvben_pipeline(tmp_path: Path) -> None:
    rng = random.Random(129530786)
    n_samples = 100
    seq = gen_sequence_mkv(rng, n_samples)

    src = tmp_path / "src.jsonl"
    write_jsonl(seq, src)

    ben = tmp_path / "out_mkv.ben"
    out_jsonl = tmp_path / "round_mkv.jsonl"

    compress_jsonl_to_ben(src, ben, overwrite=True, variant="mkv_chain")
    decompress_ben_to_jsonl(ben, out_jsonl, overwrite=True)

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

    compress_jsonl_to_xben(
        src, xben, overwrite=True, variant="standard", n_threads=1, compression_level=1
    )
    decompress_xben_to_ben(xben, ben, overwrite=True)
    decompress_ben_to_jsonl(ben, round_jsonl, overwrite=True)

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

    compress_jsonl_to_xben(
        src, xben, overwrite=True, variant="mkv_chain", n_threads=1, compression_level=1
    )
    decompress_xben_to_ben(xben, ben, overwrite=True)
    decompress_ben_to_jsonl(ben, round_jsonl, overwrite=True)

    assert src.read_bytes() == round_jsonl.read_bytes()


# ---------- Iterator/decoder parity with JSONL ----------


def test_decoder_iterator_matches_jsonl_ben(tmp_path: Path) -> None:
    rng = random.Random(129530786)
    n_samples = 120
    seq = gen_sequence_standard(rng, n_samples)

    src = tmp_path / "src.jsonl"
    write_jsonl(seq, src)

    ben = tmp_path / "out.ben"
    compress_jsonl_to_ben(src, ben, overwrite=True, variant="standard")

    # Baseline: assignments from JSONL
    baseline = read_jsonl_assignments(src)

    # PyBenDecoder over BEN
    got: list[list[int]] = []
    dec = PyBenDecoder(ben, mode="ben")
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
    compress_jsonl_to_xben(
        src, xben, overwrite=True, variant="mkv_chain", n_threads=1, compression_level=1
    )

    # Baseline via full decompression
    roundtrip = tmp_path / "direct.jsonl"
    decompress_xben_to_jsonl(xben, roundtrip, overwrite=True)
    baseline = read_jsonl_assignments(roundtrip)

    # Iterator directly over XBEN
    got: list[list[int]] = []
    dec = PyBenDecoder(xben, mode="xben")
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
    compress_jsonl_to_xben(
        src, xben, overwrite=True, variant="mkv_chain", n_threads=1, compression_level=1
    )

    # choose indices: 1,4,7,…
    want = list(range(1, n_samples + 1, 3))
    baseline = [seq[i - 1] for i in want]

    got: list[list[int]] = []
    dec = PyBenDecoder(xben, mode="xben").subsample_indices(want)
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
    compress_jsonl_to_ben(src, ben, overwrite=True, variant="mkv_chain")

    start, end = 11, 77
    baseline = seq[start - 1 : end]

    got: list[list[int]] = []
    dec = PyBenDecoder(ben, mode="ben").subsample_range(start, end)
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
    compress_jsonl_to_xben(
        src, xben, overwrite=True, variant="mkv_chain", n_threads=1, compression_level=1
    )

    step, offset = 5, 2  # keep 2,7,12,…
    baseline = [seq[i - 1] for i in range(offset, n_samples + 1, step)]

    got: list[list[int]] = []
    dec = PyBenDecoder(xben, mode="xben").subsample_every(step, offset)
    for a in dec:
        got.append(a)

    assert got == baseline


# ---------- Encoder surface (context manager & write) ----------


def test_pybenencoder_roundtrip(tmp_path: Path) -> None:
    rng = random.Random(777)
    n_samples = 60
    seq = gen_sequence_standard(rng, n_samples)

    ben = tmp_path / "out.ben"
    with PyBenEncoder(
        ben, overwrite=True, variant="standard", ben_file_only=True
    ) as enc:
        for a in seq:
            enc.write(a)

    # Use decoder to read back
    got = list(PyBenDecoder(ben, mode="ben"))
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

    compress_jsonl_to_ben(src, ben, overwrite=True, variant="mkv_chain")
    compress_ben_to_xben(ben, xben, overwrite=True, n_threads=1, compression_level=1)
    decompress_xben_to_ben(xben, ben2, overwrite=True)
    decompress_ben_to_jsonl(ben2, out_jsonl, overwrite=True)

    assert src.read_bytes() == out_jsonl.read_bytes()


def test_decoder_subsample_indices_rejects_empty_input(tmp_path: Path) -> None:
    rng = random.Random(123)
    seq = gen_sequence_standard(rng, 10)

    src = tmp_path / "src.jsonl"
    write_jsonl(seq, src)

    ben = tmp_path / "out.ben"
    compress_jsonl_to_ben(src, ben, overwrite=True, variant="standard")

    dec = PyBenDecoder(ben, mode="ben")
    with pytest.raises(Exception, match="indices must not be empty"):
        dec.subsample_indices([])


def test_decoder_subsample_every_rejects_offset_past_end(tmp_path: Path) -> None:
    rng = random.Random(456)
    seq = gen_sequence_standard(rng, 10)

    src = tmp_path / "src.jsonl"
    write_jsonl(seq, src)

    ben = tmp_path / "out.ben"
    compress_jsonl_to_ben(src, ben, overwrite=True, variant="standard")

    dec = PyBenDecoder(ben, mode="ben")
    with pytest.raises(Exception, match="offset must be <="):
        dec.subsample_every(2, 99)


def test_compress_helpers_reject_unknown_variants(tmp_path: Path) -> None:
    rng = random.Random(789)
    seq = gen_sequence_standard(rng, 5)

    src = tmp_path / "src.jsonl"
    write_jsonl(seq, src)

    with pytest.raises(ValueError, match="Unknown variant"):
        compress_jsonl_to_ben(src, tmp_path / "out.ben", overwrite=True, variant="weird")

    with pytest.raises(ValueError, match="Unknown variant"):
        compress_jsonl_to_xben(src, tmp_path / "out.xben", overwrite=True, variant="weird")


def test_module_exports_are_exposed() -> None:
    expected = {
        "PyBenDecoder",
        "PyBenEncoder",
        "compress_jsonl_to_ben",
        "compress_ben_to_xben",
        "compress_jsonl_to_xben",
        "decompress_ben_to_jsonl",
        "decompress_xben_to_jsonl",
        "decompress_xben_to_ben",
    }
    assert expected.issubset(set(binary_ensemble.__all__))
    for name in expected:
        assert hasattr(binary_ensemble, name)
    assert hasattr(binary_ensemble, "_core")


def test_pybenencoder_defaults_and_markov_alias_work(tmp_path: Path) -> None:
    samples = [[1, 1, 2], [1, 1, 2], [2, 3, 3]]

    default_ben = tmp_path / "default.ben"
    with PyBenEncoder(default_ben, overwrite=True, ben_file_only=True) as enc:
        for sample in samples:
            enc.write(sample)
    assert list(PyBenDecoder(default_ben, mode="ben")) == samples

    src = tmp_path / "src.jsonl"
    write_jsonl(samples, src)

    alias_ben = tmp_path / "alias.ben"
    alias_xben = tmp_path / "alias.xben"
    compress_jsonl_to_ben(src, alias_ben, overwrite=True, variant="markov")
    compress_jsonl_to_xben(
        src,
        alias_xben,
        overwrite=True,
        variant="markov",
        n_threads=1,
        compression_level=1,
    )
    assert list(PyBenDecoder(alias_ben, mode="ben")) == samples
    assert list(PyBenDecoder(alias_xben, mode="xben")) == samples


def test_pybenencoder_close_and_write_error_paths(tmp_path: Path) -> None:
    out = tmp_path / "out.ben"
    enc = PyBenEncoder(
        out, overwrite=True, variant="standard", ben_file_only=True
    )
    enc.write([1, 2, 3])
    enc.close()
    enc.close()
    with pytest.raises(OSError, match="already been closed"):
        enc.write([1, 2, 3])

    ctx_path = tmp_path / "ctx.ben"
    with PyBenEncoder(
        ctx_path, overwrite=True, variant="standard", ben_file_only=True
    ) as ctx_enc:
        ctx_enc.write([4, 5, 6])
    assert list(PyBenDecoder(ctx_path, mode="ben")) == [[4, 5, 6]]


def test_pybenencoder_rejects_overwrite_and_unknown_variant(tmp_path: Path) -> None:
    out = tmp_path / "out.ben"
    out.write_bytes(b"existing")

    with pytest.raises(ValueError, match="Unknown variant"):
        PyBenEncoder(tmp_path / "bad.ben", overwrite=False, variant="weird")

    with pytest.raises(OSError, match="already exists"):
        PyBenEncoder(out, overwrite=False, variant="standard")

    with pytest.raises(OSError, match="Failed to create"):
        PyBenEncoder(
            tmp_path / "missing-dir" / "out.ben",
            overwrite=False,
            variant="standard",
        )


def test_compress_helpers_reject_same_path_missing_input_and_bad_json(tmp_path: Path) -> None:
    src = tmp_path / "src.jsonl"
    write_jsonl([[1, 1, 2]], src)

    with pytest.raises(OSError, match="must differ"):
        compress_jsonl_to_ben(src, src, overwrite=True, variant="standard")

    with pytest.raises(OSError, match="does not exist"):
        compress_jsonl_to_ben(
            tmp_path / "missing.jsonl",
            tmp_path / "out.ben",
            overwrite=True,
            variant="standard",
        )

    bad_json = tmp_path / "bad.jsonl"
    bad_json.write_text("not json\n", encoding="utf-8")
    with pytest.raises(OSError, match="Failed to convert JSONL to BEN"):
        compress_jsonl_to_ben(
            bad_json,
            tmp_path / "bad.ben",
            overwrite=True,
            variant="standard",
        )

    bad_assign = tmp_path / "bad_assign.jsonl"
    bad_assign.write_text('{"assignment":"bad","sample":1}\n', encoding="utf-8")
    with pytest.raises(OSError, match="Failed to convert JSONL to XBEN"):
        compress_jsonl_to_xben(
            bad_assign,
            tmp_path / "bad.xben",
            overwrite=True,
            variant="standard",
            n_threads=1,
            compression_level=1,
        )

    with pytest.raises(OSError, match="Failed to create"):
        compress_jsonl_to_ben(
            src,
            tmp_path / "missing-dir" / "out.ben",
            overwrite=True,
            variant="standard",
        )


def test_compress_ben_to_xben_rejects_same_path_missing_input_invalid_header_and_existing_output(
    tmp_path: Path,
) -> None:
    with pytest.raises(OSError, match="does not exist"):
        compress_ben_to_xben(
            tmp_path / "missing.ben",
            tmp_path / "out.xben",
            overwrite=True,
            n_threads=1,
            compression_level=1,
        )

    bad_ben = tmp_path / "bad.ben"
    bad_ben.write_bytes(b"garbage")

    with pytest.raises(OSError, match="must differ"):
        compress_ben_to_xben(
            bad_ben,
            bad_ben,
            overwrite=True,
            n_threads=1,
            compression_level=1,
        )

    with pytest.raises(OSError, match="Failed to convert BEN to XBEN"):
        compress_ben_to_xben(
            bad_ben,
            tmp_path / "out.xben",
            overwrite=True,
            n_threads=1,
            compression_level=1,
        )

    src = tmp_path / "src.jsonl"
    write_jsonl([[1, 2, 3]], src)
    ben = tmp_path / "good.ben"
    compress_jsonl_to_ben(src, ben, overwrite=True, variant="standard")
    out = tmp_path / "exists.xben"
    out.write_bytes(b"exists")
    with pytest.raises(OSError, match="already exists"):
        compress_ben_to_xben(
            ben,
            out,
            overwrite=False,
            n_threads=1,
            compression_level=1,
        )


def test_decoder_constructor_and_mode_errors(tmp_path: Path) -> None:
    with pytest.raises(Exception, match="Unknown mode"):
        PyBenDecoder(tmp_path / "missing.ben", mode="weird")

    with pytest.raises(OSError, match="Failed to open"):
        PyBenDecoder(tmp_path / "missing.ben", mode="ben")

    bad_ben = tmp_path / "bad.ben"
    bad_ben.write_bytes(b"garbage")
    with pytest.raises(Exception, match="Failed to create BenDecoder"):
        PyBenDecoder(bad_ben, mode="ben")

    bad_xben = tmp_path / "bad.xben"
    bad_xben.write_bytes(b"garbage")
    with pytest.warns(UserWarning, match="XBEN may take a second"):
        with pytest.raises(Exception, match="Failed to create XBenDecoder"):
            PyBenDecoder(bad_xben, mode="xben")


def test_decoder_len_and_count_samples_are_lazy_and_cached(tmp_path: Path) -> None:
    samples = [[1, 1, 2], [1, 1, 2], [2, 3, 3], [4]]
    src = tmp_path / "src.jsonl"
    write_jsonl(samples, src)

    ben = tmp_path / "out.ben"
    compress_jsonl_to_ben(src, ben, overwrite=True, variant="mkv_chain")

    dec = PyBenDecoder(ben, mode="ben")
    assert len(dec) == len(samples)
    assert dec.count_samples() == len(samples)
    assert list(dec) == samples

    gone = PyBenDecoder(ben, mode="ben")
    assert len(gone) == len(samples)
    ben.unlink()
    with pytest.raises(Exception, match="Failed to create frame iterator"):
        gone.subsample_range(1, 2)


def test_decoder_xben_len_count_and_warning(tmp_path: Path) -> None:
    samples = [[1, 1], [1, 1], [2, 2], [3, 3]]
    src = tmp_path / "src.jsonl"
    write_jsonl(samples, src)

    xben = tmp_path / "out.xben"
    compress_jsonl_to_xben(
        src, xben, overwrite=True, variant="mkv_chain", n_threads=1, compression_level=1
    )

    with pytest.warns(UserWarning, match="XBEN may take a second"):
        dec = PyBenDecoder(xben, mode="xben")
    assert len(dec) == len(samples)
    assert dec.count_samples() == len(samples)
    assert list(dec) == samples


def test_decoder_subsample_validations_and_warning_paths(tmp_path: Path) -> None:
    samples = [[1], [2], [3], [4], [5]]
    src = tmp_path / "src.jsonl"
    write_jsonl(samples, src)

    ben = tmp_path / "out.ben"
    compress_jsonl_to_ben(src, ben, overwrite=True, variant="standard")

    with pytest.warns(UserWarning, match="sorted and unique"):
        got = list(PyBenDecoder(ben, mode="ben").subsample_indices([5, 1, 1, 3]))
    assert got == [samples[0], samples[2], samples[4]]

    with pytest.raises(Exception, match="indices must be 1-based"):
        PyBenDecoder(ben, mode="ben").subsample_indices([0, 1])

    with pytest.raises(Exception, match="indices must be <="):
        PyBenDecoder(ben, mode="ben").subsample_indices([6])

    with pytest.raises(Exception, match="range must be 1-based"):
        PyBenDecoder(ben, mode="ben").subsample_range(0, 2)

    with pytest.raises(Exception, match="end must be <="):
        PyBenDecoder(ben, mode="ben").subsample_range(1, 99)

    with pytest.raises(Exception, match="step and offset must be >= 1"):
        PyBenDecoder(ben, mode="ben").subsample_every(0, 1)

    with pytest.raises(Exception, match="offset must be <="):
        PyBenDecoder(ben, mode="ben").subsample_every(2, 99)

    assert list(PyBenDecoder(ben, mode="ben").subsample_range(2, 4)) == samples[1:4]
    assert list(PyBenDecoder(ben, mode="ben").subsample_every(2, 2)) == samples[1::2]


def test_decoder_count_and_subsample_fail_cleanly_if_source_disappears(tmp_path: Path) -> None:
    src = tmp_path / "src.jsonl"
    write_jsonl([[1], [2], [3]], src)

    ben = tmp_path / "out.ben"
    compress_jsonl_to_ben(src, ben, overwrite=True, variant="standard")

    dec = PyBenDecoder(ben, mode="ben")
    ben.unlink()

    with pytest.raises(Exception, match="Failed to count samples"):
        dec.count_samples()


def test_decoder_reports_zero_count_and_bad_frame_errors(tmp_path: Path) -> None:
    src = tmp_path / "src.jsonl"
    write_jsonl([[1, 1, 2]], src)

    mkv_ben = tmp_path / "mkv.ben"
    compress_jsonl_to_ben(src, mkv_ben, overwrite=True, variant="mkv_chain")
    data = bytearray(mkv_ben.read_bytes())
    data[-2:] = b"\x00\x00"
    mkv_ben.write_bytes(data)
    with pytest.raises(Exception, match="zero-count"):
        next(iter(PyBenDecoder(mkv_ben, mode="ben")))

    standard_ben = tmp_path / "standard.ben"
    compress_jsonl_to_ben(src, standard_ben, overwrite=True, variant="standard")
    truncated = standard_ben.read_bytes()[:-1]
    bad_ben = tmp_path / "truncated.ben"
    bad_ben.write_bytes(truncated)
    dec = PyBenDecoder(bad_ben, mode="ben")
    with pytest.raises(Exception, match="Error decoding next item"):
        next(iter(dec))


def test_decode_helpers_reject_same_paths_missing_inputs_existing_output_and_invalid_headers(
    tmp_path: Path,
) -> None:
    with pytest.raises(OSError, match="does not exist"):
        decompress_ben_to_jsonl(
            tmp_path / "missing.ben",
            tmp_path / "out.jsonl",
            overwrite=True,
        )

    bad_ben = tmp_path / "bad.ben"
    bad_ben.write_bytes(b"garbage")
    with pytest.raises(OSError, match="Failed to convert BEN to JSONL"):
        decompress_ben_to_jsonl(
            bad_ben,
            tmp_path / "out.jsonl",
            overwrite=True,
        )

    bad_xben = tmp_path / "bad.xben"
    bad_xben.write_bytes(b"garbage")
    with pytest.raises(OSError, match="Failed to convert XBEN to BEN"):
        decompress_xben_to_ben(
            bad_xben,
            tmp_path / "out.ben",
            overwrite=True,
        )

    with pytest.raises(OSError, match="must differ"):
        decompress_xben_to_jsonl(
            bad_xben,
            bad_xben,
            overwrite=True,
        )

    src = tmp_path / "src.jsonl"
    write_jsonl([[1, 2, 3]], src)
    ben = tmp_path / "good.ben"
    xben = tmp_path / "good.xben"
    compress_jsonl_to_ben(src, ben, overwrite=True, variant="standard")
    compress_ben_to_xben(ben, xben, overwrite=True, n_threads=1, compression_level=1)

    out = tmp_path / "exists.jsonl"
    out.write_text("exists\n", encoding="utf-8")
    with pytest.raises(OSError, match="already exists"):
        decompress_ben_to_jsonl(ben, out, overwrite=False)
