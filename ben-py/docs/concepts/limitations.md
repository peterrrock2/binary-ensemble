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

A `.bendl` file carries one assignment stream. You can append assets after finalization, but
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

`relabel_bundle()` expects a `.bendl` file with an embedded BEN stream and graph. Run it
before `compress_stream()`.

```python
from binary_ensemble import compress_stream, relabel_bundle

relabel_bundle("ensemble.bendl", out_file="limited-sorted.bendl", sort="mlc")
compress_stream("limited-sorted.bendl", out_file="limited-archive.bendl")
```

## District ids are integers

Assignments store integer district ids in the range 0–65535 (16-bit), which is far above
normal statewide redistricting use. Non-integer labels should be mapped to integers before
encoding; values outside the 16-bit range raise an `OverflowError` at write time.

## Geospatial data travels as opaque blobs

Bundles can carry geospatial data — a zipped shapefile, a GeoPackage, a GeoJSON file — as
custom binary assets. The payload is stored verbatim with a CRC32C integrity checksum
(xz-compressed on disk when it is 1 KiB or larger, transparently decompressed on read):

```python
from pathlib import Path

from binary_ensemble import BendlDecoder, BendlEncoder

# Stand-in for a real geometry file, e.g. one produced by geopandas.
gpkg_bytes = b"GPKG\x00\x01" + bytes(range(256))
Path("tracts.gpkg").write_bytes(gpkg_bytes)

encoder = BendlEncoder("with_geometry.bendl", overwrite=True)
encoder.add_asset("tracts.gpkg", Path("tracts.gpkg"), content_type="file")
encoder.close()

decoder = BendlDecoder("with_geometry.bendl")
assert decoder.read_asset_bytes("tracts.gpkg") == gpkg_bytes
```

What the bundle does **not** do is interpret the geometry: there is no spatial indexing, no
geometry validation, and — most importantly — no enforcement that the geometry's feature order
matches the dual graph's node order. That correspondence is the caller's responsibility, exactly
as it is for the graph itself. For large geometry collections that several bundles share, storing
paths, hashes, and provenance in metadata and shipping the geometry separately is still often the
better layout — embedding is a convenience for self-contained archives, not a requirement.
