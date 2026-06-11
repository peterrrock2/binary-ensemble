# TwoDelta BEN Format Specification

## Status

Stable wire format. This document specifies the on-disk byte layout of the **TwoDelta** variant of a
BEN stream, for both the plain `.ben` container and the columnar body carried inside a `.xben`
container. The variant is pinned by the committed v1.0.0 stability fixtures (`twodelta.ben`,
`twodelta.xben`).

TwoDelta shares the banner mechanism, run-length encoding, and bit-packing convention of the
**Standard** and **MkvChain** variants; those shared layers are specified in the BEN stream format
specification and are only summarized here. TwoDelta differs in that most frames are **deltas**
against the previous sample rather than independent snapshots, which gives it a different frame
layout and a different `.xben` body.

The `.bendl` bundle container embeds a BEN/XBEN stream as an opaque payload and is unaffected by the
variant; see the BENDL format specification.

## Design Goals

- Encode a full-chain ensemble from a *pairwise* ReCom sampler compactly by storing, for most
  samples, only the two district ids that changed and where.
- Remain a valid BEN stream: same banner mechanism, same `.ben`/`.xben` containers, same expanded
  sample-count semantics.
- Degrade gracefully: any transition that is not a clean two-district swap is stored as a full
  snapshot, so the variant never fails to represent a sample — it only loses compression on that
  step.

## Terminology

This document uses the workspace glossary. The terms that matter most here:

- **assignment** — a length-N `Vec<u16>` where index *i* is the district id of dual-graph node *i*.
- **ReCom-step** — a single accepted ReCom move: consecutive samples differ by exactly one pairwise
  district swap (exactly two district ids exchange positions; no position outside that pair
  changes).
- **anchor** — the first sample of the stream, stored as a full snapshot. Every later delta is
  reconstructed by replaying from the most recent snapshot.
- **delta frame** — a frame that stores a pair of district ids and the alternating run lengths over
  the positions those ids occupy, to be applied to the previous assignment.
- **snapshot frame** — a frame that stores a full assignment, used for the anchor and as the
  fallback when a transition is not a clean two-district swap.
- **inter-sample constraint** — the rule TwoDelta imposes: a delta frame is only emitted for a clean
  two-district swap where both ids are already present; everything else falls back to a snapshot.

The encoding stack layers (bit-packing, RLE, frame, stream, container) are as defined in the
glossary and the BEN stream spec.

## Byte Order

All multi-byte integers — the frame-header fields, the trailing repetition counts, and every field
of the columnar `.xben` body — are **big-endian**. (As with the other variants, this is independent
of the little-endian BENDL bundle header.)

## Inter-Sample Constraint and Frame Selection

For each sample after the anchor, the encoder classifies the transition from the previous sample in
a single scan:

- **Repeat** — no position changed value. The sample repeats the previous one. Collapsed into a
  repetition count, or emitted as a no-op delta frame (see **Repeats**).
- **Delta** — every changed position swaps between exactly the same two district ids A and B, and
  **both** A and B already appear in the previous assignment. Emitted as a delta frame.
- **Snapshot** — more than two distinct district ids change, **or** the transition introduces a
  district id that was absent from the previous assignment (so there is no prior layout to delta
  against). Emitted as a snapshot frame, which also re-establishes the anchor for subsequent deltas.

The first frame of any TwoDelta stream is always a snapshot (the anchor). A decoder MUST have a
reconstructed assignment in hand before it can apply a delta; a stream whose first frame is a delta
is malformed.

Because a delta frame is reconstructed from the previous assignment, TwoDelta frames are **not**
independently decodable: random access or subsampling requires replaying from the most recent
snapshot forward. This is the deliberate trade-off the delta encoding makes against the cheap
frame-skip subsampling that Standard and MkvChain allow.

## Delta Semantics

A delta frame stores:

- a **pair** `(A, B)` of district ids, and
- a vector of **alternating run lengths** over the positions the pair occupies in the *new*
  assignment, ordered by position.

The run lengths describe, in node order and restricted to positions holding A or B, the lengths of
maximal runs of one id then the other: the first run is id A, the second is id B, the third is A
again, and so on. The pair is ordered so that **A is whichever id occupies the lowest-indexed
position held by either id in the new assignment**. This makes the first run length at least `1`
(there is no leading zero), and it is the round-trip-determinism invariant the decoder relies on: a
pair ordered the other way would silently decode to a different assignment.

