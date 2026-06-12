# The data contract

`binary-ensemble` is intentionally small at the storage layer: it stores integer
assignments, optional graph assets, and optional metadata. The important part is making sure
those pieces agree. This page spells out the contract that every encoder, decoder, and
converter assumes.

## Assignment shape

An assignment is a plain sequence of integers:

```python
assignment = [1, 1, 2, 2, 3, 3]
```

The list position is the graph node position. The value is the district id assigned to that
node.

| Rule | Why it matters |
|---|---|
| Every assignment in one stream must have the same length. | A stream represents one ensemble over one fixed dual graph. |
| Values must be district ids in `0..=65535` (16-bit). | The binary format stores district ids compactly. |
| The order must match the graph order you intend to use when reading. | BEN cannot infer geographic meaning from the values alone. |
| Missing nodes are not represented. | Use one entry per graph node, even for islands or zero-population units. |

```python
from binary_ensemble import BendlDecoder

decoder = BendlDecoder("ensemble.bendl")
graph = decoder.read_graph()
first_assignment = next(iter(decoder))

assert graph is None or len(first_assignment) == graph.number_of_nodes()
```

## Node order

Node order is the most important invariant in the system. Assignment index `i` means "the
node at position `i` in the dual graph." If you change the graph order without changing the
assignment order, the file still decodes successfully but describes the wrong plans.

Bundles are the recommended default because they keep the graph and assignment stream
together:

```python
from binary_ensemble import BendlDecoder

decoder = BendlDecoder("ensemble.bendl")
graph = decoder.read_graph()      # NetworkX graph in assignment order, or None
metadata = decoder.read_metadata()
```

When `BendlEncoder.add_graph(..., sort="mlc")`, `sort="rcm"`, or `sort="key"` reorders a
graph, it returns the reordered graph. Write assignments in that returned order, and store
the permutation map the bundle creates for you.

```python
import networkx as nx

from binary_ensemble import BendlEncoder

graph = nx.convert_node_labels_to_integers(nx.path_graph(4))
adjacency = nx.adjacency_data(graph)

encoder = BendlEncoder("ordered.bendl", overwrite=True)
ordered_graph = encoder.add_graph(adjacency, sort="rcm")
write_order = list(ordered_graph.nodes)
assert len(write_order) == 4

with encoder.stream() as stream:
    stream.write([1, 1, 2, 2])
```

```{warning}
Reordering the graph is lossless, but it is not cosmetic. Once you choose an order, every
assignment written to that stream must use that exact order.
```

## JSONL input

The whole-file codec helpers expect JSON Lines with one JSON object per line and an
`assignment` field:

```json
{"assignment": [1, 1, 2, 2], "sample": 1}
{"assignment": [1, 2, 2, 2], "sample": 2}
```

Extra fields such as `sample`, scores, or metadata can be present in the input JSONL, but
only the assignment stream is encoded into `.ben` or `.xben`. If you need graph metadata,
sampler settings, or scores to travel with the file, put the stream in a `.bendl` file and
attach those payloads as assets.

```python
from binary_ensemble import encode_jsonl_to_ben

encode_jsonl_to_ben("plans.jsonl", "plans.ben", overwrite=True)
```

## Bundle assets

A `.bendl` file can carry well-known assets and custom assets:

| Asset | Reader helper | Typical payload |
|---|---|---|
| `graph.json` | `read_graph()` or `read_json_asset("graph.json")` | NetworkX adjacency JSON |
| `metadata.json` | `read_metadata()` | Sampler name, seed, date, chain settings |
| `node_permutation_map.json` | `read_node_permutation_map()` | Reversible old-to-new node order map |
| Custom JSON/text/binary asset | `read_json_asset()` or `read_asset_bytes()` | Scores, notes, provenance, run manifests, geometry blobs |

```python
from binary_ensemble import BendlDecoder

decoder = BendlDecoder("ensemble.bendl")

for asset in decoder.list_assets():
    print(asset["name"], asset["type"], asset["flags"])
```

## Variants and formats

The data contract is independent of the stream variant. `standard`, `mkv_chain`, and
`twodelta` all decode to the same thing: a sequence of `list[int]` assignments. Choose the
variant for compression behavior, not for downstream semantics.

Likewise, `.ben`, `.xben`, and `.bendl` carry the same assignment stream at different
packaging layers:

| Format | Carries assignments | Carries graph/assets | Best use |
|---|---:|---:|---|
| `.ben` | yes | no | Active work, fast streaming and subsampling |
| `.xben` | yes | no | Small plain-stream archive |
| `.bendl` with BEN stream | yes | yes | Recommended working bundle |
| `.bendl` with XBEN stream | yes | yes | Recommended share/archive bundle |

## Validation checklist

Before you encode a real ensemble, check these points:

- Decide the node order once, before the first sample is written.
- If you reorder a graph, run the sampler or assignment extraction in the reordered graph's
  node order.
- Keep assignment length constant for the whole stream.
- Store the graph in the bundle unless every reader already has the exact matching graph.
- Store sampler settings, random seed, scoring definitions, and provenance as metadata or
  custom assets.
- Use BEN while sampling and iterating; recompress to XBEN when the file is ready to share.
