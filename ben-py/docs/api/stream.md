# `binary_ensemble.stream`

The stream module is the low-level API for plain `.ben` and `.xben` files. A plain stream
contains assignments only; it does not carry a graph, metadata, or custom assets. Prefer
{mod}`binary_ensemble.bundle` unless some other tool specifically needs raw stream files.

## Stream vs. bundle

| Need | Use |
|---|---|
| Self-describing file with graph and metadata | `BendlEncoder` / `BendlDecoder` |
| Small raw stream for another tool | `BenEncoder` / `BenDecoder` |
| Whole-file JSONL conversion | {mod}`binary_ensemble.codec` |

```{eval-rst}
.. automodule:: binary_ensemble.stream
```

## Encoder

`BenEncoder` writes `.ben` streams. It does not write `.xben` directly; encode to BEN first,
then call {func}`binary_ensemble.codec.encode_ben_to_xben` for archival compression.

```python
from binary_ensemble import BenEncoder

with BenEncoder("api-chain.ben", overwrite=True, variant="twodelta") as encoder:
    encoder.write([1, 1, 2, 2])
    encoder.write([1, 2, 2, 2])
```

Variant choices are documented in [Encoding variants](../concepts/variants.md). Decoders
auto-detect the variant from the stream banner, so you only choose it when encoding.

```{eval-rst}
.. autoclass:: binary_ensemble.stream.BenEncoder
   :members:
```

## Decoder

`BenDecoder` iterates plain `.ben` or `.xben` streams. Use `mode="xben"` for XBEN:

```python
from binary_ensemble import BenDecoder

ben_decoder = BenDecoder("chain.ben")
xben_decoder = BenDecoder("chain.xben", mode="xben")

assert len(ben_decoder) == len(xben_decoder)
```

The same subsampling methods available on bundles are available here:

```python
from binary_ensemble import BenDecoder

for assignment in BenDecoder("chain.ben").subsample_every(25):
    print(assignment[:4])
```

Plain BEN is the fastest format for repeated reads and subsampling. XBEN is smaller, but it
pays a decompression startup cost.

```{eval-rst}
.. autoclass:: binary_ensemble.stream.BenDecoder
   :members:
```
