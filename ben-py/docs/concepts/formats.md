# Formats: BEN vs XBEN vs BENDL

`binary-ensemble` has three on-disk **containers**. They share the same underlying encoding;
they differ in how much extra compression and packaging they add.

## `.ben` — the working format

A plain BEN **stream**: a one-line banner followed by the bit-packed, run-length-encoded
frames. This is the format you _work_ with — it supports reading any sample, replaying an
ensemble, and [subsampling](../quick-help/subsample.md) without decompressing everything.

- **Fast** to write and read.
- Already much smaller than JSONL (the Colorado example: 13.5 GB → ~280 MB).
- The format the `BenEncoder` / `BenDecoder` stream classes produce and consume.

## `.xben` — the storage format

A BEN stream wrapped in [LZMA2](https://en.wikipedia.org/wiki/Lempel%E2%80%93Ziv%E2%80%93Markov_chain_algorithm).
LZMA2 exploits the repetition _across_ plans that bit-packing alone can't reach, taking the
Colorado example from ~280 MB down to 5.6 MB.

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

A bundle can wrap _either_ a BEN stream (the working form) or an XBEN stream (the compressed
form). You typically build a `.bendl` file with a BEN stream while sampling, then
[recompress it to XBEN](../quick-help/shrink-for-sharing.md) for distribution.

## Choosing a format

| If you want to…                                | Use                                                                |
| ---------------------------------------------- | ------------------------------------------------------------------ |
| Hand an ensemble to a collaborator as one file | `.bendl` (XBEN inside)                                             |
| Keep building / reading an ensemble locally    | `.bendl` (BEN inside) or `.ben`                                    |
| Archive an ensemble as small as possible       | `.xben`, or a `.bendl` recompressed to XBEN                        |
| Interoperate with the JSONL world              | convert with the [codec helpers](../quick-help/convert-formats.md) |

```{tip}
When in doubt, use a `.bendl` file. You only need the plain `.ben`/`.xben` stream classes
when you specifically don't want the bundle packaging — for example, feeding a raw stream to
another tool that expects it.
```

## How each format works

The three containers build on each other: BEN defines the encoding, XBEN compresses a BEN
stream, and BENDL packages a BEN or XBEN stream with its assets. Here's the mechanism behind
each.

### BEN: a layered encoding of one assignment at a time

A BEN stream encodes each sample (one district id per node, read in node order) through four
stacked layers:

1. **Run-length encoding (RLE).** Consecutive nodes in the same district collapse to
   `(district, length)` pairs — `[1, 1, 1, 2, 2, 2, 2, 3]` becomes `[(1, 3), (2, 4), (3, 1)]`.
   Fewer, longer runs mean a smaller frame, which is exactly why
   [node reordering](compression.md) is the biggest compression lever.
2. **Bit-packing.** Each frame inspects its own largest district id and largest run length, then
   packs every value and length to _exactly_ that many bits — no wasted bytes. The example above
   has a max id of `3` (2 bits) and a max length of `4` (3 bits), so each run costs 5 bits.
3. **Frames.** Each sample becomes one self-describing frame: a short header (the two bit-widths
   plus the payload's byte length) followed by the packed payload. For `standard` and `mkv_chain`,
   that length lets a reader **skip** a sample it doesn't want without unpacking a single payload
   bit. `twodelta` frames can still be skipped at byte level while scanning, but selected samples
   must be reconstructed by replaying from the latest snapshot checkpoint.
4. **Stream.** A 17-byte banner that names the [variant](variants.md), then the frames written
   back-to-back. There's no global index or end marker, so frames can be appended one at a time
   while sampling and read back until the input simply runs out.

The variant changes the **frame shape** — independent snapshots (`standard`), snapshots plus repeat
counts (`mkv_chain`), or deltas against the previous sample with periodic snapshots (`twodelta`).
All three ride on the same RLE and bit-packing layers underneath.

### XBEN: LZMA2 over a byte-aligned rewrite

XBEN is a BEN stream run through [LZMA2](https://en.wikipedia.org/wiki/Lempel%E2%80%93Ziv%E2%80%93Markov_chain_algorithm)
(the `xz` algorithm) — but **not** over the bit-packed frames directly. Bit-packing makes each plan
small, but it pushes runs across byte boundaries (a run can start mid-byte), so identical patterns
in different plans don't line up as identical bytes — and a byte-oriented compressor can't see the
repetition.

So XBEN first re-expands the stream into an intermediate columnar form where each run is a
fixed-width, byte-aligned `(value, length)` pair and each frame ends with a zero sentinel. Now
equivalent runs across plans line up **byte-for-byte**, and LZMA2 — which hunts for repeated byte
sequences — collapses the redundancy across the whole ensemble. This is also why
[district relabeling](compression.md) pays off: it makes structurally identical plans encode to
identical bytes, which LZMA2 then deduplicates.

```{admonition} Asymmetric cost
:class: note
Decompressing XBEN is fast (minutes, even for large files), but high-ratio LZMA2 *compression* is
slow — block-level ensembles can take an hour. Encode to XBEN once for archival or transfer; do
day-to-day reading against a BEN stream.
```

### BENDL: a self-describing, crash-recoverable bundle

A bundle is a single seekable file laid out as a fixed-size header, then the asset payloads, then
the embedded assignment stream, then a directory table at the end:

- The **header** records which stream format is embedded (BEN or XBEN), where the stream lives, the
  expanded **sample count** (so counting an ensemble is an O(1) header read, not a full scan), and a
  CRC32C checksum over the stream bytes.
- The **directory table** indexes every asset — the dual graph, the node permutation map, the
  metadata, and any custom assets — by offset and length, each with its own CRC32C. A reader can
  pull out just the graph without scanning the file, and verify it before trusting it
  (`BendlDecoder.verify()` checks every asset and the stream in one call). Large asset payloads
  are xz-compressed on disk by the writer and decompressed transparently on read.
- The **assignment stream** is stored opaquely: the bundle never parses BEN/XBEN internals, it just
  carries the bytes and notes the format. That's what lets you replace the embedded BEN stream
  with an embedded XBEN stream by recompressing only the inner stream.

The writer lays the file down in order — a provisional header marked _unfinalized_, then assets,
then the stream, then the directory — and **patches the header last** to flip it to finalized and
fill in the final lengths, checksum, and sample count. So if the process dies mid-write, the
partial file is clearly flagged incomplete and the stream bytes that reached disk are still
salvageable — see
[Recovering samples from a crashed run](../quick-help/troubleshooting.md#recovering-samples-from-a-crashed-run);
that final header patch is the single commit point.

## Going deeper

The exact byte layouts are documented in the format specifications, for readers building
interoperating tools:

- [BEN / XBEN stream format](https://github.com/peterrrock2/binary-ensemble/blob/1.0.0/docs/ben-format-spec.md)
- [TwoDelta variant format](https://github.com/peterrrock2/binary-ensemble/blob/1.0.0/docs/twodelta-format-spec.md)
- [BENDL bundle format](https://github.com/peterrrock2/binary-ensemble/blob/1.0.0/docs/bendl-format-spec.md)
