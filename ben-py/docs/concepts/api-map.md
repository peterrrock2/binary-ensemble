# The API map

The Python API is deliberately split into four modules that mirror the project's CLI tools.
Knowing which module owns which job makes the whole surface easy to navigate.

| Module | Mirrors CLI | Owns |
|---|---|---|
| {mod}`binary_ensemble.bundle` | `bendl` | Creating, reading, and transforming `.bendl` files |
| {mod}`binary_ensemble.stream` | `ben` | Reading and writing plain `.ben`/`.xben` streams |
| {mod}`binary_ensemble.codec` | `ben` (encode/decode modes) | Whole-file conversions between JSONL, BEN, and XBEN |
| {mod}`binary_ensemble.graph` | `reben` (orderings) | Reordering a dual graph before encoding |

Everything is also re-exported at the top level, so `from binary_ensemble import BendlEncoder`
works.

## Lead with `bundle`

For most work, reach for {mod}`binary_ensemble.bundle` first:

- **`BendlEncoder`** — write a bundle: attach a graph and metadata, then stream assignments.
- **`BendlDecoder`** — read a bundle: iterate assignments, recover the graph and metadata.
- **`compress_stream`** — recompress a bundle's BEN stream to XBEN, keeping every asset.
- **`relabel_bundle`** — reorder a bundle's graph and rewrite its stream to match.

A bundle keeps the assignment stream and its dual graph together, which is what you want the
vast majority of the time.

## When to use the others

**`stream`** — when you specifically *don't* want a bundle: a raw `.ben`/`.xben` stream with
no embedded graph. `BenEncoder` writes one; `BenDecoder` reads and
[subsamples](../quick-help/subsample.md) one. Note that the bundle decoder supports the same
subsampling methods, so you rarely need to drop down to the stream classes just for that.

**`codec`** — when you have whole files to convert and don't need sample-by-sample access:
`encode_jsonl_to_ben`, `encode_ben_to_xben`, `decode_ben_to_jsonl`, and friends transform an
entire file in one call. See [Convert between formats](../quick-help/convert-formats.md).

**`graph`** — when you want to reorder a dual graph yourself (to inspect the permutation, or
to reorder before running a sampler) rather than letting `BendlEncoder.add_graph` do it
inline. See [Why reordering shrinks files](compression.md).

```{admonition} `_core` is an implementation detail
:class: note
You may notice a `binary_ensemble._core` module — the compiled extension. Always import from
the public modules above (or the top level); `_core` is internal and unsupported for direct
use.
```
