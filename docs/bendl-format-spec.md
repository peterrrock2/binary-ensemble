# BENDL Format Specification Draft

## Status

Draft design for a future `.bendl` file format.

This document defines a concrete binary layout for a single-file dataset container that:

- feels like one file to users
- keeps metadata and optional assets accessible near the front
- stores the assignment stream at the end
- supports interrupted writes
- can be finalized by patching the header

This specification is intentionally separate from the existing `.ben` and `.xben` formats.

## Design Goals

- Single-file dataset container.
- Efficient access to front-loaded metadata.
- Stream-friendly assignment payloads.
- Recoverable partial files after interruption.
- Forward-compatible directory structure.
- Fast `sample_count` lookup for finalized bundles.

## Terminology

- `bundle`: a `.bendl` file.
- `asset`: a named front-loaded object such as a graph or relabel map.
- `assignment stream`: the trailing BEN or XBEN payload.
- `finalized bundle`: a bundle whose header has been patched to indicate successful completion.
- `incomplete bundle`: a bundle whose assignment stream may still be usable, but whose final size/count information is not authoritative.

## File Layout

A `.bendl` file is laid out as:

```text
[Fixed Header]
[Directory Table]
[Asset Payloads]
[Assignment Stream]
```

The assignment stream is always the final data region in the file.

## Byte Order

All fixed-width integers are encoded as little-endian unless otherwise stated.

## Fixed Header

The file begins with a fixed-size 64-byte header:

```text
offset  size  field
0       8     magic
8       2     major_version
10      2     minor_version
12      2     flags
14      1     complete
15      1     assignment_format
16      8     directory_offset
24      8     directory_len
32      8     stream_offset
40      8     stream_len
48      8     sample_count
56      8     reserved
```

### Header Fields

- `magic`
  - fixed bytes identifying the file as BENDL
  - proposed value: `b"BENDL\\0\\0\\1"`
- `major_version`
  - initial value: `1`
- `minor_version`
  - initial value: `0`
- `flags`
  - bundle-level feature flags
- `complete`
  - `0` means incomplete/unfinalized
  - `1` means finalized
- `assignment_format`
  - `1 = BEN`
  - `2 = XBEN`
- `directory_offset`
  - byte offset of the directory table
- `directory_len`
  - byte length of the directory table
- `stream_offset`
  - byte offset where the assignment stream begins
- `stream_len`
  - length in bytes of the assignment stream
  - `0` if unknown/unfinalized
- `sample_count`
  - number of expanded samples in the assignment stream
  - `u64::MAX` if unknown/unfinalized
- `reserved`
  - reserved for future extension

## Header Flags

Initial proposed header flags:

- bit 0: directory contains checksums
- bit 1: bundle contains graph asset
- bit 2: bundle contains relabel map asset
- bit 3: bundle contains metadata asset

Unrecognized flags must be ignored by readers unless a future version marks them as mandatory.

## Directory Table

The directory table is a compact binary table describing front-loaded assets.

Layout:

```text
offset  size  field
0       4     entry_count
4       ...   repeated directory entries
```

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
  - byte length of the asset payload
- `checksum_len`
  - byte length of optional checksum bytes that follow the name
- `name bytes`
  - UTF-8 asset name
- `checksum bytes`
  - optional checksum payload, interpretation depends on flags

### Asset Types

Initial proposed asset types:

- `1 = metadata.json`
- `2 = graph.json`
- `3 = relabel_map.json`
- `4 = custom user asset`

### Asset Flags

Initial proposed asset flags:

- bit 0: payload is UTF-8 JSON
- bit 1: payload is zstd-compressed
- bit 2: checksum present

Readers must skip unknown asset types and unknown flags when possible.

## Asset Payload Region

Assets are written after the directory table and before the assignment stream.

Each asset payload is raw bytes referenced by the directory table. The bundle does not require per-asset wrapper headers in the payload region because offsets and lengths are already described by the directory entries.

