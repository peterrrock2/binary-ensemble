# Read and iterate an ensemble

A `.bendl` file is self-describing, so opening one with `BendlDecoder` hands you the whole
package at once: the assignment stream *and* the graph, metadata, and any custom assets stored
beside it. There is no separate graph file to track down and no node order to remember, because
the bundle already carries both.

## Inspect before you iterate

It is worth a quick look at what a bundle holds before committing to a full pass over it. Each of
these reads comes straight from the header or the directory table, so none of them touch the
assignment stream:

```python
from binary_ensemble import BendlDecoder

decoder = BendlDecoder("ensemble.bendl")

print(len(decoder))                 # number of samples (expanded count)
print(decoder.assignment_format())  # 'ben' or 'xben'
print(decoder.asset_names())        # e.g. ['graph.json', 'metadata.json']
print(decoder.read_metadata())      # the metadata.json payload, or None
```

`len()` reads the sample count from the bundle header rather than scanning the stream, so it is
cheap enough to use for a progress-bar total.

## Iterate the assignments

Iterating a decoder yields each plan in turn as a `list[int]`: one district id per node, in graph
order.

```python
for assignment in decoder:
    # assignment is a list[int]: the district id of each node, in graph order
    ...
```

A decoder is reusable. Each `for` loop rewinds to the start of the stream automatically, so there
is no need to reopen the file between passes:

```python
total = len(decoder)
first = next(iter(decoder))      # peek the first plan
all_plans = list(decoder)        # full pass again, from the start
```

That shared cursor is also the one thing to watch: iteration is strictly sequential, so don't
drive two loops over the *same* decoder at once. When you genuinely need two positions in the
stream simultaneously, open a second `BendlDecoder` on the file.

## Recover the dual graph

Because the graph travels inside the bundle, you can rebuild full plan objects without a separate
graph file. `read_graph()` returns a live `networkx.Graph` whose node order matches the order the
assignments were written in, so a plan and its graph line up with no extra bookkeeping:

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

`read_graph()` rebuilds a NetworkX object, which is usually what you want. When you need the
underlying JSON instead, or the permutation map that a reordered bundle carries, reach for the
asset directly:

```python
raw_graph = decoder.read_json_asset("graph.json")      # parsed adjacency dict
permutation_map = decoder.read_node_permutation_map()  # None if the graph wasn't reordered
```

See [Custom assets and appending](custom-assets-and-append.md) for reading arbitrary blobs.
