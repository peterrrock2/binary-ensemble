# Glossary

Shared lexicon for the `binary-ensemble` workspace. This file is the source of truth for terminology
used in code, documentation, commit messages, and conversations about the project. When prose and
code disagree, prose follows this glossary; code identifiers should be brought into alignment by the
renames listed at the end.

## Domain Objects

- **Plan**
  - The mathematical object: a partition of dual-graph nodes into districts.
  - Orientation-free and label-free up to relabeling. A plan has many possible assignments (one per
    node ordering and district relabeling).
- **Assignment**
  - The vector encoding of a plan: a length-N `Vec<u16>` where index _i_ is the **district id** of
    node _i_, in dual-graph node order.
  - An assignment uniquely determines a plan; a plan does not uniquely determine an assignment.
- **District id**
  - The integer values stored in an assignment vector. Names _what the integer means_ (a district).
    Replaces "assignment id" in prose; code rename pending.
- **Sample**
  - One entry in an ensemble stream: the pair `(sample_number, assignment)`.
  - `sample_number` is 1-indexed and lives in _expanded_ space (see **sample count**).
- **Ensemble**
  - An ordered stream of samples produced by a single sampler run. The unit that `.ben`, `.xben`,
    and `.bendl` files all wrap.
  - Conceptually, an ensemble is a probabilistic draw from the space of possible plans.
- **Sample count**
  - The number of independent draws represented by an ensemble.
  - Always _expanded_: when a `MkvChain` frame collapses 5 identical consecutive samples into a
    single frame with `count = 5`, the ensemble's sample count contribution is 5, not 1.
  - The `sample_count` field of a bundle header carries this expanded number.

## Sampler vs Chain

- **Sampler**
  - Umbrella term for any algorithm that produces an ensemble of plans. Covers both Markov-chain
    methods (MCMC) and particle/weighted methods (SMC).
- **Chain**
  - Specifically MCMC. Use only when the Markov property matters; otherwise prefer "sampler."
- **ReCom-step**
  - A single accepted ReCom (recombination) move: consecutive samples differ by exactly one pairwise
    district swap. The transition a `TwoDelta` **delta frame** encodes. Transitions that are _not_ a
    clean pairwise swap — a multi-district move, random/independent sampling, or a district that was
    previously empty — are encoded as full snapshot frames instead (see **Variant fitness by
    sampler**).
- **Sample repetition**
  - The data shape that arises when consecutive samples in an ensemble are identical. May come from
    MCMC self-loops (proposal rejected) or from any other source.
  - `MkvChain` compresses sample repetitions via per-frame repetition counts.
  - `TwoDelta` also accommodates sample repetitions, via per-frame repetition counts plus a
    dedicated repeat-frame layout (`twodelta_repeat_frame` in `io/writer/stream_writer/ben.rs`). The
    per-frame TwoDelta delta encoder returns `EncodeError::TwoDeltaIdentical` if called with two
    identical assignments, but the stream writer routes repeats through the repeat-frame path before
    that error can surface.

## Encoding Stack

The five layers of the BEN-family encoding pipeline. Use the layer name unambiguously; never
compress multiple layers into one word.

| Layer | Name        | What it is                                                                                                   |
| ----- | ----------- | ------------------------------------------------------------------------------------------------------------ |
| 0     | bit-packing | cramming run values into bit-precise widths                                                                  |
| 1     | RLE         | `(value, length)` pairs                                                                                      |
| 2     | frame       | one sample's encoded bytes: frame header + payload, plus a repetition count for `MkvChain` and `TwoDelta`    |
| 3     | stream      | banner + concatenated frames; the contents of a `.ben` file or the LZMA2-decompressed body of a `.xben` file |
| 4     | container   | the on-disk file: `.ben`, `.xben`, or `.bendl`                                                               |

- **Banner**
  - 17-byte ASCII identifier at the start of every BEN/XBEN stream. One per file.
  - Three legal values, one per variant: `STANDARD BEN FILE`, `MKVCHAIN BEN FILE`,
    `TWODELTA BEN FILE`.
