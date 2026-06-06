# Read and iterate an ensemble

Open a `.bendl` bundle with `BendlDecoder` and you get the assignment stream *and* everything
the bundle carries alongside it.

## Inspect before you iterate

```python
from binary_ensemble import BendlDecoder

decoder = BendlDecoder("ensemble.bendl")

print(len(decoder))                 # number of samples (expanded count)
print(decoder.assignment_format())  # 'ben' or 'xben'
print(decoder.asset_names())        # e.g. ['graph.json', 'metadata.json']
print(decoder.read_metadata())      # the metadata.json payload, or None
```

`len()` is cheap and cached, so it's safe to use for a progress bar.

## Iterate the assignments

```python
for assignment in decoder:
    # assignment is a list[int]: the district id of each node, in graph order
    ...
```

You can iterate the same decoder as many times as you like — each `for` loop rewinds to the
start of the stream automatically, so there's no need to reopen the file:

```python
total = len(decoder)
first = next(iter(decoder))      # peek the first plan
all_plans = list(decoder)        # full pass again, from the start
```

The cursor is shared, so this is sequential re-iteration — don't drive two loops over the
*same* decoder at once. If you need two independent positions simultaneously, open a second
`BendlDecoder`.

## Recover the dual graph

Because the graph is embedded, you can rebuild full plan objects without a separate graph
file. `read_graph()` returns a live `networkx.Graph` whose node order matches the order the
assignments were written in:

```python
import pandas as pd
from gerrychain import Partition

decoder = BendlDecoder("ensemble.bendl")
graph = decoder.read_graph()
node_order = pd.Index(graph.nodes)

for assignment in decoder:
    series = pd.Series(assignment, index=node_order)
    partition = Partition(graph, assignment=series)
    # ... analyze the partition (cut edges, population, scores, ...)
```

## Get the raw graph or permutation map

`read_graph()` hands back a NetworkX graph; for the underlying JSON, or for a reordered
bundle's permutation map, use:

```python
raw_graph = decoder.read_json_asset("graph.json")      # parsed adjacency dict
permutation_map = decoder.read_node_permutation_map()  # None if the graph wasn't reordered
```

See [Custom assets and appending](custom-assets-and-append.md) for reading arbitrary blobs.
