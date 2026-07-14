# `binary_ensemble.bundle`

The bundle module is the recommended high-level API. It writes and reads `.bendl` files:
single-file containers that hold an assignment stream plus graph, metadata, permutation
maps, and custom assets.

## When to use it

Use this module when you want the file to be self-describing. That is the normal case for
redistricting ensembles because an assignment is only meaningful with the graph node order it
was written against.

| Task                               | API                                          |
| ---------------------------------- | -------------------------------------------- |
| Create a new bundle                | `BendlEncoder(path, overwrite=True)`         |
| Attach a dual graph                | `encoder.add_graph(graph, sort=...)`         |
| Stream assignments while sampling  | `with encoder.ben_stream() as ensemble: ...` |
| Read assignments and assets        | `BendlDecoder(path)`                         |
| Reorder/relabel an existing bundle | `relabel_bundle(...)`                        |
| Recompress a bundle to XBEN        | `compress_stream(...)`                       |
| Decompress a bundle to BEN         | `decompress_stream(...)`                     |

```python
from binary_ensemble import BendlDecoder

decoder = BendlDecoder("ensemble.bendl")
assert len(decoder) == decoder.count_samples()
assert decoder.assignment_format() in {"ben", "xben"}
```

```{eval-rst}
.. automodule:: binary_ensemble.bundle
```

## Encoder

`BendlEncoder` has two modes:

| Mode   | Open with                            | Stream writes |                            Asset writes |
| ------ | ------------------------------------ | ------------: | --------------------------------------: |
| Create | `BendlEncoder(path, overwrite=True)` |    one stream |              before or after the stream |
| Append | `BendlEncoder.append(path)`          |   unavailable | immediate appends to a finalized bundle |

The stream context finalizes the bundle when it closes cleanly. You only need to use the
encoder itself as a context manager for assets-only bundles or if that style is clearer in
your code.

```python
from binary_ensemble import BendlEncoder

encoder = BendlEncoder("api-demo.bendl", overwrite=True)
encoder.add_metadata({"sampler": "demo"})

with encoder.ben_stream() as ensemble:
    ensemble.write([1, 1, 2, 2])
    ensemble.write([1, 2, 2, 2])
```

### Graph handling

`add_graph()` accepts NetworkX adjacency JSON, a path to that JSON, raw bytes, or a readable
object. By default it preserves the input node order. Pass an explicit `sort` method to reorder
for better compression. Write assignments in the returned NetworkX graph's node order.

| `sort`  | Meaning                                        | Needs `key`? | Stores permutation map? |
| ------- | ---------------------------------------------- | -----------: | ----------------------: |
| `"mlc"` | Multi-level clustering; topology-based         |           no |                     yes |
| `"rcm"` | Reverse Cuthill-McKee topology ordering        |           no |                     yes |
| `"key"` | Sort nodes by a node attribute                 |          yes |                     yes |
| `None`  | Store the graph as-is (default)                |           no |                      no |

```python
import networkx as nx

from binary_ensemble import BendlEncoder

graph = nx.convert_node_labels_to_integers(nx.path_graph(4))
for node in graph.nodes:
    graph.nodes[node]["GEOID20"] = f"{node:04d}"

encoder = BendlEncoder("api-graph.bendl", overwrite=True)
ordered_graph = encoder.add_graph(nx.adjacency_data(graph), sort="key", key="GEOID20")

with encoder.ben_stream() as ensemble:
    ensemble.write([1, 1, 2, 2])

assert ordered_graph.number_of_nodes() == 4
```

```{eval-rst}
.. autoclass:: binary_ensemble.bundle.BendlEncoder
   :members:
```

## The stream session

`BendlEncoder.ben_stream()` returns a `BendlStreamSession`. It is intentionally small: write
assignments, then close. A bundle can have only one assignment stream.

```python
from binary_ensemble import BendlEncoder

encoder = BendlEncoder("api-session.bendl", overwrite=True)
with encoder.ben_stream(variant="twodelta") as ensemble:
    for assignment in [[1, 1, 2, 2], [1, 2, 2, 2]]:
        ensemble.write(assignment)
```

```{eval-rst}
.. autoclass:: binary_ensemble.bundle.BendlStreamSession
   :members:
```

## Decoder

`BendlDecoder` iterates the embedded stream and exposes bundle inspection methods.

| Method                             | Use                                                         |
| ---------------------------------- | ----------------------------------------------------------- |
| `len(decoder)` / `count_samples()` | Expanded number of samples                                  |
| `assignment_format()`              | `"ben"` or `"xben"` for the embedded stream                 |
| `version()` / `is_complete()`      | Bundle header inspection                                    |
| `asset_names()` / `list_assets()`  | Asset directory inspection                                  |
| `verify()`                         | Check every asset and stream checksum; raises on corruption |
| `read_graph()`                     | `networkx.Graph` rebuilt from `graph.json`, or `None`       |
| `read_metadata()`                  | Parsed `metadata.json`, or `None`                           |
| `read_node_permutation_map()`      | Parsed permutation map, or `None`                           |
| `read_json_asset(name)`            | Parse a JSON asset                                          |
| `read_asset_bytes(name)`           | Raw bytes for any asset                                     |
| `extract_stream(path)`             | Copy the embedded stream out as `.ben` or `.xben` bytes     |
| `subsample_*()`                    | Iterate only selected samples                               |

```python
from binary_ensemble import BendlDecoder

decoder = BendlDecoder("ensemble.bendl")

print(decoder.asset_names())
print(decoder.read_metadata())

for assignment in decoder.subsample_range(1, 3):
    print(assignment[:4])
```

Iteration rewinds on a fresh `for` loop. Do not drive two simultaneous loops from the same
decoder object; open a second decoder if you need independent cursors.

```{eval-rst}
.. autoclass:: binary_ensemble.bundle.BendlDecoder
   :members:
```

## Whole-bundle transforms

These functions preserve bundle assets while rewriting the embedded stream.

```python
from binary_ensemble import compress_stream, decompress_stream, relabel_bundle

relabel_bundle("ensemble.bendl", out_file="api-sorted.bendl", sort="mlc")
compress_stream("api-sorted.bendl", out_file="api-archive.bendl")
decompress_stream("api-archive.bendl", out_file="api-working.bendl")
```

Each transform takes an optional `out_file`: pass one to create a new file (`overwrite=True`
replaces an existing one), or leave it off to atomically replace the input in place.

`compress_stream()` is a whole-bundle transform: it changes the embedded BEN stream to XBEN
while retaining the graph, metadata, and other assets. `decompress_stream()` is its inverse,
changing XBEN back to BEN for faster day-to-day access while retaining the same assets.

```{eval-rst}
.. autofunction:: binary_ensemble.bundle.compress_stream

.. autofunction:: binary_ensemble.bundle.decompress_stream

.. autofunction:: binary_ensemble.bundle.relabel_bundle
```
