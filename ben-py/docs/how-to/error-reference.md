# Error reference

This page maps common symptoms to likely causes and fixes, naming the exception class each
one raises so you can catch it deliberately. As a rule of thumb: file-system problems
(missing files, refusing to overwrite, closed encoders) raise `OSError`; invalid argument
values (bad `sort`, bad `content_type`, conflicting output modes, mismatched assignment
lengths) raise `ValueError`; and container/format mismatches detected inside the Rust
engine (wrong reader for the file, invalid subsample positions, truncated streams) raise a
plain `Exception` whose message describes the problem.

## Output file already exists

**Symptom:** a writer or converter raises an `OSError` saying the output path exists.

**Cause:** writers default to `overwrite=False`.

**Fix:** choose a new path or pass `overwrite=True`.

```python
from binary_ensemble import encode_jsonl_to_ben

encode_jsonl_to_ben("plans.jsonl", "error-reference.ben", overwrite=True)
```

## Wrong reader for the file type

**Symptom:** opening a file raises an `Exception` whose message names the decoder to use —
for example *"…is a .bendl bundle, not a plain BEN/XBEN stream. Open it with
binary_ensemble.bundle.BendlDecoder instead."*

**Cause:** `.bendl`, `.ben`, and `.xben` are different containers. (A missing or unreadable
file raises `OSError` instead.)

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

**Symptom:** `ValueError: bundle has no graph.json to reorder`.

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

**Symptom:** `ValueError: relabel_bundle only supports BEN bundles; relabel before
compressing to XBEN`.

**Cause:** `relabel_bundle()` works on `.bendl` bundles with embedded BEN streams. XBEN is the
final archive step.

**Fix:** relabel first, then recompress.

```python
from binary_ensemble import compress_stream, relabel_bundle

relabel_bundle("ensemble.bendl", out_file="error-sorted.bendl", sort="mlc")
compress_stream("error-sorted.bendl", out_file="error-archive.bendl")
```

## `content_type` is rejected

**Symptom:** `ValueError: content_type must be 'json' or 'text'`, or a `ValueError` about
invalid UTF-8 / invalid JSON.

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

**Symptom:** a `ValueError` such as `key=... is only valid with sort='key'` or
`unknown sort`.

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

## A subsample call rejects its arguments

**Symptom:** an `Exception` such as `indices must be 1-based`, `indices must be <= number
of samples in base data`, `range must be 1-based and end >= start`, or `step and offset
must be >= 1`.

**Cause:** sample positions are 1-based everywhere, and out-of-range positions raise
rather than being silently dropped. (An unsorted or duplicated index list does not raise —
it is sorted and deduplicated with a `UserWarning`.)

**Fix:** clamp the request to `len(decoder)` first.

```python
from binary_ensemble import BendlDecoder

decoder = BendlDecoder("ensemble.bendl")
window = list(decoder.subsample_range(1, min(50, len(decoder))))
assert len(window) == 50
```

## `stream.write()` rejects an assignment

**Symptom:** `ValueError: assignment length N does not match graph node count M`.

**Cause:** after `add_graph()`, every assignment written to the bundle's stream is checked
against the stored graph's node count. This catches node-order bugs at write time instead
of at analysis time.

**Fix:** write assignments in the returned graph's node order — one entry per node. See
[The data contract](../concepts/data-model.md).

## A bundle opens, but `len()` or iteration raises

**Symptom:** `BendlDecoder(path)` succeeds, but `len()` or the first `for` loop raises an
`Exception` such as `truncated TwoDelta frame` or `failed to fill whole buffer`.

**Cause:** the bundle was never finalized — typically a crashed run. The header is intact
but the stream's final frame was cut off mid-write.

**Fix:** confirm with `is_complete()`, then follow
[Recovering samples from a crashed run](troubleshooting.md#recovering-samples-from-a-crashed-run).

```python
from binary_ensemble import BendlDecoder

print(BendlDecoder("ensemble.bendl").is_complete())
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
