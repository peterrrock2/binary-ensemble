# Shrink a bundle for sharing

A `.bendl` file you build while sampling usually has an embedded BEN stream in the graph's
original node order — convenient, but not as small as it could be. Before handing it to a
collaborator or archiving it, two steps get it to its smallest form:

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

To return an archived bundle to the faster BEN working format, use the inverse transform:

```python
from binary_ensemble import decompress_stream

decompress_stream("ensemble-archive.bendl", out_file="ensemble-working.bendl")
```

The graph, metadata, permutation map, and custom assets are preserved in both directions.

## In place vs. a new file

All three transforms take an optional `out_file`. Pass one to write a new bundle and leave the
original untouched (`overwrite=True` replaces an existing `out_file`); leave it off to
transform the bundle in place:

```python
relabel_bundle("ensemble.bendl", sort="key", key="GEOID20")
compress_stream("ensemble.bendl")
decompress_stream("ensemble.bendl")
```

The in-place mode writes to a temporary file and swaps it over the original only on success,
so an interrupted run won't corrupt your bundle.

```{tip}
Reorder *before* compressing. Relabeling and node reordering are what create the long runs and
cross-plan repetition that LZMA2 (inside XBEN) exploits, so doing step 1 first makes step 2
dramatically more effective.
```