- **Magic**
  - 8-byte file-format identifier at offset 0 of a `.bendl` file. Different concept and different
    shape from a banner; the two terms are kept distinct.
- **Header** — always qualified:
  - **Frame header**: the leading bytes of one frame (bit-width fields and per-variant metadata; 6
    bytes for `Standard`/`MkvChain`, 9 bytes for a `TwoDelta` delta frame; a `TwoDelta` snapshot
    frame is `MkvChain`-formatted with a 6-byte header). The 1-byte per-frame tag that precedes
    every frame in a `TwoDelta` stream is a stream-layer concern, not part of the frame header.
  - **Bundle header**: the 64-byte fixed header at offset 0 of a `.bendl` file.
  - Bare "header" is ambiguous and should not appear in shared docs.
- **Frame**
  - One sample's encoded bytes. `MkvChain` and `TwoDelta` frames carry a trailing `u16` repetition
    count. `Standard` frames are 1-sample.
- **Stream**
  - Banner plus concatenated frames. In a `TwoDelta` stream each frame is additionally prefixed with
    a 1-byte tag (snapshot vs delta) so the two body layouts self-distinguish; the first frame is
    always a snapshot. The `assignment_stream` region of a `.bendl` is exactly a layer-3 BEN/XBEN
    stream stored at a known offset; the bundle layer treats it as opaque bytes.
- **BEN32 intermediate**
  - The columnar wire format used inside an XBEN container's LZMA2-compressed body: u16 value + u16
    length per run, u32 zero sentinel between samples.
  - Used only by `Standard` and `MkvChain`. `TwoDelta` bypasses BEN32 and uses its own columnar
    layout.
  - **Never a standalone file format.** Always say "BEN32 intermediate" or "BEN32 wire format,"
    never "BEN32 file."

## Variants

- **Variant**
  - One of `{Standard, MkvChain, TwoDelta}`. A property of a stream, fixed for the whole stream by
    the banner. One variant per file.
- **Inter-sample constraint**
  - The rule a variant imposes on consecutive samples.
  - `Standard`: none.
  - `MkvChain`: identical-consecutive samples are collapsible into a single frame with `count > 1`.
  - `TwoDelta`: none on the input. A _non-repeat_ transition that is a single ReCom-step (exactly
    two district ids exchange positions; no position outside that pair changes, and both ids already
    exist) is delta-encoded; any other transition is stored as a full snapshot frame.
    Identical-consecutive samples are accommodated via repetition counts and a repeat-frame layout,
    not by the per-frame delta encoder.
- **Variant fitness by sampler**
  - `Standard`: any ensemble; baseline.
  - `MkvChain`: full-chain ensembles (every step including rejections logged). Compresses sample
    repetitions efficiently. Suitable for any sampler.
  - `TwoDelta`: any ensemble. Delta-compresses pairwise ReCom steps and emits a full snapshot frame
    for every other transition, so it is **compatible** with random sampling and Forest ReCom —
    those just produce more snapshot frames and less delta compression. Best compression comes from
    a full-chain _pairwise_ ReCom ensemble, where nearly every accepted move changes exactly two
    districts.

## Files and Containers

- **`.ben`** — a layer-3 stream stored on disk, no outer wrapping.
- **`.xben`** — a `.ben` stream's content (BEN32 intermediate for `Standard`/`MkvChain`, columnar
  for `TwoDelta`) wrapped in LZMA2.
- **`.bendl`** — a bundle: bundle header + asset payloads + assignment stream + trailing directory.
- **Container** is the umbrella term for any of these on-disk files.

## Tools and Packages

The workspace ships one Rust library, four CLI binaries, and one Python package backed by PyO3
bindings.

- **The library crate** — `binary-ensemble` on crates.io (lib name `binary_ensemble`). Contains the
  codec, I/O, ops, and bundle modules; the two CLI binaries are thin wrappers over `cli::*::run()`.
