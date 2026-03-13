# BENDL Roadmap

## Goal

Add a higher-level `.bendl` container format that feels like a single file to users while preserving the streamable nature of the underlying assignment data.

The low-level assignment formats remain:

- `.ben`
- `.xben`

The new `.bendl` format is a richer file-oriented container for:

- assignment data
- metadata
- graph data
- relabel maps
- future optional assets

## Design Principles

- Keep `.ben` and `.xben` streamable.
- Treat `.bendl` as a seekable container format for regular files.
- Put stable assets near the front of the file.
- Put the live assignment stream at the end of the file.
- Allow incomplete `.bendl` files to remain partially usable after interruption.
- Patch the header on successful finalization instead of requiring a footer.

## Proposed Layout

`.bendl` should use this high-level layout:

```text
[Fixed Header]
[Directory / Metadata Section]
[Optional Extra Assets]
[Streaming Assignments Section]
```

Where:

- the header is written first with placeholder values
- the directory and optional assets are written before streaming starts
- the assignment stream is appended at the end
- on successful completion, the writer seeks back and patches the header

## Why This Layout

This layout ensures:

- graph data and relabel maps are readable even if the stream is interrupted
- the assignment stream can still be decoded up to EOF if the file is incomplete
- final facts like `sample_count` are only written once they are actually known

## Header Concept

The exact binary layout is still to be finalized, but the header should carry fields conceptually like:

```rust
struct BendlHeader {
    magic: [u8; 8],
    version: u16,
    flags: u16,
    complete: u8,
    reserved: [u8; 5],
    directory_offset: u64,
    directory_len: u64,
    stream_offset: u64,
    stream_len: u64,
    sample_count: u64,
}
```

Notes:

- `complete == 0` means the file was not finalized
- `stream_len == 0` can mean unknown or unfinalized
- `sample_count == u64::MAX` can represent unknown sample count

## Directory / Asset Section

The directory section should describe any front-loaded assets, such as:

- graph
- relabel map
- metadata blob
- future extras

This can be backed by a simple JSON or binary directory table. The important part is that these assets are discoverable without scanning the assignment stream.

## Assignment Stream

The assignment stream should be stored at the end of the file so writing can proceed incrementally.

The stream payload may be:

- BEN data
- XBEN data

The `.bendl` container should treat this as the primary large append-only region.

## Finalization Model

Expected write flow:

1. Write a provisional header.
2. Write directory data and optional assets.
3. Record `stream_offset`.
4. Stream the assignment data.
5. On successful completion, seek back and patch the header with:
   - `complete = true`
   - `stream_len`
   - `sample_count`
   - any other finalized metadata

If writing is interrupted:

- the header remains incomplete
- the front-loaded assets are still readable
- the assignment stream may still be readable up to EOF
- exact `sample_count` is unavailable unless the reader scans

## Reader Semantics

Reader behavior should be:

- read the fixed header
- inspect `complete`
- load directory and front-loaded assets
- read assignment data starting at `stream_offset`
- if `complete == false`, treat the file as recoverable but incomplete

This means `.bendl` readers should expose both:

- whether the bundle is complete
- whether assignment data is still usable

## Relationship to Existing Formats

- `.ben` and `.xben` remain the portable stream/data formats
- `.bendl` becomes the richer container format for complete datasets

This keeps responsibilities separated:

- assignment encoding stays in BEN/XBEN
- dataset metadata and optional extras live in BENDL

## PyBen Implications

Potential future Python API support:

- open a `.bendl` file directly
- expose `sample_count` immediately if finalized
- expose optional `graph` and `relabel_map`
- fall back to scanning assignment data if `sample_count` is unknown

## Open Questions

- exact binary encoding of the directory section
- whether the asset directory should be JSON or a compact binary table
- whether checksums should be included in the header
- whether assignment payload should always be XBEN inside `.bendl`
- whether `.bendl` writing should require seekable output explicitly

## Current Recommendation

Proceed with `.bendl` as:

- a single-file container
- a seekable file format
- front-loaded metadata/assets
- trailing assignment stream
- header patched on finalize

This best matches the requirements discussed so far.