Examples of front-loaded assets:

- graph file
- relabel map
- extra metadata JSON
- provenance/configuration info

## Assignment Stream Region

The assignment stream starts at `stream_offset` and occupies `stream_len` bytes if the bundle is finalized.

The stream payload must be one of:

- BEN byte stream
- XBEN byte stream

The bundle does not reinterpret BEN/XBEN internals. It only stores the opaque assignment stream and records its format in `assignment_format`.

### Incomplete Bundles

If `complete == 0`:

- `stream_len` may be `0`
- `sample_count` may be `u64::MAX`
- readers should treat assignment data as extending from `stream_offset` to EOF

This allows partially written bundles to remain recoverable.

## Finalization Rules

Writers are expected to use this sequence:

1. Write a provisional header with:
   - `complete = 0`
   - `stream_len = 0`
   - `sample_count = u64::MAX`
2. Write the directory table.
3. Write all front-loaded assets.
4. Record `stream_offset`.
5. Write the assignment stream.
6. On successful completion:
   - compute final `stream_len`
   - compute final `sample_count`
   - seek back to patch the header
   - set `complete = 1`

If writing is interrupted before step 6, the file remains an incomplete bundle.

## Reader Rules

Readers must:

1. Validate `magic` and supported version.
2. Read the fixed header.
3. Read the directory table.
4. Make front-loaded assets available immediately.
5. Interpret the assignment stream according to `assignment_format`.
6. If `complete == 0`, treat the stream as running from `stream_offset` to EOF.

Readers should expose:

- whether the bundle is finalized
- whether `sample_count` is authoritative
- whether the assignment stream is still readable

## Recovery Semantics

If a bundle write is interrupted:

- header and front-loaded assets should still be usable if fully written
- assignment data should be readable from `stream_offset` to EOF
- `sample_count` should be treated as unknown
- the bundle should be marked incomplete

If the interruption happens before the directory or assets are fully written, the bundle may be unreadable. Writers should therefore prefer writing small front-loaded metadata first and beginning the assignment stream only after the directory is complete.

## Metadata Conventions

Although the directory is binary, metadata payloads should initially use JSON for ease of debugging.

Recommended metadata file names:

- `metadata.json`
- `graph.json`
- `relabel_map.json`

Recommended metadata fields:

```json
{
  "bundle_version": 1,
  "assignments_format": "xben",
  "variant": "mkv_chain",
  "complete": false
}
```

## Versioning Strategy

- incompatible structural changes require `major_version` bump
- additive backward-compatible fields may use `minor_version` bump
- unknown asset types should be ignored when possible

## Suggested Rust Types

Conceptual Rust representations:

```rust
pub struct BendlHeader {
    pub magic: [u8; 8],
    pub major_version: u16,
    pub minor_version: u16,
    pub flags: u16,
    pub complete: u8,
    pub assignment_format: u8,
    pub directory_offset: u64,
    pub directory_len: u64,
    pub stream_offset: u64,
    pub stream_len: u64,
    pub sample_count: u64,
    pub reserved: u64,
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

## Suggested Module Layout

If implemented in `ben`, the new code should likely live under:

```text
ben/src/bundle/
  mod.rs
  format.rs
  reader.rs
  writer.rs
  manifest.rs
```

Responsibilities:

- `format.rs`: binary header/directory definitions
- `reader.rs`: bundle reader
- `writer.rs`: bundle writer/finalizer
- `manifest.rs`: JSON metadata structs

## Out of Scope for V1

- non-seekable `.bendl` writing
- embedding assignment count inside BEN/XBEN themselves
- random-write mutation of existing bundles
- archive-level compression beyond the assignment stream format

## Current Recommendation

Implement `.bendl` V1 as:

- a seekable file container
- a fixed header plus binary directory
- front-loaded optional assets
- trailing BEN/XBEN assignment stream
- header patched on successful finalize

This keeps the format simple, recoverable, and aligned with the current streaming requirements.
