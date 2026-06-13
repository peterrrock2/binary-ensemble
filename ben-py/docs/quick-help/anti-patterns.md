# Anti-patterns

Each of these looks reasonable and runs without complaint, yet quietly leads somewhere bad: a
confusing file, a workflow that won't reproduce, or analysis that is wrong without ever raising
an error. Here is what goes wrong, and what to reach for instead.

## Writing assignments in the wrong graph order

An assignment vector carries no geographic meaning on its own; it means something only against a
specific graph node order. Line the two up wrong and every district id lands on the wrong unit,
silently.

```python
from binary_ensemble import BendlDecoder

decoder = BendlDecoder("ensemble.bendl")
graph = decoder.read_graph()
assignment = next(iter(decoder))

assert graph is not None
assert len(assignment) == graph.number_of_nodes()
```

The length check above is necessary but not sufficient: the sampler must also emit each
assignment in `list(graph.nodes)` order, or the plan and the graph disagree with no symptom to
warn you.

## Reordering the graph after writing assignments

Don't sort or relabel a graph file by hand once a stream has been encoded against it. Moving the
nodes means every assignment position has to move with them, and editing the graph alone leaves
the two out of sync. `relabel_bundle()` does both halves together:

```python
from binary_ensemble import relabel_bundle

relabel_bundle("ensemble.bendl", out_file="ensemble-sorted.bendl", sort="mlc")
```

## Using XBEN as the working format

XBEN is for archive and transfer. It is small, but compression is expensive and repeated
inspection pays decompression startup costs. Work in BEN or a BEN-backed `.bendl` file,
then recompress once.

## Shipping a plain stream without its graph

Plain `.ben` and `.xben` files do not carry graph or metadata. That is fine for internal
pipelines where the graph is guaranteed, but it is fragile for collaboration. Prefer `.bendl`
for anything shared.

## Repeated bundle extensions

Do not name bundles `run.xben.bendl`, `run.sorted.bendl`, or `run.archive.bendl`. A bundle is
a bundle regardless of the embedded stream. Use one `.bendl` extension and put state in the
basename:

| Avoid | Prefer |
|---|---|
| `run.xben.bendl` | `run-archive.bendl` |
| `run.sorted.bendl` | `run-sorted.bendl` |
| `run.relabeled.bendl` | `run-relabeled.bendl` |

Plain streams should still use `.ben` and `.xben`.

## Appending samples to a finalized bundle

Append mode is for assets only. A `.bendl` file has one assignment stream. To add more
samples, write a new bundle.

```python
from binary_ensemble import BendlEncoder

encoder = BendlEncoder.append("ensemble.bendl")
encoder.add_asset("review-notes.txt", "Asset append only.", content_type="text")
encoder.close()
```

