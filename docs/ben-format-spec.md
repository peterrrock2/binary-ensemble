# BEN Stream Format Specification

## Status

Stable wire format. This document specifies the on-disk byte layout of a BEN stream for the two
snapshot variants, **Standard** and **MkvChain**. These two variants share every layer of the
encoding except the per-frame repetition count and the inter-sample constraint; they are documented
together because they differ by a single field on the wire.

The **TwoDelta** variant is a delta encoding with a different frame shape and a different XBEN body
layout. It is out of scope here and specified separately.

This specification covers the `.ben` container and the BEN32 body carried inside a `.xben`
container. It does not cover the `.bendl` bundle container, which embeds a BEN/XBEN stream as an
opaque payload; see the BENDL format specification for that.

## Design Goals

- A compact, self-describing encoding of an ensemble of district assignments.
- Per-frame headers that allow frame-level subsampling without unpacking payload bits.
- A repetition count (MkvChain) that collapses identical consecutive samples from full-chain
  samplers into a single frame, while preserving the expanded sample count.
- A streamable layout: frames can be appended one at a time and read back without a global index.

## Terminology

This document uses the workspace glossary. The terms that matter most here:

- **assignment** — a length-N `Vec<u16>` where index *i* is the district id of dual-graph node *i*.
- **district id** — an integer value stored in an assignment. Range `0..=65535`.
- **sample** — one `(sample_number, assignment)` pair. `sample_number` lives in *expanded* space.
- **sample count** — the *expanded* number of samples: a MkvChain frame with `count = 5` contributes
  5, not 1.
- **variant** — `Standard` or `MkvChain` here. One variant per stream, fixed by the banner.
- **banner** — the 17-byte ASCII stream identifier. Distinct from a BENDL **magic**.
- **frame header** — the leading bytes of one frame (bit-width fields and payload length).
- **frame payload** — the bit-packed bytes after the frame header.

The encoding stack is layered as in the glossary:

| Layer | Name | What it is here | |---|---|---| | 0 | bit-packing | run values and run lengths
crammed into bit-precise widths | | 1 | RLE | `(value, length)` pairs over an assignment | | 2 |
frame | one sample's bytes: frame header + payload, plus a `u16` count for `MkvChain` | | 3 | stream
| banner + concatenated frames; the contents of a `.ben` file | | 4 | container | the on-disk file:
`.ben`, or `.xben` (the stream wrapped in LZMA2) |

## Byte Order

Multi-byte integers in the frame header and the trailing count are **big-endian**. The bit-packed
payload is filled most-significant-bit first (see **Frame Payload**).

This differs from the BENDL bundle header, which is little-endian. The two formats are independent.

## Stream Layout

A BEN stream (the contents of a `.ben` file, or the LZMA2-decompressed body of a `.xben` file in its
BEN32 form) is:

```text
[17-byte Banner]
[Frame 1]
[Frame 2]
...
[Frame N]
```

There is no stream-level length prefix, frame count, or trailing terminator. The stream ends at a
frame boundary; a reader that reaches end-of-input while attempting to read the first byte of the
next frame has reached a clean end of stream.

### Banner

The first 17 bytes are an ASCII banner that fixes the variant for the entire stream:

```text
offset  size  field
0       17    banner
```

- `STANDARD BEN FILE` — Standard variant.
- `MKVCHAIN BEN FILE` — MkvChain variant.

(`TWODELTA BEN FILE` denotes the TwoDelta variant, specified elsewhere.)

A reader MUST reject a stream whose first 17 bytes are not one of the known banners.

## Run-Length Encoding (Layer 1)

Before bit-packing, an assignment is converted to a vector of `(value, length)` runs, where `value`
is a district id and `length` is the number of consecutive nodes that carry it, in node order.

Example: `[1, 1, 1, 2, 2, 2, 2, 3, 1, 3, 3, 3]` becomes `[(1, 3), (2, 4), (3, 1), (1, 1), (3, 3)]`.

