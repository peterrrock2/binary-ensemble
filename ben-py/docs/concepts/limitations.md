# Limitations and invariants

This page is intentionally blunt. `binary-ensemble` is designed for a specific data shape:
large streams of district-assignment vectors over a fixed dual graph. It is very good at
that job, but it does not try to be a general geospatial archive format.

For concrete examples of what not to do, see [Anti-patterns](../how-to/anti-patterns.md).

## Assignment-only streams

Plain `.ben` and `.xben` files store only assignment streams. They do not store:

- the dual graph,
- node attributes,
- sampler settings,
- per-plan scores,
- provenance metadata.

Use `.bendl` when that context must travel with the assignments.

## One graph order per stream

A stream represents one ensemble over one fixed node order. Every assignment in the stream
must have the same length and the same positional meaning.

```python
from binary_ensemble import BendlDecoder

decoder = BendlDecoder("ensemble.bendl")
graph = decoder.read_graph()
first_assignment = next(iter(decoder))

assert graph is None or graph.number_of_nodes() == len(first_assignment)
```

If the graph order is wrong, decoding still succeeds because integer vectors are still valid.
The resulting plans are wrong, not unreadable.

## One stream per bundle

A `.bendl` bundle carries one assignment stream. You can append assets after finalization, but
you cannot append more samples or add a second stream.

```python
from binary_ensemble import BendlEncoder

encoder = BendlEncoder.append("ensemble.bendl")
encoder.add_asset("notes.txt", "post-run note", content_type="text")
encoder.close()
```

## XBEN is not the working format

XBEN is optimized for storage size, not write speed. Compression can be slow on block-level
ensembles, especially at high compression levels. Use BEN while sampling, iterating, and
subsampling; recompress to XBEN once the file is ready to share.

## Relabel before XBEN

`relabel_bundle()` expects a `.bendl` bundle with an embedded BEN stream and graph. Run it
before `compress_stream()`.

```python
from binary_ensemble import compress_stream, relabel_bundle

relabel_bundle("ensemble.bendl", out_file="limited-sorted.bendl", sort="mlc")
compress_stream("limited-sorted.bendl", out_file="limited-archive.bendl")
```

## District ids are integers

Assignments store integer district ids. The practical limit is 16-bit positive district ids,
which is far above normal statewide redistricting use. Non-integer labels should be mapped to
integers before encoding.

## No geospatial geometry

Bundles can store graph JSON and custom text or JSON assets, but they do not embed arbitrary
geospatial file trees by default. Store geometry paths, hashes, and provenance in metadata, or
ship the geometry separately when readers need it.
