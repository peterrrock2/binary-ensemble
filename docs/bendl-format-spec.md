# BENDL Format Specification

## Status

Stable wire format. This document specifies the implemented v1 `.bendl` container as produced and
consumed by the bundle reader, writer, and appender, and pinned by the committed v1.0.0 stability
fixtures.

A `.bendl` is a single-file dataset container that:

- feels like one file to users
- keeps metadata and optional assets in the same container
- stores the assignment stream as a bounded embedded payload
- supports interrupted writes
- can be finalized by patching the header

The `.bendl` container is distinct from the `.ben` and `.xben` stream formats. It embeds a BEN or
XBEN stream as an opaque payload and records which one in its header; the stream's own layout
(banner and frames) is specified in the BEN stream format specification.

## Design Goals

- Single-file dataset container.
- Directory-indexed access to metadata and optional assets.
- Stream-friendly assignment payloads.
- Recoverable partial files after interruption.
- Forward-compatible directory structure.
- Fast `sample_count` lookup for finalized bundles.

## Terminology

- `bundle`: a `.bendl` file.
- `asset`: a named object such as a graph or node permutation map.
- `assignment stream`: the embedded BEN or XBEN payload.
- `finalized bundle`: a bundle whose header has been patched to indicate successful completion.
- `incomplete bundle`: a bundle whose assignment stream may still be usable, but whose final
  size/count information is not authoritative.

## File Layout

A `.bendl` file is laid out as:

```text
[Fixed Header]
[Asset Payloads]
[Assignment Stream]
[Directory Table]
```

The directory table is normally the final data region in a finalized bundle. A post-finalize append
writes new asset payloads and a replacement directory after the old EOF, then patches the header
last; if that final patch fails, the old directory remains authoritative and newer bytes are
orphaned.

## Byte Order

All fixed-width integers are encoded as little-endian unless otherwise stated.

## Fixed Header

The file begins with a fixed-size 64-byte header:

```text
offset  size  field
0       8     magic
8       2     major_version
10      2     minor_version
12      1     finalized
13      1     assignment_format
14      2     alignment_padding
16      4     flags
20      4     stream_checksum
24      8     directory_offset
32      8     directory_len
40      8     stream_offset
48      8     stream_len
56      8     sample_count
```

Total: 64 bytes. All multi-byte integers are little-endian.

### Header Fields

- `magic` — 8 bytes identifying the file as BENDL. Value: `b"BENDL\0\0\x01"`.
- `major_version` — incompatible-change version. Current value: `1`.
- `minor_version` — additive backward-compatible version. Current value: `0`.
- `finalized` — `0` means incomplete/unfinalized; `1` means finalized.
- `assignment_format` — `1 = BEN` (plain bit-packed), `2 = XBEN` (xz-compressed BEN32).
- `alignment_padding` — two bytes of padding that keep the following 8-byte fields at offset ≥ 24
  8-byte aligned. Writers set to zero; readers ignore non-zero bytes. Not a forward-compat slot —
  new fields must live elsewhere.
- `flags` — 32-bit bundle-level feature flags. See **Header Flags** below. Bits without a defined
  constant are reserved; writers set them to zero.
- `stream_checksum` — CRC32C (Castagnoli polynomial) of the on-disk assignment stream bytes
  `[stream_offset, stream_offset + stream_len)`. For XBEN streams the CRC is over the compressed
  bytes, not the decompressed content. Valid only when `HEADER_FLAG_STREAM_CHECKSUM` (bit 0) is set
  in `flags`. Writers set this field to zero while the bundle is unfinalized and patch it with the
  final value at finalization time.
- `directory_offset` — absolute byte offset of the directory table. Zero if no directory has been
  written yet.
- `directory_len` — byte length of the directory table. Zero if absent.
- `stream_offset` — byte offset where the assignment stream begins.
- `stream_len` — exact byte length of the assignment stream. Zero if unfinalized. Readers MUST
  surface an error if the backing file is shorter than this declared length.
- `sample_count` — number of expanded samples in the assignment stream. `-1` if unfinalized.

## Header Flags

- **Bit 0 — `HEADER_FLAG_STREAM_CHECKSUM`**: the `stream_checksum` field contains a valid CRC32C
  over the on-disk assignment stream bytes. Library writers always set this flag and write a valid
  checksum. The clear-flag state exists only for adversarial/foreign bytes and partial-recovery
  flows; verified reader APIs return `Unavailable` when this flag is clear.

