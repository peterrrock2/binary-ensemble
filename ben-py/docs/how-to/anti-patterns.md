# Anti-patterns

These are valid-looking patterns that produce bad workflows, confusing files, or silently
wrong analysis.

## Writing assignments in the wrong graph order

An assignment vector has no geographic meaning by itself. It only means something with
respect to the graph order.

```python
from binary_ensemble import BendlDecoder

decoder = BendlDecoder("ensemble.bendl")
graph = decoder.read_graph()
assignment = next(iter(decoder))

assert graph is not None
assert len(assignment) == graph.number_of_nodes()
```

That length check is necessary but not sufficient. The sampler still must write assignments
in `list(graph.nodes)` order.

## Reordering the graph after writing assignments

Do not sort or relabel a graph file by hand after encoding a stream. If the graph order
changes, every assignment position must be rewritten too. Use `relabel_bundle()` for that.

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