To decode a delta against the previous assignment: walk the assignment in node order; at each
position that currently holds A or B, overwrite it with the active run's id, consuming run lengths
in order and flipping A↔B at each run boundary. Positions holding any other id are left untouched. A
decoder MUST error if the run lengths are exhausted before all pair positions are covered, or if run
length remains after the assignment ends.

### Worked example

Previous assignment `[1, 1, 2, 2]`, new assignment `[1, 2, 2, 2]` (the node at index 1 moves from
district 1 to district 2).

- Changed ids are `{1, 2}`, both already present → a delta. The lowest pair position in the new
  assignment is index 0, which holds `1`, so the pair is `(1, 2)`.
- Restricted to pair positions (all four here), the new assignment reads `1, 2, 2, 2` → run lengths
  `[1, 3]`.
- Decoding `[1, 1, 2, 2]` with pair `(1, 2)` and runs `[1, 3]`: position 0 stays `1` (run of 1),
  positions 1–3 become `2` (run of 3) → `[1, 2, 2, 2]`. ✓

## Plain `.ben` Layout

A TwoDelta `.ben` stream is:

```text
[17-byte Banner: "TWODELTA BEN FILE"]
[Tagged Frame 1]   (snapshot — the anchor)
[Tagged Frame 2]
...
```

Unlike Standard and MkvChain, every frame is prefixed with a 1-byte **frame tag** that selects the
body layout:

```text
[tag: u8][frame body]
```

- `0x00` — **snapshot frame**. The body is byte-for-byte an MkvChain frame (see the BEN stream
  spec):

  ```text
  offset      size  field
  0           1     max_val_bit_count
  1           1     max_len_bit_count
  2           4     n_bytes
  6           ...   payload (n_bytes bytes, bit-packed RLE of the full assignment)
  6+n_bytes   2     count
  ```

- `0x01` — **delta frame**. The body is:

  ```text
  offset      size  field
  0           2     pair_a            (district id A)
  2           2     pair_b            (district id B)
  4           1     max_len_bit_count
  5           4     n_bytes
  9           ...   payload (n_bytes bytes, bit-packed run lengths)
  9+n_bytes   2     count
  ```

The delta frame header is 9 bytes (the stream-layer tag is not part of the frame header). The
payload bit-packs the alternating run lengths, each in `max_len_bit_count` bits,
most-significant-bit first, with the final byte zero-padded on the low side — the same bit-packing
convention the BEN stream spec defines for run lengths, except only lengths are packed (the pair
lives in the header, and there are no per-run values). `max_len_bit_count` is in `1..=16`; a decoder
MUST reject `0` or a value above `16`. All stored run lengths are `>= 1`; any zero decoded from the
padding tail is discarded.

The stream ends at a clean end-of-input on the **tag boundary**. End-of-input after a tag byte but
before a complete body is a truncated frame and MUST error.

### Worked example (continued)

The delta from the example above, with `max_len = 3` → `max_len_bit_count = 2`, runs `[1, 3]` packed
as `01 11` → `0x70`, `n_bytes = 1`, `count = 1`:

```text
01  00 01  00 02  02  00 00 00 01  70  00 01
^tag ^A    ^B     ^bits ^n_bytes   ^pl  ^count
```

## XBEN Layout (Columnar Body)

A TwoDelta `.xben` file wraps a columnar body in LZMA2. The body is **not** the per-frame tagged
layout above, and it is **not** the BEN32 intermediate used by Standard/MkvChain. It is a distinct
columnar layout that batches deltas for better compression:

```text
[17-byte Banner: "TWODELTA BEN FILE"]   (inside the LZMA2 payload)
[Body Frame 1]   (full frame — the anchor)
[Body Frame 2]
...
```

Each body frame is discriminated by its first byte:

- `0x00` — **full frame** (snapshot/anchor):

  ```text
  [0x00]
  [run_count: u32]
  [ run_count × ( value: u16, length: u16 ) ]     full-assignment RLE runs
  [count: u16]
  ```

  Unlike the BEN32 frames of the other variants, a full frame is **length-prefixed** by `run_count`
  rather than terminated by a zero sentinel.

