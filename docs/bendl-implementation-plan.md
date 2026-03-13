# BENDL Implementation Plan

## Goal

Turn the `.bendl` roadmap and format specification into an implementation sequence that is low-risk and easy to validate incrementally.

This plan assumes:

- `.ben` and `.xben` remain unchanged
- `.bendl` is a new seekable container format
- the assignment stream is stored at the end of the file
- header fields are patched on successful finalization

## Guiding Strategy

Build `.bendl` in layers:

1. binary format types
2. read-only support
3. write/finalize support
4. CLI integration
5. Python integration

This keeps the early steps small and testable.

## Phase 1: Core Format Types

Add a new top-level module:

```text
ben/src/bundle/
  mod.rs
  format.rs
  manifest.rs
```

### Tasks

- Define `BendlHeader`.
- Define constants for:
  - magic bytes
  - version numbers
  - assignment format identifiers
  - asset types
  - asset flags
- Define `BendlDirectoryEntry`.
- Implement binary encode/decode helpers for:
  - header read/write
  - directory entry read/write
- Add manifest-side serde structs for JSON metadata assets.

### Deliverable

Pure format layer with no I/O orchestration yet.

### Tests

- header round-trip tests
- directory entry round-trip tests
- invalid magic/version tests
- asset flag parsing tests

## Phase 2: Read-Only Bundle Support

Add:

```text
ben/src/bundle/reader.rs
```

### Tasks

- Implement `BendlReader<R: Read + Seek>`.
- Validate and parse the fixed header.
- Read and decode the directory table.
- Expose accessors for:
  - `is_complete()`
  - `sample_count() -> Option<u64>`
  - `assignment_format()`
  - `assets()`
- Implement helpers to:
  - open asset payloads by name/type
  - open the assignment stream region
- For incomplete bundles:
  - treat assignment stream as `stream_offset..EOF`

### Deliverable

A read-only API that can inspect bundle metadata and expose the embedded assignment stream.

### Tests

- parse finalized bundle fixture
- parse incomplete bundle fixture
- recover front-loaded assets when `complete == 0`
- ignore unknown asset types cleanly

## Phase 3: Bundle Writer

Add:

```text
ben/src/bundle/writer.rs
```

### Tasks

- Implement `BendlWriter<W: Write + Seek>`.
- Write provisional header.
- Write directory table.
- Write front-loaded assets.
- Track `stream_offset`.
- Stream BEN or XBEN payload at the end.
- Count samples while writing.
- On `finish()`:
  - compute `stream_len`
  - patch header
  - set `complete = 1`

### Important Constraints

- Writing should require `Seek`.
- `finish()` should be explicit.
- `Drop` should not silently attempt complex repair/finalization.

### Deliverable

A bundle writer that can produce finalized `.bendl` files and leave partially usable files behind if interrupted.

### Tests

- finalized bundle writes correct header fields
- incomplete writer leaves `complete = 0`
- assets remain readable after partial write
- correct `sample_count` patching

## Phase 4: Assignment Stream Integration

Connect bundle writing to the existing BEN/XBEN infrastructure.

### Tasks

- Allow writer to store:
  - BEN assignment stream
  - XBEN assignment stream
- Reuse existing encoders rather than reimplementing stream encoding.
- Add helper APIs such as:
  - `write_ben_stream(...)`
  - `write_xben_stream(...)`
  - `open_assignment_reader(...)`

### Deliverable

The bundle layer becomes a thin container around the current assignment formats.

### Tests

- bundle with BEN payload decodes correctly
- bundle with XBEN payload decodes correctly
- incomplete XBEN stream remains partially readable when possible

## Phase 5: CLI Support

Add CLI commands after the core library is stable.

Potential command surface:

```text
ben bundle create
ben bundle inspect
ben bundle extract
```

### Tasks

- create `.bendl` from assignment stream + optional assets
- inspect header and asset list
- extract embedded assets or assignment payload
- report completeness/finalization state

### Deliverable

User-facing bundle workflow in the Rust CLI.

### Tests

- integration tests for create/inspect/extract
- interrupted/incomplete bundle inspection
- metadata visibility before finalized stream count

## Phase 6: Python Support

Add optional `pyben` support once the Rust API settles.

### Tasks

- expose bundle inspection API
- expose `sample_count` if finalized
- expose graph/relabel-map asset loading
- optionally expose embedded assignment stream through `PyBenDecoder`

### Deliverable

Python can open `.bendl` as a higher-level dataset object.

### Tests

- open finalized bundle
- open incomplete bundle
- read graph metadata without forcing assignment scan

## Recommended Implementation Order

Recommended practical sequence:

1. `format.rs`
2. `reader.rs`
3. tests + sample fixtures
4. `writer.rs`
5. CLI support
6. `pyben` support

This order gives you inspection/debugging tools before write-path complexity.

## Suggested Public API Shape

Possible `ben` API surface:

```rust
pub mod bundle;

pub use bundle::reader::BendlReader;
pub use bundle::writer::BendlWriter;
```

And bundle module internals:

```rust
bundle::format
bundle::manifest
bundle::reader
bundle::writer
```

## Risks

- Header patching requires seekable outputs.
- Incomplete bundles need carefully defined recovery behavior.
- XBEN payloads may still require full scan when bundle metadata is absent or unfinalized.
- Asset directory changes should be versioned carefully to preserve forward compatibility.

## Recommended First Milestone

The first milestone should be:

- parse and inspect `.bendl` files
- list bundled assets
- open assignment stream region
- expose `complete` and `sample_count`

That gives immediate value and makes it easier to validate the spec before building the writer.
