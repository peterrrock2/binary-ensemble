# Format Stability Policy

This crate ships committed binary fixtures under `ben/tests/fixtures/v<n>/` and a matching
`ben/tests/test_format_stability.rs` that decodes each one. The fixtures are the v1.0.0 wire-format
stability contract.

The fixtures have a second consumer: `ben/tests/test_fixture_mutations.rs` drives every
single-byte mutation and truncation prefix of each binary fixture through every public read entry
point, asserting panic freedom on corrupt input. Both suites must keep passing against the same
committed bytes.

## Contract

**Once a fixture directory is committed for a stable major version, every file inside it MUST
continue to decode correctly in every later release of that major version, and across major versions
to the extent practical.** That is the entire policy.

In particular:

- The decoded output of every committed fixture must equal the canonical input it was minted from.
- A reader at any later commit must accept the fixture without errors on any verifying path (asset
  checksums, stream checksum, decode-to-JSONL).
- Forward-compatible bits (reserved header flags, reserved asset-flag bits, higher minor versions)
  must continue to be ignored by readers, as the fixtures specifically pin that behavior.

## What "never regenerate in place" means

When the wire format changes:

- **Additive minor change** (a new flag bit, a new asset type): mint a _new_ fixture into the
  current `v<n>/` directory if it pins behavior the existing fixtures do not, but **leave every
  existing fixture untouched**.
- **Breaking major change** (header shape, frame shape, checksum algorithm): add a new
  `tests/fixtures/v<n+1>/` directory and a parallel generator and test set. The older `v<n>/`
  directory stays exactly as it was, and the test suite continues to decode it through whatever
  versioned reader path the library provides.

If a fixture is found to be wrong after the fact — e.g. it was minted with a bug that has since been
fixed — the right response is **not** to regenerate it. The right response is one of:

1. If the fixture's bytes are now invalid under v1.0.0 readers, the bug shipped in a release and the
   v1.0.0 reader needs to keep accepting that exact pattern (additive fixup, not byte-level
   rewrite).
1. If the fixture was committed before any release shipped to users, the breakage is internal and
   the fixture may be regenerated, but only with a clear note in the commit message.

The default answer is option 1.

## Why the regen test is `#[ignore]`

`generate_format_stability_fixtures` exists so that the regeneration procedure is documented in code
rather than in a separate script, but it is `#[ignore]` so that a routine `cargo test` cannot
overwrite the committed bytes by accident. The only legitimate reasons to run it are:

- Bootstrapping a brand-new fixture directory for a new major version.
- Adding a brand-new fixture inside the current directory that does not already exist.

Both cases should land in a dedicated PR whose title makes the intent explicit (e.g.
`fixtures: add v2.0.0 stability set`). Running the generator over an already-populated directory in
any other context is a bug.

`TwoDelta` is a released, stable variant: its `twodelta.*` fixtures are part of the frozen set,
minted by `generate_format_stability_fixtures` alongside every other variant, and bound by the same
contract above. Its wire format does not change within a major version.

## Inventory

The current `v1.0.0` set covers:

- `standard.ben`, `mkvchain.ben`, `twodelta.ben` — one BEN file per variant. The `twodelta.*`
  fixtures are minted from `TWODELTA_CANONICAL_JSONL` (not the shared `CANONICAL_JSONL`) and
  deliberately exercise **mixed snapshot/delta frames**: an anchor snapshot, a 2-swap delta, a
  repeat, a >2-district transition that forces a mid-stream snapshot, and a delta rebased onto it.
- `standard.xben`, `mkvchain.xben`, `twodelta.xben` — one XBEN file per variant.
- `flags_set.bendl` — a BENDL bundle with every currently-defined header and asset flag bit set on
  at least one object: header `HEADER_FLAG_STREAM_CHECKSUM`; a graph asset flagged
  `ASSET_FLAG_JSON | ASSET_FLAG_XZ | ASSET_FLAG_CHECKSUM`; a metadata asset flagged
  `ASSET_FLAG_JSON | ASSET_FLAG_CHECKSUM`; an XBEN assignment stream.
- `unknown_flags.bendl` — a derivative of `flags_set.bendl` with reserved bits set in the header
  `flags` and in a custom asset's `asset_flags`. Pins forward-compatible reader behavior: unknown
  bits must be ignored, all known operations still succeed.
- `interop.pcompress` — the canonical ensemble encoded by the **foreign PCompress
  implementation** (the `pcompress` crates.io dependency, mggg's real encoder). Pins the
  `ben pcompress` interop contract: genuine PCompress bytes must keep converting to BEN that decodes back to
  `source.jsonl`. Minted by the focused `generate_pcompress_interop_fixture` regenerator;
  re-minting is legitimate only when the pinned `pcompress` dependency version changes its wire
  format, in a dedicated PR.
- `source.jsonl`, `source_twodelta.jsonl`, `source_graph.json`, `source_metadata.json` —
  human-readable sources committed alongside the binary fixtures so the contents can be inspected
  without running the codec. `source.jsonl` mints the Standard/MkvChain/BENDL fixtures;
  `source_twodelta.jsonl` mints the TwoDelta fixtures.

If you add a new fixture, list it here.

## Cross-host reproducibility note

XBEN compression uses `xz` and is sensitive to thread count, compression level, and block size. The
generator pins `n_threads = Some(1)`, `compression_level = Some(6)`, and lets the codec choose the
rest, which makes the minted bytes deterministic across machines. **Stability does not depend on
this**, however — only the decoded output is contractual. If a future liblzma version produces
different compressed bytes for the same input, that does not break this contract; what matters is
that the committed bytes continue to decode.