- `0x02` — **chunk frame**: a columnar batch of `n` delta frames. Fields are stored column-by-column
  across all `n` frames:

  ```text
  [0x02]
  [n: u32]                                  number of delta frames in the chunk
  [ n × ( pair_a: u16, pair_b: u16 ) ]      pairs column
  [ n × ( count: u16 ) ]                    counts column
  [ n × ( run_count: u32 ) ]                per-frame run-length count column
  [ run_data: u16 × (sum of run_counts) ]   all frames' run lengths, concatenated in frame order
  ```

  The run lengths here are stored as plain `u16` values, not bit-packed.

Tag value `0x01` is not used in the `.xben` body. The first body frame MUST be a full frame; a chunk
before any full frame has no anchor to delta against and is malformed. A chunk's deltas precede any
following full frame, so replaying body frames in order reconstructs the samples in order. The
default batch size is 10000 delta frames per chunk; the batch size affects only compression and
framing, never the decoded result.

## Repeats

A repeated sample (identical to the previous one) is represented in one of two ways, both of which
preserve the *expanded* sample count:

- via a frame **`count` greater than 1**, which expands to that many identical samples (as in
  MkvChain); or
- via a **no-op delta frame** whose run lengths reproduce the previous assignment unchanged. This
  arises when a repeat must be emitted as its own frame rather than merged into a neighbor's count.

As with the other variants, a frame `count` of `0` is invalid and MUST be rejected by readers.

## Run-Length Representability

Run lengths in delta-shaped frames (deltas, chunks, and no-op repeat deltas) are `u16` and MUST be
greater than zero, so a pair-projected run longer than `65535` positions cannot be expressed in a
delta-shaped frame: splitting it would require interleaving zero-length runs, which readers reject
as corruption. A writer that encounters such a run MUST fall back to a snapshot (plain `.ben`) or
full (`.xben`) frame, whose RLE layer splits long runs into consecutive maximal runs natively.
Readers need no special handling — the fallback arrives as an ordinary snapshot/full frame.

## Reader Rules

A reader MUST:

1. Read and validate the 17-byte `TWODELTA BEN FILE` banner.
1. For a plain `.ben` stream: read frames as `[tag][body]`, dispatching `0x00` to the snapshot body
   and `0x01` to the delta body; reject any other tag. End the stream on a clean EOF at a tag
   boundary; treat an EOF inside a body as truncation.
1. For an `.xben` body: after LZMA2 decompression and the banner, read body frames by leading byte,
   dispatching `0x00` to the full layout and `0x02` to the chunk layout; reject any other tag.
1. Maintain the previous assignment. Reconstruct snapshot/full frames directly; apply delta/chunk
   frames against the previous assignment. Error if a delta is encountered before any anchor.
1. Treat each frame's `count` as its expanded sample multiplicity (reject `count == 0`), and add it
   to the expanded sample count.

## Relationship to the Other Variants

- A TwoDelta **snapshot** frame body is byte-identical to an MkvChain frame; the only stream-level
  difference is the leading tag byte. The anchor and every fallback snapshot reuse that layout.
- TwoDelta is the only variant whose frames depend on prior frames, and the only one whose `.xben`
  body is columnar rather than BEN32. Converting a TwoDelta stream to Standard or MkvChain requires
  replaying the deltas into full assignments; converting the other direction requires a pairwise
  ReCom sampler's transition structure and is only valid when every accepted move changes exactly
  two districts.
- TwoDelta is **not compatible** with random sampling (consecutive samples can differ in arbitrarily
  many ids) or with Forest ReCom (a single move can touch more than two districts). For those, use
  Standard or MkvChain.

## Versioning Strategy

The frame tags, the snapshot/delta/full/chunk layouts, the bit-packing convention, and the pair
ordering invariant are contractual: committed fixtures encoded under a stable major version MUST
continue to decode in every later release of that major version. Any change to a layout or to the
pair-ordering rule is a breaking change requiring a new fixture set under a new major version;
existing fixtures are never regenerated in place. See the format-stability policy.

## Out of Scope

- The Standard and MkvChain variants (independent snapshot frames; BEN32 `.xben` body).
- The `.bendl` bundle container that embeds a BEN/XBEN stream as an opaque payload.
- LZMA2 framing details; XBEN treats LZMA2 as an opaque outer wrapper around the columnar body.