Both `value` and `length` are `u16`. A run longer than `65535` is split into consecutive runs of the
same value, each at most `65535` long. The assignment length N is **not** stored anywhere in the
frame; it is recovered as the sum of all run lengths. Readers MUST reconstruct N this way and MUST
NOT assume a fixed N across frames.

## Frame Layout (Layer 2)

Both variants share the same 6-byte frame header and bit-packed payload. MkvChain appends a 2-byte
repetition count; Standard does not.

### Standard frame

```text
offset  size  field
0       1     max_val_bit_count
1       1     max_len_bit_count
2       4     n_bytes
6       ...   payload (n_bytes bytes)
```

A Standard frame is exactly `6 + n_bytes` bytes.

### MkvChain frame

```text
offset      size  field
0           1     max_val_bit_count
1           1     max_len_bit_count
2           4     n_bytes
6           ...   payload (n_bytes bytes)
6+n_bytes   2     count
```

An MkvChain frame is exactly `6 + n_bytes + 2` bytes.

### Frame header fields

- `max_val_bit_count` — the number of bits used to encode each run's district id in the payload.
  Computed as the bit width of the largest district id in this frame, with a floor of `1` (so a
  frame of all-zero district ids still uses 1 bit per value). Range `1..=16`.
- `max_len_bit_count` — the number of bits used to encode each run's length in the payload. Computed
  as the bit width of the largest run length in this frame, with a floor of `1`. Range `1..=16`.
- `n_bytes` — the exact byte length of the bit-packed payload that follows the header (`u32`,
  big-endian). Equal to `ceil((max_val_bit_count + max_len_bit_count) * n_runs / 8)`.
- `count` *(MkvChain only)* — the number of identical consecutive samples this frame represents
  (`u16`, big-endian). MUST be `>= 1`; a reader MUST treat `count == 0` as a corrupt frame and
  error. The frame's assignment is emitted `count` times, and the stream's expanded sample count
  increases by `count`.

A Standard frame always represents exactly one sample. It carries no count on the wire; readers
treat its count as `1`.

## Frame Payload (Layer 0)

The payload is the RLE run vector bit-packed at the widths declared in the header. For each run, in
order, the encoder emits:

1. the district id in `max_val_bit_count` bits, then
1. the run length in `max_len_bit_count` bits.

Bits are packed most-significant-bit first into a byte stream: the first run's value occupies the
high bits of byte 0. After the final run, any leftover bits in the last byte are zero-padded on the
low side to reach a byte boundary. `n_bytes` counts that final padded byte.

Because each run occupies a fixed `max_val_bit_count + max_len_bit_count` bits, a decoder reads runs
back by consuming that many bits at a time until it has consumed `n_bytes` worth of payload,
ignoring the trailing zero-pad bits of the final byte. The run vector is then expanded into the
assignment by repeating each `value` `length` times.

### Worked example

Take the assignment `[1, 1, 1, 2, 2, 2, 2, 3]`.

- RLE: `[(1, 3), (2, 4), (3, 1)]`.
- Largest district id is `3` → `max_val_bit_count = 2`. Largest run length is `4` →
  `max_len_bit_count = 3`. Each run takes `2 + 3 = 5` bits; 3 runs = 15 bits → `n_bytes = 2`.
- Bit string, value then length per run (MSB first): `01 011  10 100  11 001` = `01011 10100 11001`
  → pad to 16 bits → `0101 1101 0011 0010`.
- Payload bytes: `0x5D 0x32`.
- Standard frame: `02 03 00 00 00 02 5D 32`.
- MkvChain frame for the same sample repeated 4 times: `02 03 00 00 00 02 5D 32 00 04`.

## XBEN Body (BEN32 Intermediate)

A `.xben` file wraps a BEN stream in LZMA2. For Standard and MkvChain, the bytes inside the LZMA2
stream are **not** the bit-packed layer-2 frames above; they are the **BEN32 intermediate**, a
fixed-width columnar form that compresses better under LZMA2. (TwoDelta uses a different XBEN body
and is out of scope here.)

