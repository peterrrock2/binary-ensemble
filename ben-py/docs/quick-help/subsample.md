# Subsample a large ensemble

When an ensemble has millions of plans, you often want only a slice — every 1000th plan, a
contiguous range, or a handful of specific indices. The decoders support this directly: they
never materialize the samples you skip, and where the stream allows it they skip whole
frames without unpacking them.

All three methods are available on both `BendlDecoder` (for bundles) and `BenDecoder` (for
plain streams). Each returns a decoder you iterate. Indices are **zero-based**; out-of-range
indices raise rather than being silently dropped, duplicate indices are dropped, and an unsorted
index list is sorted (with a `UserWarning`).

```{note}
How cheap skipping is depends on the stream's [encoding variant](../concepts/variants.md).
`standard` and `mkv_chain` frames state their byte length up front, so the reader hops
straight over unwanted samples. `twodelta` — the default — stores most samples as deltas, so
the reader still has to replay the deltas between snapshots to reconstruct the samples you
keep. Plain BEN `twodelta` writers insert checkpoint snapshots after at most 50,000 dependent
delta frames, so replay is bounded by nearby snapshots, but skipped samples are still not free. If
heavy random access or repeated subsampling is your primary workload, encode with
`variant="standard"` or `variant="mkv_chain"`.
```

## By specific indices

```python
from binary_ensemble import BendlDecoder

for assignment in BendlDecoder("ensemble.bendl").subsample_indices([0, 49, 99]):
    print(assignment[:10])
```

## By a contiguous range

Ranges are **zero-based and half-open**, like Python slices: `subsample_range(10, 15)` yields
indices 10, 11, 12, 13, and 14.

```python
for assignment in BendlDecoder("ensemble.bendl").subsample_range(10, 15):
    print(assignment[:10])
```

## By a fixed stride

`subsample_every(step, offset=0)` follows the same positions as `plans[offset::step]`:

```python
for assignment in BendlDecoder("ensemble.bendl").subsample_every(25):
    print(assignment[:10])
```

## Subsampling plain streams (and XBEN)

The same methods work on a `BenDecoder`. For an `.xben` stream, pass `mode="xben"`:

```python
from binary_ensemble import BenDecoder

# Plain BEN stream — skipping is cheapest here.
for assignment in BenDecoder("chain.ben").subsample_every(25):
    print(assignment[:10])

# XBEN works too, at the cost of a one-time decompression startup.
for assignment in BenDecoder("chain.xben", mode="xben").subsample_range(10, 15):
    print(assignment[:10])
```

```{tip}
A BEN stream is the cheapest container to subsample. An XBEN stream pays a one-time
decompression startup cost to begin reading, after which skipping costs the same as in the
equivalent BEN stream. If you'll subsample an XBEN file repeatedly, extract it to BEN first
with [`decode_xben_to_ben`](convert-formats.md).
```
