# Encoding variants

A BEN stream is encoded with one of three **variants**. The variant controls how individual
plans (frames) are stored relative to each other; it's fixed for the whole stream when you
encode, and **decoding auto-detects it**, so you never pass a variant when reading a file
back.

You choose a variant with the `variant=` argument on the encoders and the
`encode_jsonl_to_*` helpers.

All three sit on the same foundation — run-length encoding and bit-packing into frames, behind
a banner (see [how BEN works](formats.md#how-each-format-works)). What differs is the **frame
shape**: whether a frame stands alone, carries a repeat count, or is a difference against the
plan before it. That choice also decides whether you can [subsample](../how-to/subsample.md) by
skipping frames or have to replay them.

## `standard`

Each plan is stored independently — RLE + bit-packing, nothing more. One sample is one frame: a
small header (the bit-widths and the payload byte length) followed by the packed runs. Nothing in
a frame refers to any other frame.

Because every frame is self-contained and its byte length is in the header, a reader can **skip
straight over** frames it doesn't want without unpacking them — so random access and subsampling
are cheap. For ensembles with no repetition its output is very slightly smaller than `mkv_chain`;
for chains with repeats, the other variants win comfortably.

- **Good for:** any ensemble; a safe baseline.

## `mkv_chain`

Like `standard`, but each frame also carries a **repetition count**. A run of identical
consecutive plans collapses into one frame plus a count of *N*, which the reader expands back into
*N* samples — so the stored sample count is preserved while the bytes aren't. This is built for
**MCMC chains logged in full**, where a rejected proposal leaves the same plan repeated on
consecutive steps (self-loops, as in [Reversible ReCom](https://mggg.org/rrc)).

Frames are still independently decodable, so `mkv_chain` keeps `standard`'s cheap frame-skip
subsampling. It only beats `standard` when consecutive plans actually repeat; with no repeats the
extra count field makes it marginally larger.

- **Good for:** full-chain MCMC ensembles where rejections produce repeated plans.

## `twodelta`

The **default**, and usually the best general-purpose choice. Instead of storing every plan in
full, it stores most plans as the **difference** from the one before.

The first sample is a full snapshot — the **anchor**. For each later sample, the encoder looks at
how it changed from the previous one and picks a frame type:

- **Delta frame** — the change is one clean **pairwise** recombination: exactly two districts swap
  some nodes (both district ids already exist, nothing else moves). The frame stores just those two
  ids and where they now sit, applied on top of the previous plan. This is the case that makes
  `twodelta` small.
- **Repeat** — the plan is unchanged, handled with a repetition count like `mkv_chain`.
- **Snapshot frame** — anything else (a multi-district move, a brand-new district id, independent
  or random sampling) is stored as a full plan, which also becomes the new anchor for the deltas
  that follow.

Because it always falls back to snapshots, `twodelta` is **compatible with every sampler** —
non-ReCom ensembles just produce more snapshots and less delta savings. Its best case is a
full-chain *pairwise* ReCom ensemble, where nearly every accepted move is a two-district swap.

The trade-off: a delta frame only makes sense relative to the plan before it, so a reader
reconstructs a sample by **replaying forward from the most recent snapshot**. That means
`twodelta` gives up the cheap frame-skip subsampling that `standard` and `mkv_chain` allow — random
access costs a short replay.

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
