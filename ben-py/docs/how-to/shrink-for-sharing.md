# Shrink a bundle for sharing

A bundle you build while sampling is usually a BEN bundle in the graph's original node order —
convenient, but not as small as it could be. Before handing it to a collaborator or archiving
it, two steps get it to its smallest form:

1. **Relabel and reorder** so assignments form long runs and equivalent plans encode
   identically.
2. **Recompress** the stream to XBEN.

## Step 1: relabel and reorder

`relabel_bundle` reorders the embedded graph, rewrites every assignment into the new node
order, and stores the reversible permutation map — all while preserving your metadata and
custom assets:

```python
from binary_ensemble import relabel_bundle

# Sort by a geographic key (often the most effective ordering). Use sort="mlc" or
# sort="rcm" for a topology-based ordering instead.
relabel_bundle("ensemble.bendl", out_file="ensemble-sorted.bendl", sort="key", key="GEOID20")
```

See [Why reordering shrinks files](../concepts/compression.md) for what `mlc`, `rcm`, and
`key` do.

## Step 2: recompress to XBEN

`compress_stream` re-encodes the bundle's BEN stream as XBEN, carrying every asset across
unchanged:

```python
from binary_ensemble import compress_stream

compress_stream("ensemble-sorted.bendl", out_file="ensemble-archive.bendl")
```

The result is a single `.bendl` that's typically orders of magnitude smaller — and still
self-describing, since the graph and permutation map travel inside it.

## In place vs. a new file

Both transforms take **either** `out_file` (write a new bundle) **or** `in_place=True`
(atomically replace the original). Passing both, or neither, raises:

```python
relabel_bundle("ensemble.bendl", in_place=True, sort="key", key="GEOID20")
compress_stream("ensemble.bendl", in_place=True)
```

`in_place=True` writes to a temporary file and swaps it over the original only on success, so
an interrupted run won't corrupt your bundle.

```{tip}
Reorder *before* compressing. Relabeling and node reordering are what create the long runs and
cross-plan repetition that LZMA2 (inside XBEN) exploits, so doing step 1 first makes step 2
dramatically more effective.
```