The decompressed BEN32 body is:

```text
[17-byte Banner]
[BEN32 Frame 1]
[BEN32 Frame 2]
...
```

The banner is the same 17-byte identifier as in the plain `.ben` stream and sits inside the
compressed payload.

A BEN32 frame is a sequence of 4-byte runs followed by a 4-byte zero sentinel:

```text
[run: u16 value BE][run: u16 length BE]   (repeated, one per RLE run)
[00 00 00 00]                              (4-byte zero sentinel: end of frame)
```

For MkvChain, a `u16` big-endian `count` follows the sentinel:

```text
... [00 00 00 00][count: u16 BE]
```

The zero sentinel is unambiguous because a valid run never has length `0`. As in the native frame
layout, MkvChain `count` MUST be `>= 1`.

The `stream_checksum` recorded by a BENDL bundle for an embedded XBEN stream is computed over these
compressed bytes, not over the decompressed BEN32 body.

## Reader Rules

A reader MUST:

1. Read and validate the 17-byte banner; reject unknown banners. The banner fixes the variant for
   the whole stream.
1. Read frames in the variant's wire format until a clean end of input at a frame boundary.
1. For each frame, recover the assignment by unpacking `n_bytes` of payload at the declared bit
   widths and expanding the runs; the assignment length is the sum of run lengths.
1. For MkvChain, read the trailing `u16` count after the payload, reject `count == 0`, emit the
   assignment `count` times, and add `count` to the expanded sample count. For Standard, treat the
   count as `1`.

A reader MUST surface an error (not a truncated result) if input ends partway through a frame
header, payload, or trailing count.

A reader MUST reject a run with a zero length anywhere it can observe one: in a bit-packed frame
payload (outside the final byte's zero-padding region) and in a BEN32 run that is not the frame
sentinel. The encoder never produces zero-length runs, so any such run is a corruption signal;
tolerating one would either silently drop data or shift later runs out of position.

A reader MAY impose an implementation-defined sanity bound on the expanded length of a single
assignment (the sum of a frame's run lengths) and reject frames that exceed it. The wire format
places no limit on assignment length, but each run can demand up to 65535 elements, so without a
bound a small malicious frame could request an arbitrarily large allocation. The bound MUST sit
well above any real dual graph (this implementation uses 2^27 ≈ 134 million elements).

Frame-level subsampling does not require unpacking payload bits: a reader can skip a frame by
reading its 6-byte header, seeking past `n_bytes` (and, for MkvChain, the 2-byte count), and only
unpacking the payloads of frames it keeps.

## Relationship Between the Variants

A Standard stream and a MkvChain stream are wire-incompatible: they carry different banners, and
MkvChain frames are 2 bytes longer. Semantically, a Standard stream is equivalent to a MkvChain
stream in which every frame has `count = 1` — MkvChain only adds value when a sampler produces
identical consecutive samples (for example, MCMC self-loops from rejected proposals). A converter
that re-encodes between the two MUST expand each MkvChain frame's `count` into that many Standard
frames, and conversely MAY collapse runs of identical Standard frames into a single MkvChain frame
with the appropriate count.

## Versioning Strategy

The frame header shape, the bit-packing rule, and the BEN32 body layout are contractual: committed
fixtures encoded under a stable major version MUST continue to decode in every later release of that
major version. A change to the frame header shape, the bit-packing convention, or the BEN32 layout
is a breaking change that requires a new fixture set under a new major version; existing fixtures
are never regenerated in place. See the format-stability policy.

## Out of Scope

- The TwoDelta variant (different frame shape and XBEN body).
- The BENDL bundle container that embeds a BEN/XBEN stream as an opaque payload.
- LZMA2 framing details; XBEN treats LZMA2 as an opaque outer wrapper around the BEN32 body.