Bits 1–31 are reserved. Unrecognized flag bits must be ignored by readers.

## Directory Table

The directory table is a compact binary table describing asset payloads.

Layout:

```text
offset  size  field
0       4     entry_count
4       ...   repeated directory entries
```

`entry_count` is the number of asset entries that follow; the assignment stream is stored outside
the directory and does not count toward it. Readers MUST reject a bundle whose `entry_count` exceeds
`MAX_DIRECTORY_ENTRIES` (256) **before** allocating per-entry storage, so a corrupt or adversarial
count cannot force a large reservation. Writers MUST NOT emit more than `MAX_DIRECTORY_ENTRIES`
entries.

Each directory entry has the following header:

```text
offset  size  field
0       2     asset_type
2       2     asset_flags
4       2     name_len
6       2     reserved
8       8     payload_offset
16      8     payload_len
24      4     checksum_len
28      ...   name bytes
...     ...   checksum bytes
```

### Directory Entry Fields

- `asset_type`
  - identifies the meaning of the payload
- `asset_flags`
  - encoding/compression flags for that asset
- `name_len`
  - UTF-8 byte length of the asset name
- `payload_offset`
  - absolute file offset of the asset payload
- `payload_len`
  - exact byte length of the on-disk asset payload. Readers MUST surface an error if the backing
    file is shorter than this declared length; they MUST NOT silently return a truncated payload.
- `checksum_len`
  - byte length of the optional checksum bytes that follow the name. MUST be `4` when the
    `ASSET_FLAG_CHECKSUM` bit (bit 2) is set and `0` when it is clear; readers MUST reject any entry
    where the flag and `checksum_len` disagree.
- `name bytes`
  - UTF-8 asset name
- `checksum bytes`
  - optional checksum payload, interpretation depends on flags

### Asset Types

Defined asset types:

- `1 = metadata.json`
- `2 = graph.json`
- `3 = node_permutation_map.json`
- `4 = custom user asset`

Types `1`–`3` are singleton known assets: each may appear at most once and MUST use its standardized
name. Type `4` is a custom asset with a writer-chosen name, and multiple are allowed.

### Asset Flags

Defined asset flags:

- bit 0: payload is UTF-8 JSON
- bit 1: payload is xz-compressed
- bit 2: checksum present

When bit 2 is set, the trailing checksum is exactly four little-endian bytes holding a CRC32C
(Castagnoli polynomial) over the **on-disk payload bytes**
(`payload_offset .. payload_offset + payload_len`). For an xz-compressed asset the CRC covers the
compressed bytes, so verification happens before decompression. Library writers always set bit 2 and
write a valid checksum.

Readers must skip unknown asset types and unknown flag bits when possible.

## Asset Payload Region

Assets are written after the fixed header and before the assignment stream. Each asset payload is
referenced by the trailing directory table.

Each asset payload is raw bytes referenced by the directory table. The bundle does not require
per-asset wrapper headers in the payload region because offsets and lengths are already described by
the directory entries.

The directory is the sole authority on which payloads exist. A bundle MAY contain byte ranges that
no directory entry (and no header field) references — for example, the payload left behind when a
writer removes an asset by rewriting the directory without its entry, or a superseded directory
left behind by an append. Readers MUST locate payloads solely via directory offsets and MUST NOT
assume payloads are contiguous. Whole-bundle rewrites (a compaction or a recompression) reclaim
unreferenced ranges; the user-facing removal paths (the `bendl remove` CLI command and the Python
facade) compact automatically after removing.

Examples of assets:

- graph file
- node permutation map
- extra metadata JSON
- provenance/configuration info

## Assignment Stream Region

The assignment stream starts at `stream_offset` and occupies `stream_len` bytes if the bundle is
finalized.

The stream payload must be one of:

- BEN byte stream
- XBEN byte stream

The bundle does not reinterpret BEN/XBEN internals. It only stores the opaque assignment stream and
records its format in `assignment_format`.

### Incomplete Bundles

If `finalized == 0`:

- `stream_len` may be `0`
- `sample_count` is `-1`
- readers should treat assignment data as extending from `stream_offset` to EOF

This allows partially written bundles to remain recoverable.

## Finalization Rules

Writers are expected to use this sequence:

1. Write a provisional header with:
   - `finalized = 0`
   - `stream_len = 0`
   - `sample_count = -1`
1. Write all asset payloads.
1. Record `stream_offset`.
1. Write the assignment stream.
1. Compute the assignment-stream checksum.
1. Write the trailing directory table.
1. On successful completion:
   - compute final `stream_len`
   - compute final `sample_count`
   - record final `directory_offset` and `directory_len`
   - seek back to patch the header
   - set `finalized = 1`

