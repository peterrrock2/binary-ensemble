# Custom assets and appending

A bundle isn't limited to the graph and metadata — you can attach arbitrary named blobs, and
you can add more to a bundle even after it's finalized.

## Attach metadata and custom assets

`add_metadata` writes the canonical `metadata.json`. `add_asset` writes any named blob with a
`content_type` describing the payload:

| `content_type` | Payload | Decoder behavior |
|---|---|---|
| `"json"` | Valid UTF-8 JSON (a `dict`/`list` is serialized for you) | `read_json_asset` auto-parses it |
| `"text"` | Any UTF-8 text | Raw bytes via `read_asset_bytes` |
| `"binary"` | Arbitrary bytes — a zipped shapefile, a GeoPackage, anything | Raw bytes via `read_asset_bytes` |
| `"file"` | A `str` or `Path` naming a file, whose contents are read and stored as binary | Raw bytes via `read_asset_bytes` |

The payload itself may be bytes-like, a `str` (always stored as UTF-8 *content* — a plain
string is never treated as a path except under `content_type="file"`), a `dict`/`list` (JSON
only), an open file or other object with `.read()`, or a `pathlib.Path` whose file is read:

```python
from binary_ensemble import BendlEncoder

encoder = BendlEncoder("ensemble.bendl", overwrite=True)
encoder.add_metadata({"sampler": "ReCom", "seed": 1234})
encoder.add_asset("scores.json", {"mean_cut_edges": 41.2}, content_type="json")
encoder.add_asset("README.txt", "Generated for the 2026 analysis.", content_type="text")
encoder.add_asset("render.png", b"\x89PNG\r\n\x1a\n...", content_type="binary")

with encoder.ben_stream() as ensemble:
    for assignment in [[1, 1, 2, 2], [1, 2, 2, 2]]:
        ensemble.write(assignment)
```

Every asset is stored with a CRC32C integrity checksum, and payloads of 1 KiB or more are
xz-compressed on disk by default — both invisible on read. Assets may be added before *or*
after the stream — only the stream itself is single-use. (The one exception is a *reordering*
`add_graph`, which must come before the stream because it sets the node order the chain
writes in.)

## Read assets back

```python
from binary_ensemble import BendlDecoder

decoder = BendlDecoder("ensemble.bendl")

print(decoder.asset_names())                  # [...'metadata.json', 'scores.json', 'README.txt', 'render.png']
print(decoder.read_json_asset("scores.json")) # {'mean_cut_edges': 41.2}  (parsed)
print(decoder.read_asset_bytes("README.txt")) # b'Generated for the 2026 analysis.'  (raw bytes)
```

Use `read_json_asset` for JSON assets (it parses them) and `read_asset_bytes` for raw bytes of
anything. The canonical getters `read_metadata()`, `read_graph()`, and
`read_node_permutation_map()` are shortcuts for the well-known assets.

To check integrity before trusting a bundle you received, `decoder.verify()` validates every
asset checksum and the stream checksum against the on-disk bytes, raising on any mismatch
(iteration alone does not check checksums):

```python
BendlDecoder("ensemble.bendl").verify()   # raises on corruption or an unfinalized bundle
```

## Append to a finalized bundle

To add assets to a bundle that's already finalized, open it with `BendlEncoder.append`. In
append mode each `add_*` commits immediately, and `ben_stream()` is unavailable (a bundle's
assignment stream is written once):

```python
from binary_ensemble import BendlEncoder

encoder = BendlEncoder.append("ensemble.bendl")
encoder.add_asset("notes.txt", "Reviewed and approved.", content_type="text")
encoder.close()
```

```{note}
Each post-finalize add rewrites the bundle's directory, so it's perfect for a handful of extra
assets but not for tight loops. Attach what you can up front, and use `append` for the
occasional addition after the fact.
```
