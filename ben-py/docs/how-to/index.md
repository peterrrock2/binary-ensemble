# How-to guides

Task-focused recipes for common jobs. Each one is short and assumes you've met the basics in
the [Quickstart](../getting-started/quickstart.md).

## Sample data for these guides

The recipes below assume a small `ensemble.bendl`, a `plans.jsonl`, a `chain.ben` /
`chain.xben` pair, and a `gerrymandria.json` dual graph in your working directory. To follow
along, create them all with this snippet:

<!-- docs-test: setup -->
```python
import json

import networkx as nx

from binary_ensemble import BendlEncoder, BenEncoder, encode_ben_to_xben

# A small dual graph: an 8x8 grid with unit population, contiguous stripe districts,
# and a GEOID20-style key to sort on.
SIDE = 8
graph = nx.convert_node_labels_to_integers(nx.grid_2d_graph(SIDE, SIDE))
for node in graph.nodes:
    _row, col = divmod(node, SIDE)
    graph.nodes[node].update(TOTPOP=1, district=col // 2 + 1, GEOID20=f"{node:04d}")
adjacency = nx.adjacency_data(graph)
n_nodes = SIDE * SIDE

# The GerryChain how-to reads the dual graph from this file.
with open("gerrymandria.json", "w") as handle:
    json.dump(adjacency, handle)

# 120 toy plans on the grid's nodes.
plans = [[(node + step) % 4 + 1 for node in range(n_nodes)] for step in range(120)]

# A self-describing bundle (graph + metadata + the plans)...
encoder = BendlEncoder("ensemble.bendl", overwrite=True)
encoder.add_graph(adjacency, sort=None)
encoder.add_metadata({"sampler": "demo", "seed": 0})
with encoder.stream("ben") as stream:
    for plan in plans:
        stream.write(plan)

# ...the same plans as JSONL...
with open("plans.jsonl", "w") as handle:
    for sample, plan in enumerate(plans, start=1):
        handle.write(json.dumps({"assignment": plan, "sample": sample}) + "\n")

# ...and as plain BEN / XBEN streams.
with BenEncoder("chain.ben", overwrite=True) as stream:
    for plan in plans:
        stream.write(plan)
encode_ben_to_xben("chain.ben", "chain.xben", overwrite=True)
```

::::{grid} 1 1 2 2
:gutter: 3

:::{grid-item-card} End-to-end workflow
:link: end-to-end-workflow
:link-type: doc

Build a working `.bendl` bundle, inspect it, attach provenance, and archive it with XBEN.
:::

:::{grid-item-card} API cookbook
:link: api-cookbook
:link-type: doc

Copy focused snippets for the most common Python API tasks.
:::

:::{grid-item-card} Examples gallery
:link: examples-gallery
:link-type: doc

Small standalone patterns for minimal bundles, rich bundles, conversion, subsampling, and archival.
:::

:::{grid-item-card} Anti-patterns
:link: anti-patterns
:link-type: doc

Avoid node-order mistakes, repeated bundle extensions, wrong working formats, and fragile sharing.
:::

:::{grid-item-card} Compress a GerryChain run
:link: compress-gerrychain-run
:link-type: doc

Stream a ReCom chain straight into a self-describing `.bendl` bundle.
:::

:::{grid-item-card} Read and iterate an ensemble
:link: read-and-iterate
:link-type: doc

Open a bundle, recover its graph and metadata, and walk its assignments.
:::

:::{grid-item-card} Subsample a large ensemble
:link: subsample
:link-type: doc

Pull a subset of plans by index, range, or stride — without decoding the whole file.
:::

:::{grid-item-card} Convert between formats
:link: convert-formats
:link-type: doc

Whole-file transforms between JSONL, BEN, and XBEN.
:::

:::{grid-item-card} Shrink a bundle for sharing
:link: shrink-for-sharing
:link-type: doc

Reorder, relabel, and recompress a bundle to its smallest shareable form.
:::

:::{grid-item-card} Custom assets and appending
:link: custom-assets-and-append
:link-type: doc

Attach metadata and arbitrary blobs, then add more to a finalized bundle.
:::

:::{grid-item-card} Troubleshooting
:link: troubleshooting
:link-type: doc

Diagnose wrong readers, incomplete bundles, missing graphs, and node-order mismatches.
:::

:::{grid-item-card} Error reference
:link: error-reference
:link-type: doc

Map common exceptions and confusing symptoms to causes and fixes.
:::

::::
