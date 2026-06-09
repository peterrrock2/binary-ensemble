# `binary_ensemble.codec`

The codec module contains whole-file transforms. These functions do not expose an iterator:
they read one file and write another.

Use them for conversion jobs. Use {mod}`binary_ensemble.stream` for sample-by-sample access
to plain streams, and {mod}`binary_ensemble.bundle` when graph and metadata should stay with
the assignments.

## Inputs and outputs

| Function family | Input | Output | Carries assets? |
|---|---|---|---:|
| `encode_jsonl_to_*` | JSON Lines with an `assignment` field | BEN or XBEN stream | no |
| `encode_ben_to_xben` | BEN stream | XBEN stream | no |
| `decode_*_to_jsonl` | BEN or XBEN stream | JSON Lines | no |
| `decode_xben_to_ben` | XBEN stream | BEN stream | no |

The expected JSONL shape is:

```json
{"assignment": [1, 1, 2, 2], "sample": 1}
{"assignment": [1, 2, 2, 2], "sample": 2}
```

Only the `assignment` values are encoded into the stream. Store graph data, sampler
settings, scores, and provenance in a `.bendl` bundle if they need to travel with the file.

```{eval-rst}
.. automodule:: binary_ensemble.codec
```

## Encoders

```python
from binary_ensemble import encode_ben_to_xben, encode_jsonl_to_ben, encode_jsonl_to_xben

encode_jsonl_to_ben("plans.jsonl", "api-plans.ben", overwrite=True)
encode_ben_to_xben("api-plans.ben", "api-plans.xben", overwrite=True)

encode_jsonl_to_xben(
    "plans.jsonl",
    "api-direct.xben",
    overwrite=True,
    variant="twodelta",
    compression_level=9,
)
```

`variant=` is only used when creating BEN frames from assignments. XBEN-specific knobs
(`n_threads`, `compression_level`, and `xz_block_size`) tune the LZMA2 stage.

```{eval-rst}
.. autofunction:: binary_ensemble.codec.encode_jsonl_to_ben

.. autofunction:: binary_ensemble.codec.encode_jsonl_to_xben

.. autofunction:: binary_ensemble.codec.encode_ben_to_xben
```

## Decoders

```python
from binary_ensemble import decode_ben_to_jsonl, decode_xben_to_ben, decode_xben_to_jsonl

decode_ben_to_jsonl("chain.ben", "api-chain.jsonl", overwrite=True)
decode_xben_to_ben("chain.xben", "api-chain.ben", overwrite=True)
decode_xben_to_jsonl("chain.xben", "api-chain-from-xben.jsonl", overwrite=True)
```

Decoding auto-detects the stream variant from the file; you never pass `variant=` when
reading.

```{eval-rst}
.. autofunction:: binary_ensemble.codec.decode_ben_to_jsonl

.. autofunction:: binary_ensemble.codec.decode_xben_to_jsonl

.. autofunction:: binary_ensemble.codec.decode_xben_to_ben
```