- **The CLI tool family** — `ben` and `bendl`. `ben` is a subcommand tree spanning several
  architectural roles; `bendl` owns the container role:
  - **Codec role** — `ben encode`/`decode`/`xencode`/`xdecode`. Encode/decode BEN-family streams,
    plus `ben xz-compress`/`xz-decompress` wrapping convenience.
  - **Pipeline role** — `ben relabel`/`canonicalize`/`reencode` (and `sort-graph`). Drives the
    relabel pipeline (decode → transform → re-encode) with canned transforms (first-seen relabel,
    key-based or topology-based node ordering, variant re-encode).
  - **Bridge role** — `ben pcompress` (`from-ben` / `to-ben` / `to-xben`). Translates between BEN
    and the foreign **PCompress** format.
  - **Bundle tool** — `bendl`. Create / inspect / extract / append for `.bendl` containers.
- **The Python package** — `binary_ensemble` on PyPI. The user-facing Python entry point.
- **The Python bindings crate** — `ben-py` (cdylib `ben_py_core`). Internal scaffolding; users never
  import this name — they import `binary_ensemble`.
- **`_core`** — the pymodule produced by the bindings crate; imported as `binary_ensemble._core`.
  Implementation detail — Python users reference re-exported names from the package, not `_core`
  directly.

### Cross-language consistency

Verbs and class names are intentionally aligned across Rust and Python to keep the lexicon uniform.
The Python API uses `encode_*` / `decode_*` (matching Rust prose), not `compress_*` / `decompress_*`
(the historical Python-only naming, scheduled for rename). Python classes are exposed as
`BenEncoder` / `BenDecoder` (the `Py` prefix on Rust-side structs is a PyO3 implementation
convention and is stripped at the Python boundary).

## Operations and Verbs

CLI mode names and prose verbs are not always identical. Prose follows this glossary; CLI flags are
listed for reference.

- **encode**
  - Produce some BEN-family output from JSONL or another BEN-family input.
  - CLI: `ben encode` (JSONL → BEN), `ben xencode` (JSONL → XBEN, or BEN → XBEN with `--from-ben`).
- **decode**
  - Produce JSONL from a BEN-family input.
  - CLI: `ben decode` (BEN → JSONL, or XBEN → BEN with `--from-xben`), `ben xdecode` (XBEN → JSONL).
- **`x` prefix**
  - Means "with LZMA2 wrapping." Not a separate verb; a modifier on `encode`/`decode`.
- **Sample lookup** _(prose)_ / random-access decode
  - Decode just sample N from a BEN file.
  - CLI: `ben lookup -n N`.
- **Subsampling**
  - Iterate over a subset of frames without consuming the whole stream. The umbrella that
    `lookup -n N` is the special case "subsample of size 1."
- **Asset extract** vs **sample-range extract**
  - Two unrelated operations sharing the verb "extract." Always qualify in prose.
  - **Asset extract**: pull a named asset out of a bundle. Code: `extract_asset`. CLI:
    `bendl extract`.
  - **Sample-range extract**: pull a contiguous range of samples out of a BEN file. Code:
    `extract_sample_range`.
- **xz-compress / xz-decompress**
  - Wrap an arbitrary file in xz / unwrap it. Not a BEN-aware operation; included in the `ben` CLI
    for convenience.
- **Inspect**
  - List the assets in a bundle. CLI: `bendl inspect`.
- **Create**
  - Build a new bundle from a stream plus assets. CLI: `bendl create`.
- **Append** _(strict)_
  - Add a new asset to a _finalized_ bundle: write new asset payloads after the old EOF, write a
    replacement trailing directory, then repatch the header. The old directory becomes orphaned
    bytes after a successful patch, and remains authoritative if the final header patch fails.
  - **Never** means extending the assignment stream. If stream-extension is ever wanted, call it
    **rewrite** or **reflow** (the implementation builds a new bundle and copies assets across).
- **Bridge**
  - The architectural role of `ben pcompress`: a translator between our formats and a foreign format
    (PCompress). Distinct from a codec, which is internal.

