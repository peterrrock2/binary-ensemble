# Formats: BEN vs XBEN vs BENDL

`binary-ensemble` has three on-disk **containers**. They share the same underlying encoding;
they differ in how much extra compression and packaging they add.

## `.ben` — the working format

A plain BEN **stream**: a one-line banner followed by the bit-packed, run-length-encoded
frames. This is the format you *work* with — it supports reading any sample, replaying an
ensemble, and [subsampling](../how-to/subsample.md) without decompressing everything.

- **Fast** to write and read.
- Already much smaller than JSONL (the Colorado example: 27 GB → ~550 MB).
- The format the `BenEncoder` / `BenDecoder` stream classes produce and consume.

## `.xben` — the storage format

A BEN stream wrapped in [LZMA2](https://en.wikipedia.org/wiki/Lempel%E2%80%93Ziv%E2%80%93Markov_chain_algorithm).
LZMA2 exploits the repetition *across* plans that bit-packing alone can't reach, taking the
Colorado example from ~550 MB down to ~6 MB.

```{admonition} XBEN is for storage and transfer, not active work
:class: important
Decompression is fast (a large file extracts in a few minutes), but **compression is slow** —
high-ratio XBEN encoding of a block-level ensemble can take an hour or more. Encode to XBEN
once for archival or sharing; do your day-to-day reading against a BEN stream.
```

## `.bendl` — the bundle (recommended)

A **bundle** packages a BEN or XBEN assignment stream together with its assets in a single
self-describing file:

- the **dual graph** (`graph.json`), so the node order travels with the data;
- a **node permutation map** (`node_permutation_map.json`), if the graph was reordered;
- **metadata** (`metadata.json`) — seeds, sampler settings, anything you want;
- arbitrary **custom assets** you attach.

Because the graph is embedded, a collaborator can open a `.bendl` and immediately reconstruct
plans — no separate graph file to track down, no chance of pairing the wrong one. This is why
the bundle is the recommended default.

A bundle can wrap *either* a BEN stream (the working form) or an XBEN stream (the compressed
form). You typically build a BEN bundle while sampling, then
[recompress it to XBEN](../how-to/shrink-for-sharing.md) for distribution.

## Choosing a format

| If you want to… | Use |
|---|---|
| Hand an ensemble to a collaborator as one file | `.bendl` (XBEN inside) |
| Keep building / reading an ensemble locally | `.bendl` (BEN inside) or `.ben` |
| Archive an ensemble as small as possible | `.xben`, or a `.bendl` recompressed to XBEN |
| Interoperate with the JSONL world | convert with the [codec helpers](../how-to/convert-formats.md) |

```{tip}
When in doubt, use a `.bendl` bundle. You only need the plain `.ben`/`.xben` stream classes
when you specifically don't want the bundle packaging — for example, feeding a raw stream to
another tool that expects it.
```

## Going deeper

The exact byte layouts are documented in the format specifications, for readers building
interoperating tools:

- [BEN / XBEN stream format](https://github.com/peterrrock2/binary-ensemble/blob/main/docs/ben-format-spec.md)
- [TwoDelta variant format](https://github.com/peterrrock2/binary-ensemble/blob/main/docs/twodelta-format-spec.md)
- [BENDL bundle format](https://github.com/peterrrock2/binary-ensemble/blob/main/docs/bendl-format-spec.md)
