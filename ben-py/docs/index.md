---
sd_hide_title: true
---

# binary-ensemble

```{div} sd-text-center sd-fs-2 sd-font-weight-bold
binary-ensemble
```

```{div} sd-text-center sd-fs-5 sd-text-secondary
Compress, store, and stream massive ensembles of districting plans.
```

```{div} sd-text-center
[Get started](getting-started/quickstart.md){.download-badge}
[Concepts](concepts/overview.md){.download-badge}
[API reference](api/index.md){.download-badge}
```

---

Redistricting samplers like [GerryChain](https://gerrychain.readthedocs.io)'s ReCom,
ForestReCom, and Sequential Monte Carlo routinely emit **millions of plans**. Stored as
JSONL, a single ensemble can run to *tens of gigabytes* — most of it redundant, because
consecutive plans barely differ. **BEN** (Binary-Ensemble) is a compression format and
toolkit built for exactly this data: it turns those JSONL mountains into compact binary
files you can store, share, and stream sample-by-sample without unpacking the whole thing.

`binary-ensemble` is the Python interface to the
[binary-ensemble Rust crate](https://github.com/peterrrock2/binary-ensemble/tree/1.0.0/ben).

```{admonition} How much smaller?
:class: tip
A real 100k-plan ensemble on Colorado's ~140k census blocks is **27 GB** as JSONL.
Reordered by `GEOID20` it compresses to a **~550 MB** BEN stream, and then to a
**~6 MB** XBEN file — over a **4500× reduction**, fully lossless.
```

## Install

```bash
pip install binary-ensemble
```

## A first taste

Write an ensemble into one self-describing `.bendl` bundle, then read it back:

```python
from binary_ensemble import BendlEncoder, BendlDecoder

plans = [[1, 1, 2, 2], [1, 2, 2, 2], [1, 1, 1, 2]]

# The stream context finalizes the bundle when it closes.
encoder = BendlEncoder("ensemble.bendl", overwrite=True)
with encoder.stream("ben") as stream:
    for assignment in plans:
        stream.write(assignment)

# Iterate the assignments straight back out, one at a time.
for assignment in BendlDecoder("ensemble.bendl"):
    print(assignment)
```

## Where to next

::::{grid} 1 1 2 2
:gutter: 3

:::{grid-item-card} {octicon}`rocket` Getting started
:link: getting-started/quickstart
:link-type: doc

Install the package and compress your first ensemble in a few lines.
:::

:::{grid-item-card} {octicon}`book` Concepts
:link: concepts/overview
:link-type: doc

Dual graphs, assignments, the BEN/XBEN/BENDL formats, and the compression levers —
the mental model, data contract, performance model, and compatibility story behind the API.
:::

:::{grid-item-card} {octicon}`tools` How-to guides
:link: how-to/index
:link-type: doc

Task-focused recipes: compress a GerryChain run, subsample, convert formats,
shrink a bundle for sharing, diagnose errors, and copy cookbook patterns.
:::

:::{grid-item-card} {octicon}`code` API reference
:link: api/index
:link-type: doc

Every public class and function in `binary_ensemble`, organized by module.
:::

:::{grid-item-card} {octicon}`mortar-board` Tutorial notebooks
:link: user/using_bendl
:link-type: doc

Executable notebooks with rendered outputs. CI runs them end to end against the live API.
:::

::::

```{toctree}
:hidden:
:caption: Getting started

getting-started/installation
getting-started/quickstart
```

```{toctree}
:hidden:
:caption: Concepts

concepts/overview
concepts/vocabulary
concepts/data-model
concepts/jsonl-schema
concepts/formats
concepts/variants
concepts/compression
concepts/ordering-deep-dive
concepts/performance
concepts/api-map
concepts/cli-parity
concepts/limitations
concepts/compatibility
concepts/release-versioning
```

```{toctree}
:hidden:
:caption: How-to guides

how-to/index
how-to/end-to-end-workflow
how-to/api-cookbook
how-to/examples-gallery
how-to/anti-patterns
how-to/compress-gerrychain-run
how-to/read-and-iterate
how-to/subsample
how-to/convert-formats
how-to/shrink-for-sharing
how-to/custom-assets-and-append
how-to/troubleshooting
how-to/error-reference
```

```{toctree}
:hidden:
:caption: Tutorials

user/using_ben_py
user/using_bendl
```

```{toctree}
:hidden:
:caption: API reference

api/index
```

```{toctree}
:hidden:
:caption: Project

format stability <https://github.com/peterrrock2/binary-ensemble/blob/1.0.0/docs/format-stability.md>
Rust crate source <https://github.com/peterrrock2/binary-ensemble/tree/1.0.0/ben>
GitHub <https://github.com/peterrrock2/binary-ensemble>
```
