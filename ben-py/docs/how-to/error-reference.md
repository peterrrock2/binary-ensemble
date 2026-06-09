# Error reference

This page maps common symptoms to likely causes and fixes.

## Output file already exists

**Symptom:** a writer or converter raises an `OSError` saying the output path exists.

**Cause:** writers default to `overwrite=False`.

**Fix:** choose a new path or pass `overwrite=True`.

```python
from binary_ensemble import encode_jsonl_to_ben

encode_jsonl_to_ben("plans.jsonl", "error-reference.ben", overwrite=True)
```

## Wrong reader for the file type

**Symptom:** opening a file raises an error that points you at another decoder.

**Cause:** `.bendl`, `.ben`, and `.xben` are different containers.

**Fix:** use the matching reader.

```python
from binary_ensemble import BendlDecoder, BenDecoder

bundle = BendlDecoder("ensemble.bendl")
ben_stream = BenDecoder("chain.ben")
xben_stream = BenDecoder("chain.xben", mode="xben")

assert bundle.assignment_format() in {"ben", "xben"}
assert ben_stream.assignment_format() == "ben"
assert xben_stream.assignment_format() == "xben"
```

## `read_graph()` returns `None`

**Cause:** the bundle has no `graph.json` asset.

**Fix:** inspect assets, then attach the graph in future bundles.

```python
from binary_ensemble import BendlDecoder

decoder = BendlDecoder("ensemble.bendl")
print(decoder.asset_names())
```

## Relabeling fails because the bundle has no graph

**Cause:** `relabel_bundle()` must know the graph order to rewrite assignment positions.

**Fix:** create bundles with `add_graph()`, or relabel before discarding the graph context.

```python
import networkx as nx

from binary_ensemble import BendlEncoder

graph = nx.convert_node_labels_to_integers(nx.path_graph(4))

encoder = BendlEncoder("error-with-graph.bendl", overwrite=True)
encoder.add_graph(nx.adjacency_data(graph), sort=None)
with encoder.stream("ben") as stream:
    stream.write([1, 1, 2, 2])
```

## Relabeling fails after XBEN recompression

**Cause:** `relabel_bundle()` works on `.bendl` bundles with embedded BEN streams. XBEN is the
final archive step.

**Fix:** relabel first, then recompress.

```python
from binary_ensemble import compress_stream, relabel_bundle

relabel_bundle("ensemble.bendl", out_file="error-sorted.bendl", sort="mlc")
compress_stream("error-sorted.bendl", out_file="error-archive.bendl")
```

## `content_type` is rejected

**Cause:** `add_asset()` accepts only `content_type="json"` or `content_type="text"` from the
Python wrapper. JSON payloads must be valid UTF-8 JSON; text payloads must be valid UTF-8.

**Fix:** choose the right content type and validate payloads before writing.

```python
from binary_ensemble import BendlEncoder

encoder = BendlEncoder("error-assets.bendl", overwrite=True)
encoder.add_asset("valid.json", '{"ok": true}', content_type="json")
encoder.add_asset("valid.txt", "plain text", content_type="text")

with encoder.stream("ben") as stream:
    stream.write([1, 1, 2, 2])
```

## `sort="key"` fails

**Cause:** key ordering requires a `key=` argument, and every node must have the relevant
attribute unless you use `key="id"`.

**Fix:** provide the key and check the graph attributes.

```python
import networkx as nx

from binary_ensemble import graph

dual_graph = nx.convert_node_labels_to_integers(nx.path_graph(4))
for node in dual_graph.nodes:
    dual_graph.nodes[node]["GEOID20"] = f"{node:04d}"

ordered_graph, _ = graph.reorder(
    nx.adjacency_data(dual_graph),
    sort="key",
    key="GEOID20",
)

assert ordered_graph.number_of_nodes() == 4
```

## Assignments decode but downstream maps are wrong

**Cause:** graph order and assignment order do not match.

**Fix:** compare assignment length to graph size, then audit how assignments were extracted
from the sampler.

```python
from binary_ensemble import BendlDecoder

decoder = BendlDecoder("ensemble.bendl")
graph = decoder.read_graph()
assignment = next(iter(decoder))

assert graph is not None
assert len(assignment) == graph.number_of_nodes()
```

Length agreement is necessary, not sufficient. The only complete fix is writing assignments
in the embedded graph's node order.

## XBEN startup warning

**Cause:** XBEN must initialize decompression before yielding assignments.

**Fix:** this is expected. Convert to BEN if you will repeatedly inspect or subsample the
same stream.

```python
from binary_ensemble import decode_xben_to_ben

decode_xben_to_ben("chain.xben", "error-work.ben", overwrite=True)
```
