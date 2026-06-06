# Compress a GerryChain run

The most common workflow: run a [GerryChain](https://gerrychain.readthedocs.io) ReCom chain
and stream every plan straight into a single self-describing `.bendl` bundle, so you never
materialize a giant JSONL file.

```{note}
This recipe needs GerryChain installed: `pip install gerrychain`. `binary-ensemble` itself
only ever sees plain lists of integers, so the same pattern works with any sampler.
```

## Set up the chain

```python
from functools import partial

from gerrychain import Partition, Graph, MarkovChain, updaters, accept
from gerrychain.proposals import recom
from gerrychain.constraints import contiguous

graph = Graph.from_json("gerrymandria.json")

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

The one thing to get right is **node order**: an assignment vector is only meaningful in the
dual graph's node order, so reorder each plan to match the order you embed.

```python
from binary_ensemble import BendlEncoder

# The order assignments must be written in.
node_order = list(graph.nodes)

encoder = BendlEncoder("ensemble.bendl", overwrite=True)
encoder.add_graph("gerrymandria.json", sort=None)          # embed the dual graph as-is
encoder.add_metadata({"sampler": "ReCom", "epsilon": 0.01, "steps": 1000})

with encoder.stream("ben", variant="twodelta") as stream:  # twodelta suits ReCom chains
    for partition in chain:
        series = partition.assignment.to_series()
        assignment = series.loc[node_order].astype(int).tolist()
        stream.write(assignment)
# the bundle is finalized when the stream context closes
```

That's it — `ensemble.bendl` now holds all 1,000 plans plus the graph and metadata in one
file. To read it back, see [Read and iterate an ensemble](read-and-iterate.md).

## Make it smaller

The bundle above stores the graph in its original node order. For a much smaller file, reorder
the graph (so assignments form long runs) and recompress to XBEN — see
[Shrink a bundle for sharing](shrink-for-sharing.md). You can do this after the fact, so it
never complicates the sampling loop.

```{tip}
Encoding `twodelta` (the default) delta-compresses pairwise ReCom moves. If you log a full
MCMC chain *including rejections*, `variant="mkv_chain"` collapses the repeated plans
instead. See [Encoding variants](../concepts/variants.md).
```
