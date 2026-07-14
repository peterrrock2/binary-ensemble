# Convert between formats

The {mod}`binary_ensemble.codec` helpers transform whole files in a single call — no
iteration, no decoder objects. Use them when you have a complete file to convert and don't
need sample-by-sample access.

The expected JSONL shape is one plan per line:

```
{"assignment": [1, 1, 2, 2, ...], "sample": 1}
{"assignment": [1, 2, 2, 2, ...], "sample": 2}
```

## JSONL → BEN

```python
from binary_ensemble import encode_jsonl_to_ben

encode_jsonl_to_ben("plans.jsonl", "plans.ben")                    # default variant: twodelta
encode_jsonl_to_ben("plans.jsonl", "plans.ben", variant="mkv_chain", overwrite=True)
```

## BEN → XBEN (maximum compression)

```python
from binary_ensemble import encode_ben_to_xben

encode_ben_to_xben("plans.ben", "plans.xben", overwrite=True)
```

You can also go straight from JSONL to XBEN with `encode_jsonl_to_xben`. The XBEN encoders
accept tuning knobs:

```python
from binary_ensemble import encode_jsonl_to_xben

encode_jsonl_to_xben(
    "plans.jsonl",
    "plans.xben",
    overwrite=True,
    variant="twodelta",
    n_threads=8,          # parallelize across cores (default: all available)
    compression_level=9,  # 0 (fastest) … 9 (smallest); default 9
)
```

```{important}
XBEN compression is slow — high-ratio encoding of a block-level ensemble can take an hour or
more. Decompression, by contrast, is fast. Encode to XBEN once for storage; work against BEN
day to day. See [Formats](../concepts/formats.md).
```

## Decoding back out

The decoders mirror the encoders and all take `(in_file, out_file, overwrite=False)`:

```python
from binary_ensemble import decode_ben_to_jsonl, decode_xben_to_jsonl, decode_xben_to_ben

decode_ben_to_jsonl("plans.ben", "plans.jsonl", overwrite=True)     # BEN  -> JSONL
decode_xben_to_jsonl("plans.xben", "plans.jsonl", overwrite=True)   # XBEN -> JSONL
decode_xben_to_ben("plans.xben", "plans.ben", overwrite=True)       # XBEN -> BEN (to work with it)
```

```{note}
By default these refuse to overwrite an existing output file; pass `overwrite=True` to
replace it. You never specify a variant when decoding — it's detected from the stream.
```

## Working with bundles instead?

These helpers operate on plain streams and JSONL. To recompress the stream _inside_ a
`.bendl` file (keeping its graph and metadata), use
[`compress_stream`](shrink-for-sharing.md) instead. Its inverse, `decompress_stream`, changes the
embedded XBEN stream back to BEN while also preserving every bundle asset.