## Dual Graphs

The geographic adjacency graph that gives meaning to a node ordering. Every assignment vector is
interpreted with respect to a particular dual graph: index _i_ is the district id of dual-graph node
_i_.

- **Dual graph**
  - The adjacency graph over geographic units (blocks, VTDs, tracts, precincts). Nodes are units;
    edges are adjacencies. The redistricting term, used in prose.
  - In code-internal contexts (bundle asset names, type names) the bare word "graph" suffices
    because the redistricting context is implicit.
- **NetworkX adjacency format** (or **NX adjacency JSON**)
  - The on-disk JSON shape we read and write for dual graphs (`NxGraphAdjFormat`). Bundle asset
    name: `graph.json`.
  - One of several JSON formats NetworkX itself supports; we pick this one specifically. Avoid the
    ambiguous "graph format" — qualify when format-precise.
- **Node ordering**
  - A permutation of nodes — "which node sits at index 0, 1, 2, ..." Produced by an ordering
    operation; consumed by node reordering.
- **Key-based ordering**
  - Sort nodes by a node attribute. Driven by `sort_json_file_by_key`. Example: sort by `GEOID20`.
- **Topology-based ordering**
  - Sort nodes by graph topology, not by attribute. Driven by `sort_json_file_by_ordering` with an
    **ordering method**.
- **Sort key**
  - The attribute name passed to a key-based ordering (e.g., `"id"`, `"GEOID20"`).
- **Ordering method**
  - The enum value passed to a topology-based ordering. Current options:
    - **MLC** — Multi-Level Clustering. Recursive clustering, applied per connected component.
    - **RCM** — Reverse Cuthill-McKee. Bandwidth-minimization, applied per connected component.
- **Connected component**
  - A maximal connected subgraph. Both MLC and RCM order each component independently and
    concatenate the results.
- **Node permutation map**
  - The data artifact that records a node ordering: a sparse `HashMap<usize, usize>` (or dense
    `Vec<usize>`) mapping new index → old index.
  - The on-disk form is JSON; the bundle stores it as the `node_permutation_map.json` asset.
  - The Rust convention is `new_to_old_node_map` for the sparse form and `Vec<usize>` with
    `perm[new_idx] == old_idx` for the dense form.
- **Sparse vs dense permutation**
  - Sparse: `HashMap<usize, usize>`. The on-disk form. Compact when many nodes are unmoved.
  - Dense: `Vec<usize>`. The fast-lookup form built by `dense_permutation`.
- **Geographic unit**
  - The thing a dual-graph node represents in the real world. Examples: a US Census block, a VTD, a
    tract, a precinct.
  - "Node" (graph term) and "geographic unit" (geography term) are the same object viewed from two
    angles. "Node" is canonical in codec/format/relabel discussions; the specific geography term is
    used when the substrate matters for the discussion.
- **Resolution**
  - The chosen geographic-unit type for an ensemble. Block is the highest resolution; VTD, tract,
    county get progressively lower.
  - A property of the _ensemble_, not of the BEN-family file format.
- **GEOID** / **GEOID20**
  - US Census Bureau identifier strings. A common choice of **sort key** for key-based node
    ordering.

## Relabeling Taxonomy

Three operations historically all called some variant of "relabel." The umbrella "relabel" alone now
means **district relabeling**.

- **District relabeling**
  - Rename the integer values in an assignment vector. A pure value permutation. The plain word
    "relabel" without qualifier means this.
- **Node reordering** (or **node permutation**)
  - Permute the node positions in an assignment vector — rearrange which dual-graph node sits at
    each index. Driven by a **node permutation map** (see Dual Graphs section), which is itself
    produced by sorting the dual graph (key-based or topology-based).
- **Relabel pipeline** (or **relabel machinery**)
  - The codec scaffolding that streams decode → transform → re-encode. Implemented as a single
    driver `relabel_ben_file(reader, writer, options)` parameterised by `RelabelOptions` in
    `ops/relabel/`. Neutral about which transform runs.
