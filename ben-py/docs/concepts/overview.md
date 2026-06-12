# Overview

## The problem

A redistricting **sampler** — GerryChain's ReCom, ForestReCom, a Sequential Monte Carlo
routine — explores the space of legal districting plans by emitting a long sequence of
plans. Serious analyses want *many* plans: tens of thousands to millions.

The natural way to store them is [JSONL](https://jsonlines.org) (JSON Lines), one plan per
line:

```
{"assignment": [1, 1, 2, 2, 3, 3, ...], "sample": 1}
{"assignment": [1, 1, 2, 2, 3, 1, ...], "sample": 2}
...
```

This is simple and portable, but it does not scale. A 100,000-plan ensemble on Colorado's
~140,000 census blocks is **27 GB** of JSONL. Most of that is redundancy: each assignment
is mostly long runs of the same district id, and consecutive plans differ only slightly.

## What BEN does

**BEN** (Binary-Ensemble) is a binary format that wrings out that redundancy. The core
compression is deliberately simple and works in two stages:

1. **Run-length encoding (RLE)** — `[1, 1, 1, 2, 2, 2, 2, 3]` becomes
   `[(1, 3), (2, 4), (3, 1)]`. Districting plans are mostly long runs, so this is a big win,
   especially when nearby geographic units sit next to each other in the node ordering.
2. **Bit-packing** — each run's value and length are stored in the minimum number of bits,
   not padded out to whole bytes.

On top of that, the **XBEN** format adds LZMA2 compression to exploit the repetition *across*
plans, and several **encoding variants** specialize for how a particular sampler produces its
plans.

```{admonition} The headline result
:class: tip
That 27 GB Colorado JSONL ensemble, reordered by `GEOID20`, becomes a **~550 MB** BEN stream,
and then a **~6 MB** XBEN file — a **>4500×** reduction, completely lossless. The biggest
single lever is *node reordering*; see [Why reordering shrinks files](compression.md).
```

## The format family

BEN comes as three on-disk **containers**, each suited to a different job:

| Container | What it is | Use it for |
|-----------|-----------|------------|
| `.ben`    | A plain BEN **stream** | Working with an ensemble: reading, replaying, subsampling |
| `.xben`   | A BEN stream wrapped in LZMA2 | Long-term storage and transferring ensembles |
| `.bendl`  | A **bundle**: a BEN/XBEN stream plus the dual graph and metadata | The recommended default — one self-describing file |

[Formats: BEN vs XBEN vs BENDL](formats.md) covers the trade-offs in detail.

## How the Python API is organized

The Python package mirrors the project's CLI tools:

- **{mod}`binary_ensemble.bundle`** — read and write `.bendl` files (start here).
- **{mod}`binary_ensemble.stream`** — read and write plain `.ben`/`.xben` streams.
- **{mod}`binary_ensemble.codec`** — convert whole files between JSONL, BEN, and XBEN.
- **{mod}`binary_ensemble.graph`** — reorder a dual graph before encoding.

See [The API map](api-map.md) for when to reach for each, and the
[Vocabulary](vocabulary.md) page for the precise meaning of *plan*, *assignment*,
*sample*, and *ensemble*.

For the invariants that must hold across a real run — assignment length, graph node order,
JSONL shape, and bundle assets — see [The data contract](data-model.md).

For operational guidance after the basics, see [Performance guide](performance.md),
[Graph ordering deep dive](ordering-deep-dive.md), [Limitations and invariants](limitations.md),
and [Compatibility and stability](compatibility.md).
