# Changelog

Notable changes to the `binary-ensemble` Python package, newest first. The byte-level
stability promises for the BEN/XBEN/BENDL formats themselves are covered separately in
[Compatibility and stability](concepts/compatibility.md).

## 1.0.0

First stable release of the rewritten Python API.

- **`.bendl` bundles** — `BendlEncoder` / `BendlDecoder` read and write the single-file
  bundle format: an assignment stream plus the dual graph, node permutation map, metadata,
  and custom assets. `compress_stream` recompresses a bundle's stream to XBEN and
  `relabel_bundle` reorders a bundle's graph and rewrites its stream to match, both
  preserving every asset.
- **Plain streams** — `BenEncoder` / `BenDecoder` write and iterate plain `.ben`/`.xben`
  streams, with frame-skipping subsampling (`subsample_indices`, `subsample_range`,
  `subsample_every`) shared with the bundle decoder.
- **Whole-file codecs** — `encode_jsonl_to_ben`, `encode_jsonl_to_xben`,
  `encode_ben_to_xben`, and the matching `decode_*` helpers convert complete files
  between JSONL, BEN, and XBEN.
- **Graph reordering** — `binary_ensemble.graph` exposes the MLC, RCM, and key-based
  orderings used by `add_graph` and `relabel_bundle`.
- **Encoding variants** — `standard`, `mkv_chain`, and `twodelta` (the default), with
  automatic variant detection on read.
- `binary_ensemble.__version__` reports the installed package version.

Requires Python 3.11+; NetworkX is the only runtime dependency.
