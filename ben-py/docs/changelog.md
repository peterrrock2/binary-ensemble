# Changelog

Notable changes to the `binary-ensemble` Python package and Rust core, newest first. The
byte-level stability promises for the BEN/XBEN/BENDL formats themselves are covered separately
in [Compatibility and stability](concepts/compatibility.md).

## Unreleased

- **Explicit lookup and zero-based subsampling (behavior change):** `ben lookup` now requires
  either `index` (zero-based) or `sample` (one-based). Decoder subsampling now uses zero-based
  indices, half-open ranges, and a zero-based `offset` defaulting to `0`.
- **Graph embedding default (behavior change):** `BendlEncoder.add_graph()` now preserves the
  input node order by default (`sort=None`) instead of applying multi-level clustering. Existing
  code that relies on implicit MLC ordering must pass `sort="mlc"`; explicit `"mlc"`, `"rcm"`,
  and `"key"` ordering is unchanged.
- **Bidirectional BENDL stream transforms:** added `decompress_stream`, the inverse of
  `compress_stream`, to rewrite an embedded XBEN stream as BEN. Both functions preserve graph,
  metadata, permutation-map, and custom assets; support a new output or atomic in-place
  replacement; handle assets-only bundles; and reject bundles with the wrong source stream
  format. Asset payloads and JSON flags are preserved, while their storage compression is
  normalized to the writer's current defaults.
- **TwoDelta event API (Rust):** added `TwoDeltaFrameEvent`, `TwoDeltaFrameEventReader`, and
  `BenStreamReader::into_twodelta_events()`. External tools can consume BEN or XBEN TwoDelta
  streams as snapshot and delta events, including repeat counts and position-level old/new label
  changes, instead of receiving only expanded assignments. Error behavior is covered for wrong
  variants, missing anchors, zero-count frames, unknown tags, and truncated input.
- **Python and release packaging:** bumped the workspace to 1.0.1, declared Python 3.11 through
  3.14 support, and added the Linux build tooling needed by the development environment.
- **Release automation:** refreshed GitHub Actions, restored trusted PyPI/TestPyPI publishing for
  tagged and manual builds, made missing wheel artifacts fail the workflow, and fixed the wheel
  smoke test to compare platform-independent LF bytes on Windows.
- **Documentation and tests:** updated the API reference, tutorials, and format-conversion guides
  for the new graph default and BENDL transforms. Added asset-preserving BEN/XBEN round-trip tests,
  public API and typing checks, and extensive coverage for the TwoDelta event reader.

## 1.0.0

First stable release of the rewritten Python API.

- **`.bendl` files** — `BendlEncoder` / `BendlDecoder` read and write the single-file
  bundle format: an assignment stream plus the dual graph, node permutation map, metadata,
  and custom assets. `compress_stream` recompresses a bundle's stream to XBEN and
  `relabel_bundle` reorders a bundle's graph and rewrites its stream to match, both
  preserving every asset.
- **Custom assets** — `add_asset` accepts JSON, text, and arbitrary binary payloads (plus a
  `"file"` content type that reads a path), with CRC32C checksums on every asset,
  transparent xz compression for payloads of 1 KiB or more, and `BendlDecoder.verify()` to
  validate a whole bundle's checksums in one call.
- **Plain streams** — `BenEncoder` / `BenDecoder` write and iterate plain `.ben`/`.xben`
  streams, with variant-aware subsampling (`subsample_indices`, `subsample_range`,
  `subsample_every`) shared with the bundle decoder. `standard` and `mkv_chain` skip whole
  frames; `twodelta` replays from snapshots.
- **Whole-file codecs** — `encode_jsonl_to_ben`, `encode_jsonl_to_xben`,
  `encode_ben_to_xben`, and the matching `decode_*` helpers convert complete files
  between JSONL, BEN, and XBEN.
- **Graph reordering** — `binary_ensemble.graph` exposes the MLC, RCM, and key-based
  orderings used by `add_graph` and `relabel_bundle`.
- **Encoding variants** — `standard`, `mkv_chain`, and `twodelta` (the default), with
  automatic variant detection on read.
- `binary_ensemble.__version__` reports the installed package version.

Requires Python 3.11+; NetworkX is the only runtime dependency.
