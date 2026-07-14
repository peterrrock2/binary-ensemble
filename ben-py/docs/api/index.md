# Overview

The public API is split into four modules that mirror the project's CLI tools. Everything
listed here is also re-exported from the top-level `binary_ensemble` namespace, so
`from binary_ensemble import BendlEncoder` and
`from binary_ensemble.bundle import BendlEncoder` are equivalent.

```{tip}
New here? Reach for **{mod}`binary_ensemble.bundle`** first. A `.bendl` file keeps the
assignment stream and its dual graph together in one self-describing file, which is what you
want the vast majority of the time. The other modules are for plain streams, whole-file
conversions, and graph preprocessing.
```

::::{grid} 1 1 2 2
:gutter: 3

:::{grid-item-card} {octicon}`package` bundle
:link: bundle
:link-type: doc

`BendlEncoder`, `BendlDecoder`, `compress_stream`, `decompress_stream`, `relabel_bundle` — the
recommended single-file `.bendl` format.
:::

:::{grid-item-card} {octicon}`list-unordered` stream
:link: stream
:link-type: doc

`BenEncoder`, `BenDecoder` — plain `.ben`/`.xben` streams when you don't need a bundle.
:::

:::{grid-item-card} {octicon}`arrow-switch` codec
:link: codec
:link-type: doc

Whole-file `encode_*` / `decode_*` transforms between JSONL, BEN, and XBEN.
:::

:::{grid-item-card} {octicon}`sort-desc` graph
:link: graph
:link-type: doc

Reorder a dual graph (MLC, RCM, or by key) before encoding to shrink the result.
:::

::::

```{toctree}
:hidden:

bundle
stream
codec
graph
```
