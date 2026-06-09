# Python and CLI parity

The Python package wraps the same Rust engine as the command-line tools. The API names mirror
the CLI split so workflows can move between notebooks, scripts, and shell pipelines.

## Command map

| CLI task | Python equivalent | Notes |
|---|---|---|
| Encode JSONL to BEN | `encode_jsonl_to_ben(...)` | Whole-file conversion |
| Encode JSONL to XBEN | `encode_jsonl_to_xben(...)` | Whole-file conversion plus XBEN compression |
| Convert BEN to XBEN | `encode_ben_to_xben(...)` | Plain stream only |
| Decode BEN to JSONL | `decode_ben_to_jsonl(...)` | Plain stream only |
| Decode XBEN to BEN | `decode_xben_to_ben(...)` | Useful before repeated subsampling |
| Decode XBEN to JSONL | `decode_xben_to_jsonl(...)` | Plain stream only |
| Create a BENDL bundle | `BendlEncoder(...)` | Recommended Python workflow |
| Inspect a BENDL bundle | `BendlDecoder(...).list_assets()` | Also exposes graph and metadata helpers |
| Extract a bundle stream | `BendlDecoder(...).extract_stream(...)` | Copies embedded BEN/XBEN stream bytes |
| Append bundle assets | `BendlEncoder.append(...)` | Asset appends only; no stream appends |
| Relabel/reorder a bundle | `relabel_bundle(...)` | Requires BEN stream plus graph |
| Recompress bundle stream | `compress_stream(...)` | BEN bundle to XBEN bundle |
| Reorder a graph | `binary_ensemble.graph.reorder(...)` | Same orderings as bundle relabeling |

## Plain stream conversion

```python
from binary_ensemble import encode_ben_to_xben, encode_jsonl_to_ben

encode_jsonl_to_ben("plans.jsonl", "cli-parity.ben", overwrite=True)
encode_ben_to_xben("cli-parity.ben", "cli-parity.xben", overwrite=True)
```

## Bundle inspection

```python
from binary_ensemble import BendlDecoder

decoder = BendlDecoder("ensemble.bendl")

print(decoder.assignment_format())
print(decoder.count_samples())
print(decoder.list_assets())
```

## Bundle transform

```python
from binary_ensemble import compress_stream, relabel_bundle

relabel_bundle("ensemble.bendl", out_file="cli-parity-sorted.bendl", sort="mlc")
compress_stream("cli-parity-sorted.bendl", out_file="cli-parity-archive.bendl")
```

## Choosing shell or Python

Use the CLI for shell pipelines and batch conversions. Use Python when the assignment stream
is produced inside Python, when you need to attach structured metadata, or when downstream
analysis is also in Python.
