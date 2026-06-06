# Encoding variants

A BEN stream is encoded with one of three **variants**. The variant controls how individual
plans (frames) are stored relative to each other; it's fixed for the whole stream when you
encode, and **decoding auto-detects it**, so you never pass a variant when reading a file
back.

You choose a variant with the `variant=` argument on the encoders and the
`encode_jsonl_to_*` helpers.

## `standard`

Each plan is stored independently — RLE + bit-packing, nothing more. It's the simplest
encoding and the baseline. For ensembles with no repetition, its output is very slightly
smaller than `mkv_chain`; for chains with repeats, the other variants win comfortably.

- **Good for:** any ensemble; a safe baseline.

## `mkv_chain`

Like `standard`, but identical consecutive plans are collapsed into a single frame carrying a
repetition count. This is built for **MCMC chains logged in full** — including self-loops,
where a proposal was rejected and the same plan repeats (as in
[Reversible ReCom](https://mggg.org/rrc)).

- **Good for:** full-chain MCMC ensembles where rejections produce repeated plans.

## `twodelta`

The **default**, and usually the best general-purpose choice. It delta-encodes **pairwise
ReCom steps**: when two consecutive plans differ by exactly one recombination move (two
districts swap some nodes, nothing else changes), only the difference is stored. Any other
transition — a multi-district move, independent/random sampling, a newly created district —
is stored as a full snapshot frame instead, and identical consecutive plans are handled with
repetition counts.

Because it falls back to snapshots, `twodelta` is **compatible with every sampler**; non-ReCom
ensembles just produce more snapshot frames and less delta savings. Its best-case compression
comes from a full-chain *pairwise* ReCom ensemble, where nearly every accepted move changes
exactly two districts.

- **Good for:** ReCom chains (best case) and as a robust default for anything else.

## Choosing a variant

| Sampler / data shape | Recommended variant |
|---|---|
| Pairwise ReCom chain | `twodelta` (default) |
| Full MCMC chain with many rejections/repeats | `mkv_chain` |
| Independent / random sampling, ForestReCom, mixed | `twodelta` or `standard` |
| Not sure | `twodelta` (the default) |

```{admonition} You don't decode by variant
:class: note
The variant is recorded in the stream's banner, so readers detect it automatically. The only
place you specify a variant is when **encoding**.
```
