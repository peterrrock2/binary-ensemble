# Quickstart

This page takes you from zero to a compressed, self-describing ensemble in a few minutes.
If a term is unfamiliar, the [Concepts](../concepts/overview.md) section explains the model
behind the API.

## The one thing to know

A districting plan is represented as an **assignment**: a flat list of integers, one per
node of a dual graph, giving the district id of each node.

```python
assignment = [1, 1, 2, 2]   # nodes 0 and 1 are in district 1; nodes 2 and 3 in district 2
```

An **ensemble** is just a sequence of these. `binary-ensemble` compresses that sequence.

## Write an ensemble

The recommended container is a **`.bendl` file** — a single self-describing file. Open a
`BendlEncoder`, attach any metadata, then write assignments through a `stream` context that
finalizes the bundle when it closes:

```python
from binary_ensemble import BendlEncoder

plans = [[1, 1, 2, 2], [1, 2, 2, 2], [1, 1, 1, 2]]

encoder = BendlEncoder("ensemble.bendl", overwrite=True)
encoder.add_metadata({"sampler": "demo", "seed": 1234})
with encoder.stream() as stream:
    for assignment in plans:
        stream.write(assignment)
# bundle is finalized here
```

## Read it back

Open a `BendlDecoder` and iterate. The bundle knows how many samples it holds and what it
carries:

```python
from binary_ensemble import BendlDecoder

decoder = BendlDecoder("ensemble.bendl")

print(len(decoder))             # 3
print(decoder.asset_names())    # ['metadata.json']
print(decoder.read_metadata())  # {'sampler': 'demo', 'seed': 1234}

for assignment in decoder:
    print(assignment)
```

## Make it self-describing

The real value of a bundle is embedding the **dual graph** so a collaborator can open the
file without hunting down the matching graph JSON. `add_graph` accepts a graph in NetworkX
*adjacency* form (a `dict`) or a path to a graph JSON file:

```python
import networkx as nx
from binary_ensemble import BendlEncoder, BendlDecoder

graph = nx.grid_2d_graph(2, 2)
graph = nx.convert_node_labels_to_integers(graph)
adjacency = nx.adjacency_data(graph)          # the dict shape add_graph expects

encoder = BendlEncoder("ensemble.bendl", overwrite=True)
encoder.add_graph(adjacency, sort=None)       # store as-is; see below for reordering
with encoder.stream() as stream:
    for assignment in [[1, 1, 2, 2], [1, 2, 2, 2]]:
        stream.write(assignment)

decoder = BendlDecoder("ensemble.bendl")
graph = decoder.read_graph()                  # back as a live NetworkX graph
print(graph.number_of_nodes(), "nodes")
```

```{tip}
Passing `sort="rcm"` or `sort="mlc"` instead of `sort=None` reorders the graph's nodes for
much better compression and records a reversible permutation map. See
[Why reordering shrinks files](../concepts/compression.md).
```

## Already have JSONL files?

If your sampler already wrote a JSONL ensemble, the `codec` helpers convert whole files in
one call — no iteration required:

```python
from binary_ensemble import encode_jsonl_to_ben, encode_ben_to_xben, decode_ben_to_jsonl

encode_jsonl_to_ben("plans.jsonl", "plans.ben")        # JSONL -> BEN (fast, working format)
encode_ben_to_xben("plans.ben", "plans.xben")          # BEN -> XBEN (smallest, for storage)
decode_ben_to_jsonl("plans.ben", "plans_again.jsonl")  # round-trip back to JSONL
```

## Next steps

- [Compress a GerryChain run](../how-to/compress-gerrychain-run.md) — the most common workflow.
- [Subsample a large ensemble](../how-to/subsample.md) without decoding the whole thing.
- [Concepts](../concepts/overview.md) — formats, encoding variants, and how the compression works.