If writing is interrupted before step 7, the file remains an incomplete bundle.

## Reader Rules

Readers must:

1. Validate `magic` and supported `major_version`. Higher `minor_version` values are accepted.
1. Read the fixed header.
1. Read the authoritative directory table identified by `directory_offset` and `directory_len`.
   Reject a declared `entry_count` above `MAX_DIRECTORY_ENTRIES` (256) before allocating, and reject
   any bytes left over in the directory region after the declared entries.
1. Validate the directory: asset names must be unique, and a singleton known type (1–3) must use its
   standardized name. Reject a directory that violates either rule.
1. Make directory-listed assets available.
1. Interpret the assignment stream according to `assignment_format`.
1. If `finalized == 0`, treat the stream as running from `stream_offset` to EOF (or to
   `directory_offset` when a provisional directory was written).

Readers should expose:

- whether the bundle is finalized
- whether `sample_count` is authoritative
- whether the assignment stream is still readable

## Recovery Semantics

If a bundle write is interrupted:

- header and assets should still be usable if fully written and directory-listed
- assignment data should be readable from `stream_offset` to EOF
- `sample_count` should be treated as unknown
- the bundle should be marked incomplete

If the interruption happens before the final directory or header patch is written, the bundle may be
incomplete. Post-finalize append is ordered so that the old directory remains authoritative until
the replacement directory is committed by the final header patch.

## Metadata Conventions

Although the directory is binary, metadata payloads use JSON for ease of debugging.

Standardized metadata file names:

- `metadata.json`
- `graph.json`
- `node_permutation_map.json`

The `metadata.json` payload mirrors the fixed header for human readability; the header (and, for the
variant, the embedded stream banner) remains authoritative. Its fields are:

```json
{
  "major_version": 1,
  "minor_version": 0,
  "assignment_format": "xben",
  "variant": "mkv_chain",
  "complete": false
}
```

- `major_version` / `minor_version` — mirror the header version fields.
- `assignment_format` — `"ben"` or `"xben"`, mirroring the header `assignment_format` byte.
- `variant` — `"standard"`, `"mkv_chain"`, or `"two_delta"`, mirroring the embedded stream banner.
  Optional; omitted when unknown.
- `complete` — mirrors the header `finalized` flag.

## Versioning Strategy

- incompatible structural changes require `major_version` bump
- additive backward-compatible fields may use `minor_version` bump
- unknown asset types should be ignored when possible

## Rust Types

The in-memory representations of the header and directory entries:

```rust
pub struct BendlHeader {
    pub magic: [u8; 8],
    pub major_version: u16,
    pub minor_version: u16,
    pub finalized: u8,
    pub assignment_format: u8,
    pub alignment_padding: u16,
    pub flags: u32,
    pub stream_checksum: u32,
    pub directory_offset: u64,
    pub directory_len: u64,
    pub stream_offset: u64,
    pub stream_len: u64,
    pub sample_count: i64,
}

pub struct BendlDirectoryEntry {
    pub asset_type: u16,
    pub asset_flags: u16,
    pub name: String,
    pub payload_offset: u64,
    pub payload_len: u64,
    pub checksum: Option<Vec<u8>>,
}
```

## Module Layout

The implementation lives under:

```text
ben/src/io/bundle/
  mod.rs
  format.rs
  reader.rs
  writer.rs
  manifest.rs
  verify.rs
  error.rs
```

Responsibilities:

- `format.rs`: binary header/directory definitions and their encode/decode helpers
- `reader.rs`: bundle reader (header + directory parsing, asset and stream access)
- `writer.rs`: bundle writer/finalizer and the post-finalize appender
- `manifest.rs`: JSON metadata structs
- `verify.rs`: bounded readers, CRC tees, and checksum verification adapters
- `error.rs`: read-side error and checksum-error types

## Out of Scope for V1

- non-seekable `.bendl` writing
- embedding assignment count inside BEN/XBEN themselves
- random-write mutation of existing bundles
- archive-level compression beyond the assignment stream format

## Summary

`.bendl` v1 is:

- a seekable file container
- a fixed header plus asset payloads, assignment stream, and trailing binary directory
- optional assets referenced by directory entries
- an embedded BEN/XBEN assignment stream
- a header patched on successful finalize

This keeps the format simple, recoverable, and aligned with the streaming requirements.
