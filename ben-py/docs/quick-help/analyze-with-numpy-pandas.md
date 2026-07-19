# Analyze an ensemble with NumPy and pandas

The decoders yield each plan as a plain `list[int]`, which makes them sampler-agnostic —
but for real analysis you usually want the whole ensemble (or a slice of it) as one
array, so scores can be computed vectorized instead of plan by plan. This guide shows the
patterns; it assumes the sample files from [How-to guides](index.md).

## Load an ensemble into a NumPy array

Stack the decoded plans into an `(n_samples, n_nodes)` array. District ids fit in 16 bits
(see [Limitations](../concepts/limitations.md)), so `dtype=np.uint16` keeps the array
compact:

```python
import numpy as np

from binary_ensemble import BendlDecoder

decoder = BendlDecoder("ensemble.bendl")
ensemble = np.array(list(decoder), dtype=np.uint16)

print(ensemble.shape)        # (120, 64): one row per sample, one column per node
```

```{admonition} Will it fit?
:class: warning
The array costs `n_samples × n_nodes × 2` bytes at `uint16`. A 10,000-plan ensemble on a
10,000-node graph is a comfortable 200 MB — but 50k plans on 140k census blocks is 14 GB.
For ensembles that big, [subsample](subsample.md) before stacking, or use the streaming
pattern at the end of this page.
```

To stack only a slice, chain a subsample call into the same expression:

```python
thinned = np.array(list(BendlDecoder("ensemble.bendl").subsample_every(10)), dtype=np.uint16)
print(thinned.shape)         # (12, 64)
```

## Score plans vectorized

With the ensemble as an array, per-plan statistics become one-liners over `axis=1`:

```python
district_one_share = (ensemble == 1).mean(axis=1)   # fraction of nodes in district 1
districts_used = ensemble.max(axis=1)               # highest district id per plan

print(district_one_share[:4])
print(districts_used[:4])
```

Per-district node counts for every plan come from {func}`numpy.bincount` row by row:

```python
n_districts = int(ensemble.max())
sizes = np.stack([np.bincount(plan, minlength=n_districts + 1)[1:] for plan in ensemble])

print(sizes.shape)           # (120, 4): district sizes, one row per plan
```

If node populations live on the graph, weight the counts to get district populations:

```python
decoder = BendlDecoder("ensemble.bendl")
graph = decoder.read_graph()
populations = np.array([graph.nodes[node]["TOTPOP"] for node in graph.nodes])

district_pops = np.stack(
    [np.bincount(plan, weights=populations, minlength=n_districts + 1)[1:] for plan in ensemble]
)
print(district_pops[0])      # population of districts 1..4 under the first plan
```

The order of `populations` matches the order of the assignment columns because both
follow the embedded graph's node order — that positional contract is the whole point of
bundling the graph (see [The data contract](../concepts/data-model.md)).

## Label columns with a DataFrame

A {class}`pandas.DataFrame` makes the node order explicit by naming each column after its
graph node. Decoded JSONL uses one-based `sample` labels, so use the same labels here if you want
the table to match that representation:

```python
import pandas as pd

decoder = BendlDecoder("ensemble.bendl")
graph = decoder.read_graph()
node_labels = [graph.nodes[node]["GEOID20"] for node in graph.nodes]

frame = pd.DataFrame(ensemble, columns=node_labels)
frame.index = pd.RangeIndex(1, len(frame) + 1, name="sample")

print(frame.iloc[:3, :4])
```

(Use `list(graph.nodes)` as the labels when the graph has no geographic key.)

From here, everything in the pandas toolbox applies — `frame.eq(1).mean(axis=1)`,
groupbys over melted long form, joins against node-level covariates keyed by `GEOID20`,
and so on.

## Stream when the ensemble doesn't fit in memory

For ensembles too large to stack, iterate once and accumulate. The decoder never holds
more than one plan in memory, so this scales to millions of samples:

```python
import numpy as np

from binary_ensemble import BendlDecoder

decoder = BendlDecoder("ensemble.bendl")

totals = np.zeros(decoder.read_graph().number_of_nodes())
for assignment in decoder:
    plan = np.asarray(assignment, dtype=np.uint16)
    totals += plan == 1

frequency = totals / len(decoder)
print(frequency[:8])         # how often each node lands in district 1
```

`len(decoder)` is cheap (on a finalized bundle it is read from the header), so it is safe
to use for the denominator or a progress bar.

## Any sampler, any source

Nothing on this page is GerryChain-specific. The decoders return plain integer lists no
matter what produced the stream — a ReCom chain, ForestReCom, Sequential Monte Carlo, or
a JSONL file converted with the [codec helpers](convert-formats.md) — so the same NumPy
and pandas patterns apply to all of them.
