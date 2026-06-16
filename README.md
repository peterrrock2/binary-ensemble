# Binary Ensemble (BEN)

**Compress, store, and stream massive ensembles of districting plans.**

Redistricting samplers like [GerryChain](https://gerrychain.readthedocs.io)'s ReCom,
ForestReCom, and Sequential Monte Carlo routinely emit millions of plans. Stored as JSONL, a
single ensemble can run to tens of gigabytes — most of it redundant, because each plan is
mostly long runs of the same district id and consecutive plans barely differ. BEN is a
compression format and toolkit built for exactly this data. It was built as an analogue to
[PCompress](https://github.com/mggg/pcompress) and interoperates with it.

> A real 50k-plan ensemble on Colorado's ~140k census blocks is **13.5 GB** as JSONL.
> Relabeled and sorted by `GEOID20` it becomes a **~280 MB** BEN file, and then a **5.6 MB**
> XBEN file — over a **2,400× reduction**, fully lossless.

The expected input is canonicalized JSONL, one plan per line:

```
{"assignment": [1, 1, 2, 2, ...], "sample": 1}
{"assignment": [1, 2, 2, 2, ...], "sample": 2}
```

## The format family

| Format   | What it is                                                                                        | Use it for                                                       |
| -------- | ------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------- |
| `.ben`   | Bit-packed, run-length-encoded stream                                                             | Working with an ensemble: reading, replaying, subsampling        |
| `.xben`  | A BEN stream further compressed with LZMA2                                                        | Long-term storage and transfer (slow to create, fast to extract) |
| `.bendl` | A BEN or XBEN stream plus the dual graph, metadata, and custom assets in one self-describing file | Sharing an ensemble without losing its context                   |

The byte-level layouts are specified in [`docs/`](./docs):
[BEN/XBEN](./docs/ben-format-spec.md) · [TwoDelta variant](./docs/twodelta-format-spec.md) ·
[BENDL](./docs/bendl-format-spec.md) · [format stability policy](./docs/format-stability.md).

## What's in the repository

| Component               | What it does                                                                                                                                                                         |
| ----------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| [`ben/`](./ben)         | The Rust crate ([`binary-ensemble` on crates.io](https://crates.io/crates/binary-ensemble)) and the CLI tools below                                                                  |
| `ben` (CLI)             | Encode/decode between JSONL, BEN, and XBEN; random-access lookup; relabel/canonicalize/reencode to compress better; convert to/from [PCompress](https://github.com/mggg/pcompress)   |
| `bendl` (CLI)           | Create, inspect, extract from, and append assets to `.bendl` files                                                                                                                   |
| [`ben-py/`](./ben-py)   | The Python package ([`binary-ensemble` on PyPI](https://pypi.org/project/binary-ensemble/)) — full docs at [binary-ensemble.readthedocs.io](https://binary-ensemble.readthedocs.io/) |
| [`docs/`](./docs)       | Format specifications, stability policy, and project glossary                                                                                                                        |
| [`example/`](./example) | Small sample files used throughout this README                                                                                                                                       |
| [`fuzz/`](./fuzz)       | Fuzz targets for the readers and writers                                                                                                                                             |

## Install

Command-line tools (installs `ben` and `bendl`):

```bash
cargo install binary-ensemble    # from crates.io
cargo install --path ben         # from a repository checkout
```

Python package:

```bash
pip install binary-ensemble
```

## CLI quick start

Using [`example/small_example.jsonl`](./example/small_example.jsonl):

```bash
ben encode small_example.jsonl                 # -> small_example.ben
ben xencode small_example.ben                  # -> small_example.xben
ben decode small_example.xben -w               # XBEN -> BEN (one layer down)
ben decode small_example.ben -o roundtrip.jsonl     # BEN -> JSONL
ben lookup small_example.ben -n 4              # prints sample 4: [1, 1, 1, 2, ...]
```

`ben` also has `xdecode` (XBEN straight to JSONL) and general-purpose `xz-compress` /
`xz-decompress` subcommands. The `--variant` flag selects the frame encoding (`standard`,
`mkvchain`, or `twodelta`); readers detect the variant automatically, so it is only ever
specified when encoding. Lookup is fastest on plain `.ben` files: `standard`/`mkvchain`
frames can be skipped directly, and `twodelta` lookup seeks to the latest snapshot checkpoint
before replaying deltas.

### One self-describing file: `bendl`

A plain stream is just assignments — it is meaningless without the dual graph that defines
its node order. A `.bendl` file keeps the stream and that context together:

```bash
bendl create -i small_example.ben -o run.bendl --metadata meta.json
bendl inspect run.bendl                        # header, sample count, asset directory
bendl append run.bendl --asset notes.txt=notes.txt
bendl extract run.bendl --asset metadata.json -o meta-out.json
bendl extract run.bendl --stream -o extracted.ben
```

`create` and `append` also take `--graph` and `--node-permutation-map` for the standardized
assets. Asset payloads are checksummed (CRC32C) and xz-compressed on disk by default.

## Making files smaller: relabeling

BEN's core compression is run-length encoding, so anything that produces longer runs of the
same district id shrinks the files dramatically. `ben` provides the two big levers:

1. **First-seen relabeling (`ben canonicalize`).** `[2,2,3,3,1,1]` and `[0,0,1,1,2,2]` are
   the same partition with different labels, but the XBEN compressor cannot know that.
   Renumbering districts in order of first appearance (starting at 0) makes equivalent plans
   encode identically:

   ```bash
   ben canonicalize 50k_CO_chain.ben
   # rewrites 50k_CO_chain.ben in place
   # (pass --output-file to write elsewhere, or --add-suffix for
   #  50k_CO_chain_first_seen_relabeled.ben)
   ```

2. **Node reordering (`ben relabel`).** Nearby geographic units tend to share a district, so
   sorting the dual graph's nodes by a geographic key (or a topology-based ordering via
   `--ordering mlc` / `--ordering rcm`) turns each plan into a handful of long runs:

   ```bash
   ben relabel 50k_CO_chain.ben -d CO_small.json -k GEOID20 -o 50k_CO_chain_sorted.ben
   # -> 50k_CO_chain_sorted.ben               (the rewritten ensemble)
   # -> CO_small_sorted_by_GEOID20.json       (the reordered dual graph)
   # -> CO_small_sorted_by_GEOID20_map.json   (the reversible permutation map)
   ```

   Without `-o`/`--output-file` the relabeled ensemble lands at
   `50k_CO_chain_sorted_by_GEOID20.ben` next to the input.

To switch BEN variant without relabeling, use `ben reencode --output-variant <variant>`.

On the Colorado example ([`example/CO_small.json`](./example/CO_small.json), with the
50k-plan ensemble in [`example/50k_CO_chain.xben`](./example/50k_CO_chain.xben)),
this takes the BEN file from ~3.6 GB to ~280 MB, and the final `ben xencode` brings it to
5.6 MB.

**A note on speed:** XBEN _decoding_ is fast — a large file extracts in minutes. High-ratio
XBEN _encoding_ is slow (an hour or more for block-level ensembles; ~10 minutes for VTD-level
ones). Encode to XBEN once for storage; work against BEN day to day.

## Python

The Python package wraps the same engine and adds an ergonomic streaming API:

```python
from binary_ensemble import BendlEncoder, BendlDecoder

plans = [[1, 1, 2, 2], [1, 2, 2, 2], [1, 1, 1, 2]]

encoder = BendlEncoder("ensemble.bendl", overwrite=True)
encoder.add_metadata({"sampler": "demo", "seed": 1234})
with encoder.ben_stream() as ensemble:
    for assignment in plans:
        ensemble.write(assignment)

for assignment in BendlDecoder("ensemble.bendl"):
    print(assignment)
```

See [binary-ensemble.readthedocs.io](https://binary-ensemble.readthedocs.io/) for the
quickstart, concept guides, how-to recipes (including streaming a GerryChain run straight
into a `.bendl` file), and the full API reference.

## How the compression works

A BEN stream encodes each assignment vector in two stages:

1. **Run-length encoding.** `[1,1,1,2,2,2,2,3,1,3,3,3]` becomes
   `[(1,3), (2,4), (3,1), (1,1), (3,3)]` — districting plans on a well-ordered graph are
   mostly long runs.
2. **Bit-packing.** Each frame stores its values and lengths at the minimum bit width its
   own maxima require. The example above has a max id of 3 (2 bits) and a max run length of
   4 (3 bits), so the frame is:

   ```
   00000010                             <- bits per district id (2)
   00000011                             <- bits per run length (3)
   00000000 00000000 00000000 00000100  <- payload byte length (4, big-endian u32)
   01011_10100_11001_01001_11011_0000000  <- the five runs, 5 bits each, zero-padded
   ```

XBEN then runs LZMA2 over the stream to exploit the repetition _across_ plans. Bit-packed
frames don't line up byte-for-byte, so the encoder first re-expands them into a byte-aligned
intermediate form (BEN32, one `(value, length)` pair per 4 bytes) that LZMA2 can deduplicate
— one frame at a time, so memory stays flat. This is also why first-seen relabeling helps:
structurally identical plans become byte-identical, and LZMA2 collapses them.

The `ben pcompress` subcommand converts between BEN and PCompress (`ben pcompress from-ben /
to-ben / to-xben <file>`), so ensembles can move between the two ecosystems. Both formats use
0-based district ids, so ids transcode unchanged.

## Assumptions and limitations

- **Node order is the contract.** Assignments are stored in dual-graph node order; the
  format cannot detect a mismatched graph. Decoding a stream against the wrong node order
  yields valid-looking but wrong plans — which is exactly why `.bendl` files embed the
  graph.
- **Samples are 1-indexed**, and decoded ensembles always start at sample 1.
- **District ids and run lengths are 16-bit** (ids 0–65535, run lengths 1–65535) — far
  beyond any statewide redistricting use.
- A machine running the codecs only needs memory for one assignment vector at a time.
- BEN shines on long assignment vectors (census blocks). On coarser units (VTDs, tracts,
  ~10–20k nodes) the ratios are more modest; for exceptionally small MCMC ensembles on
  coarse units, [PCompress](https://github.com/mggg/pcompress)'s byte-level delta encoding
  is a good alternative.

## Testing and format stability

A compression format's worst failure mode is decoding _silently wrong_, so the test policy
is built around the wire formats rather than just the code:

- **Golden fixtures.** Byte-exact reference files for every format and variant — including
  one minted by the real PCompress encoder for interop — are committed under
  [`ben/tests/fixtures/`](./ben/tests/fixtures). Any later version of the readers must
  accept them unchanged; format additions mint new fixtures instead of rewriting old ones.
  The rules are spelled out in the [format stability policy](./docs/format-stability.md).
- **Mutation tests.** Every committed fixture is re-read under exhaustive single-byte
  mutations: corruption must fail loudly, never decode into plausible-looking data.
- **Property-based tests** (proptest) check encode/decode round-trips, boundary conditions,
  and that the high-level operations agree with their naive reference implementations.
- **Streaming soak test.** An encoder thread pipes a multi-gigabyte logical ensemble into a
  decoder while the test asserts peak memory stays bounded — pinning the "streaming, not
  slurping" invariant that nothing else in the suite would catch.
- **Coverage-guided fuzzing** ([`fuzz/`](./fuzz)) of the four read surfaces (BEN, XBEN
  container and body, BENDL), since readers must survive arbitrary untrusted bytes.
- **Big-endian suite.** The full Rust suite also runs on s390x under QEMU, because the
  formats are byte-order-sensitive and an endianness bug would corrupt silently.
- **Docs are tests.** Every Python code snippet in the documentation executes under pytest,
  and the tutorial notebooks run end to end in CI with warnings as errors.

CI runs format and lint checks on every push; the heavy gates (full Rust + Python suites,
big-endian, time-boxed fuzzing) run on demand via `/ci-full`, `/ci-endian`, and `/ci-fuzz`
PR comments or a manual workflow dispatch. Release builds produce wheels for Linux, macOS,
and Windows (including ARM), smoke-testing every wheel that can execute on its build
runner. Locally, the same gates are exposed as `task test`, `task fuzz`, `task test-endian`,
and `task coverage-summary`.

## License

MIT — see [LICENSE](./LICENSE.md).
