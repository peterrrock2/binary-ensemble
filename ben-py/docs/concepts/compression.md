# Why reordering shrinks files

BEN's base compression is run-length encoding plus bit-packing. RLE turns an assignment into
`(value, length)` pairs:

```
[1, 1, 1, 2, 2, 2, 2, 3]   ->   [(1, 3), (2, 4), (3, 1)]
```

The fewer, longer runs an assignment has, the smaller it gets. So **anything that produces
longer runs makes the files dramatically smaller**. There are two levers for that, and they
are where almost all the savings come from.

## Lever 1: node reordering (the big one)

Nearby geographic units tend to land in the same district. If the dual graph's **node
ordering** keeps neighbors adjacent, assignments collapse into a handful of long runs instead
of many short ones.

### Why a good node order creates long runs

RLE only shrinks an assignment when _consecutive_ nodes share a district id. And districts
aren't random scatterings of nodes — they're **contiguous, densely-connected regions** of the
dual graph. So a run appears wherever the node order happens to place nodes from the same
district side by side, and the longer those stretches, the fewer runs the assignment needs.

That reframes the goal: order the nodes so that **nodes likely to share a district sit next to
each other**. Every assignment in the ensemble is read in that _one_ order, so a good order
pays off across the entire ensemble at once — and because the runs become longer and more
regular, the byte patterns _across_ plans get more repetitive too, which feeds the XBEN
(LZMA2) stage on top (Lever 2).

The three orderings below are different heuristics for that same goal. All of them are
**lossless permutations**: reordering records a node permutation map, so the original order is
always recoverable, and the values inside each assignment are untouched — only their positions
move.

### Sort by a key, e.g. `GEOID` (`sort="key"`)

A Census `GEOID` is a _hierarchical_ identifier — state, then county, then tract, then block —
so sorting nodes lexicographically by `GEOID` lays the map out in nested geographic order: the
blocks within a tract end up adjacent, the tracts within a county end up adjacent, and so on.
Because districts are assembled from geographically-contiguous pieces, units that are close in
that hierarchy usually fall in the same district, which produces long runs.

When you _have_ a meaningful geographic key this is often the single most effective ordering,
and it's the cheapest — it's just a sort. Use any node attribute via
`sort="key", key="GEOID20"`.

### Reverse Cuthill–McKee (`sort="rcm"`)

RCM comes from sparse linear algebra, where it reorders a matrix to pull all the non-zeros
close to the diagonal (it minimizes _bandwidth_). On a graph that amounts to: walk it
breadth-first from a peripheral node, number nodes as you reach them, then reverse the result.
The effect is that **graph-adjacent nodes get nearby indices**. Since the edges _inside_ a
district far outnumber the edges that cross a district boundary, neighbors usually share a
district — so nearby indices usually share a district, and the runs grow.

RCM uses only the graph's topology — no attributes required — so it's a solid default when you
don't have a geographic key to sort on.

### Multi-level clustering (`sort="mlc"`)

MLC reorders by **community structure**: it recursively groups the graph into tightly-connected
clusters and lays the nodes out cluster by cluster (each connected component handled on its
own). A district is, almost by definition, a tightly-connected cluster of units — so ordering
by clusters tends to line up with district boundaries even more closely than RCM does. That is
why it is the **default** for `add_graph`. Like RCM, it needs only the topology.

### In Python

```python
import networkx as nx

from binary_ensemble import graph

# A dual graph in NetworkX adjacency form, with a GEOID20 attribute to sort on.
dual_graph = nx.convert_node_labels_to_integers(nx.grid_2d_graph(4, 4))
for node in dual_graph.nodes:
    dual_graph.nodes[node]["GEOID20"] = f"{node:04d}"
adjacency = nx.adjacency_data(dual_graph)

reordered, permutation_map = graph.reorder(adjacency, sort="key", key="GEOID20")
# or graph.reorder(adjacency, sort="mlc") / sort="rcm"
```

Reordering returns a **node permutation map** so the change is fully reversible — you can
always recover the original node order. When you embed a graph in a bundle with
`BendlEncoder.add_graph(..., sort="mlc")`, the permutation map is stored for you.

### Which ordering should I use?

- **Have a geographic key** like `GEOID`? Start with `sort="key"` — usually the strongest, and
  the cheapest.
- **No useful key?** Use `sort="mlc"` (the default) or `sort="rcm"`; both work from the graph's
  topology alone.

These are all heuristics, so the exact win depends on your dual graph. Reordering is cheap and
reversible, so it rarely hurts — though on a _tiny_ ensemble the extra permutation map can
occasionally make the file net-larger. It pays off most right before an expensive XBEN
recompress, where every byte saved in BEN is amplified.

```{admonition} This is where the headline number comes from
:class: tip
The Colorado example: 100k plans on ~140k census blocks is **27 GB** of JSONL. Reordered by
`GEOID20`, the BEN stream is **~550 MB**; compressed to XBEN it's **~6 MB** — over **4500×**
smaller. Without the reorder, the same XBEN is far larger.
```

## Lever 2: district relabeling

LZMA2 (the compressor behind XBEN) spots repeated _byte sequences_ across plans. Two plans
can be structurally identical yet use different district numbers:

```
[2, 2, 3, 3, 1, 1]
[1, 1, 2, 2, 3, 3]   # same partition, different labels
```

To a human these are "the same map"; to LZMA2 they look different. **First-seen relabeling**
fixes this by renumbering district ids in order of first appearance, starting at 1, so
equivalent plans encode identically and compress better. Run it before encoding to XBEN.

## Putting it together

The recommended pipeline for a small, shareable archive is:

1. Build a `.bendl` file with a BEN stream while sampling (ideally on an already-reordered graph).
2. **Relabel and reorder** the bundle to maximize run length and cross-plan repetition.
3. **Recompress** the bundle's stream to XBEN.

`relabel_bundle` does step 2 in one call — it reorders the embedded graph, rewrites every
assignment into the new order, and stores the permutation map:

```python
from binary_ensemble import relabel_bundle, compress_stream

relabel_bundle("ensemble.bendl", out_file="ensemble-relabeled.bendl", sort="key", key="GEOID20")
compress_stream("ensemble-relabeled.bendl", out_file="ensemble-archive.bendl")
```

See [Shrink a bundle for sharing](../quick-help/shrink-for-sharing.md) for the full recipe.

## A note on resolution

BEN excels on **census-block** ensembles, where assignments are long (hundreds of thousands
of nodes) and runs are long. For coarser units like VTDs or tracts, assignments are short
(10–20k nodes) and the compression ratios are more modest — still useful, just less dramatic.
For very small files from MCMC on coarse units, the byte-level delta encoding of
[PCompress](https://github.com/mggg/pcompress) can be a good alternative.
