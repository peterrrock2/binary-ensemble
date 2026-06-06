# Subsample a large ensemble

When an ensemble has millions of plans, you often want only a slice — every 1000th plan, a
contiguous range, or a handful of specific indices. The decoders support this directly, and
they do it by **skipping** frames rather than decoding everything, so it stays fast.

All three methods are available on both `BendlDecoder` (for bundles) and `BenDecoder` (for
plain streams). Each returns a decoder you iterate.

## By specific indices

```python
from binary_ensemble import BendlDecoder

for assignment in BendlDecoder("ensemble.bendl").subsample_indices([1, 50, 100]):
    print(assignment[:10])
```

## By a contiguous range

```python
for assignment in BendlDecoder("ensemble.bendl").subsample_range(10, 15):
    print(assignment[:10])
```

## By a fixed stride

`subsample_every(step)` yields every `step`-th sample (with an optional `offset`):

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
Subsampling a BEN stream is fastest because frames can be skipped without decompressing. An
XBEN stream pays a one-time startup cost to begin reading, after which skipping is cheap
again. If you'll subsample an XBEN file repeatedly, extract it to BEN first with
[`decode_xben_to_ben`](convert-formats.md).
```
