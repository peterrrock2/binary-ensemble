import json
import random
from pathlib import Path
from typing import Iterable, List

import pytest

from pyben import (
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
    with PyBenEncoder(ben, overwrite=True, variant="standard") as enc:
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