- **First-seen relabeling** (or **first-seen district labeling**)
  - The specific district relabeling that renames labels in order of first appearance, starting at
    0\. Replaces the historical "canonicalize_assignment" terminology; code rename pending.
- **The relabel subcommands**
  - `ben relabel` / `canonicalize` / `reencode`: the CLI entry points that run the relabel pipeline
    with one of the canned transforms.

## Disambiguated Terms

Words that historically had multiple meanings; the meanings are now segregated.

### "canonical\*" — three former senses, now two

- **Canonicalized JSONL** — input format conventions: `assignment` and `sample` keys, sample numbers
  from 1, etc. Reserved meaning. Stays.
- **First-seen relabeling** — the operation formerly called "canonicalize_assignment." Loses the
  "canonical" word.
- **Standardized name** — the required filename for a known asset in a bundle (formerly "canonical
  name"). Renamed; code rename pending.

### "header" — qualify always

- **Frame header**: per-frame BEN bytes.
- **Bundle header**: 64-byte BENDL prefix.

### "extract" — qualify always

- **Asset extract**: bundle.
- **Sample-range extract**: BEN file.

### "payload" — qualify always

- **Asset payload**: directory-entry-referenced bytes.
- **Frame payload**: frame-internal bytes after the frame header.

## Bundle Internals

- **Bundle**
  - A `.bendl` file. An instance.
- **Bundle header**
  - The 64 fixed bytes at offset 0.
- **Magic**
  - The 8 leading bytes of the bundle header.
- **Assignment stream**
  - The embedded BEN/XBEN stream — opaque to the bundle layer.
- **Assignment format**
  - The bundle header field saying whether the embedded stream is BEN or XBEN. **Distinct from
    variant** (which lives in the stream's banner).
- **Asset**
  - A directory entry plus its payload bytes. Always an instance.
- **Asset type**
  - The kind of an asset. Values: `{metadata = 1, graph = 2, node_permutation_map = 3, custom = 4}`.
- **Known asset**
  - Type ∈ {1, 2, 3}. Singleton, fixed standardized name, format-defined semantics.
- **Custom asset**
  - Type 4. Writer-chosen name, multiple allowed per bundle.
- **Standardized name**
  - The required filename for a known asset (e.g., `node_permutation_map.json`).
- **Directory** / **directory table**
  - The list of directory entries. The header's `directory_offset` and `directory_len` identify the
    authoritative directory; successful finalization writes it at EOF, while failed post-finalize
    appends may leave newer orphaned bytes after the old authoritative directory.
- **Directory entry**
  - One row of the directory: type + flags + name + offset + length + checksum.
- **Bundle flags**
  - The `flags: u32` field at offset 16 of the bundle header. Bundle-level capabilities.
- **Asset flags**
  - The `asset_flags: u16` field on each directory entry. Per-asset encoding/checksum.
- **Finalize** _(verb)_
  - Write the trailing directory and flip the finalized flag. The terminating step of bundle
    creation.
- **Finalized** _(state)_
  - The bundle's directory and stream lengths are authoritative; safe to read with no recovery
    logic.
- **Incomplete** _(state)_
  - The finalized flag is unset. The directory may be missing; the assignment stream extends to EOF.
- **Provisional directory**
  - An optional pre-stream directory written for crash recovery. Becomes obsolete once finalize
    writes the authoritative trailing directory.
- **Trailing directory** / **authoritative directory**
  - The directory pointed to by the bundle header. The one readers should consult.
- **Post-finalize append**
  - Adding a new asset to a finalized bundle (see **append**).

## Future work

- **Extract disambiguation in the CLI surface.** Today `bendl extract` is the only "extract" verb on
  the CLI side; sample-range extract lives at the library API. If a future change makes both forms
  callable from the same binary, the CLI surface needs to disambiguate per the **asset extract** vs
  **sample-range extract** prose distinction (see Operations and Verbs).
