# Vocabulary

These are the core terms used throughout the docs and the API. They come from the project's
[glossary](https://github.com/peterrrock2/binary-ensemble/blob/main/docs/glossary.md), which
is the source of truth for the whole workspace.

## Dual graph

The geographic adjacency graph that gives meaning to an assignment. Nodes are geographic
**units** (census blocks, VTDs, tracts, precincts); edges connect units that are adjacent.
The dual graph fixes a **node ordering** — which unit is index 0, which is index 1, and so on.

In Python, dual graphs are read and written in **NetworkX adjacency format** (a JSON shape).
`BendlDecoder.read_graph()` hands one back to you as a live `networkx.Graph`.

## Plan

The mathematical object: a partition of the dual graph's nodes into districts. A plan is
*label-free up to relabeling* — renumbering the districts gives the same plan.

## Assignment

The concrete vector encoding of a plan: a list of integers of length *N* (the number of
nodes), where index *i* holds the **district id** of node *i*, in dual-graph node order.

```python
assignment = [1, 1, 2, 2, 3, 3]   # node 0 -> district 1, node 2 -> district 2, ...
```

An assignment uniquely determines a plan, but a single plan has many valid assignments (one
per node ordering and per district relabeling). This freedom is exactly what the
[compression levers](compression.md) exploit.

```{admonition} Node order is load-bearing
:class: warning
An assignment only means something *with respect to a particular dual graph's node order*.
If you write assignments in one order and read them against a graph in another, you get
silent nonsense. This is why bundles embed the graph — so the order travels with the data.
```

## District id

The integer values inside an assignment. The maximum supported district id is **65535**
(it must fit in 16 bits), which is far beyond any real statewide map.

## Sample

One entry in an ensemble: the pair `(sample_number, assignment)`. The `sample_number` is
**1-indexed** — decoded ensembles always start at sample 1.

## Ensemble

An ordered stream of samples produced by a single sampler run. Conceptually it's a
probabilistic draw from the space of plans. Every `.ben`, `.xben`, and `.bendl` file wraps
exactly one ensemble.

## Sample count

The number of draws an ensemble represents, always counted in *expanded* terms. When a
variant collapses five identical consecutive samples into one frame, the sample count still
goes up by five, not one. `len(decoder)` reports this expanded count.

## Variant

How a stream encodes its frames internally — one of `standard`, `mkv_chain`, or `twodelta`.
A variant is fixed for an entire stream when you encode, and decoding **auto-detects** it, so
you never specify a variant when reading. See [Encoding variants](variants.md).

## Sampler vs chain

- **Sampler** — any algorithm that produces an ensemble (covers both MCMC and SMC).
- **Chain** — specifically an MCMC method, where the Markov property matters.

Use *sampler* unless you specifically mean a Markov chain.
