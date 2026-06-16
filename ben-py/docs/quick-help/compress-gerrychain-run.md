# Compress a GerryChain run

The most common workflow: run a [GerryChain](https://gerrychain.readthedocs.io) ReCom chain
and stream every plan straight into a single self-describing `.bendl` file, so you never
materialize a giant JSONL file.

```{note}
This recipe needs GerryChain installed: `pip install gerrychain`. `binary-ensemble` itself
only ever sees plain lists of integers, so the same pattern works with any sampler.
```

## Reorder the graph before building the chain

The best compression wins come from graph order. `BendlEncoder.add_graph(..., sort="mlc")`
embeds an MLC-reordered graph and returns that reordered graph as a live NetworkX graph. Build
the GerryChain run on that returned graph so the sampler and the bundle agree on node order.

```python
from functools import partial

from gerrychain import Partition, Graph, MarkovChain, updaters, accept
from gerrychain.proposals import recom
from gerrychain.constraints import contiguous

from binary_ensemble import BendlEncoder

encoder = BendlEncoder("ensemble.bendl", overwrite=True)

# Explicitly show the default: MLC reorders the graph for better run-length compression.
mlc_graph = encoder.add_graph("gerrymandria.json", sort="mlc")

# Hand the reordered graph back into GerryChain. This is the load-bearing step:
# the chain now runs in the same node order the bundle stores.
graph = Graph.from_networkx(mlc_graph)
node_order = list(graph.nodes)

initial_partition = Partition(
    graph,
    assignment="district",
    updaters={"population": updaters.Tally("TOTPOP")},
)

ideal_population = sum(initial_partition["population"].values()) / len(initial_partition)

proposal = partial(
    recom, pop_col="TOTPOP", pop_target=ideal_population, epsilon=0.01, node_repeats=2
)

chain = MarkovChain(
    proposal=proposal,
    constraints=[contiguous],
    accept=accept.always_accept,
    initial_state=initial_partition,
    total_steps=1000,
)
```

## Stream the chain into a bundle

The one thing to get right is still **node order**. Since the chain was built on
`Graph.from_networkx(mlc_graph)`, each plan should be written in `node_order`, the node order
from that same GerryChain graph.

```python
encoder.add_metadata(
    {
        "sampler": "ReCom",
        "epsilon": 0.01,
        "steps": 1000,
        "node_order": "mlc",
    }
)

with encoder.ben_stream(variant="twodelta") as ensemble:  # twodelta suits ReCom chains
    for partition in chain:
        series = partition.assignment.to_series()
        assignment = series.loc[node_order].astype(int).tolist()
        ensemble.write(assignment)
# the bundle is finalized when the stream context closes
```

That's it — `ensemble.bendl` now holds all 1,000 plans plus the graph and metadata in one
file. To read it back, see [Read and iterate an ensemble](read-and-iterate.md).

## Why this is better than reordering later

You _can_ write a raw-order `.bendl` file with a BEN stream and later call
`relabel_bundle()` to reorder the graph and rewrite the stream. But when you control the
sampling code, it is cleaner to reorder first:

1. `add_graph(..., sort="mlc")` stores the reordered graph and permutation map.
2. `Graph.from_networkx(mlc_graph)` makes GerryChain run on that exact graph.
3. `series.loc[node_order]` writes assignments in that exact order.

That means the working BEN file is already locality-friendly, so every downstream step starts
from the compressed-friendly order.

## Archive the result

After the run, recompress the embedded BEN stream to XBEN for sharing:

```python
from binary_ensemble import compress_stream

compress_stream("ensemble.bendl", out_file="ensemble-archive.bendl")
```

For more on final archival workflows, see [Shrink a bundle for sharing](shrink-for-sharing.md).

```{tip}
Encoding `twodelta` (the default) delta-compresses pairwise ReCom moves. If you log a full
MCMC chain *including rejections*, `variant="mkv_chain"` collapses the repeated plans
instead. See [Encoding variants](../concepts/variants.md).
```
