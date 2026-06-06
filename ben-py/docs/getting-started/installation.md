# Installation

`binary-ensemble` ships as a pre-built wheel for Linux, macOS, and Windows, so the usual
one-liner is all you need:

```bash
pip install binary-ensemble
```

The package requires **Python 3.11 or newer**. Its only runtime dependency is
[NetworkX](https://networkx.org) (used to hand dual graphs back to you as graph objects).

## Optional: GerryChain

The how-to guides and tutorials that build ensembles with
[GerryChain](https://gerrychain.readthedocs.io) need it installed alongside
`binary-ensemble`:

```bash
pip install binary-ensemble gerrychain
```

`binary-ensemble` itself does **not** depend on GerryChain — it accepts plain Python lists
of integers, so it works with any sampler (ForestReCom, SMC, your own code) or with
pre-existing JSONL files.

## Verify the install

```python
import binary_ensemble

print(binary_ensemble.__all__)
```

You should see the public surface: the `BendlEncoder`/`BendlDecoder` bundle classes, the
`BenEncoder`/`BenDecoder` stream classes, the `encode_*`/`decode_*` codec helpers, and the
`bundle`, `stream`, `codec`, and `graph` submodules.

## Building from source

`binary-ensemble` is a [PyO3](https://pyo3.rs) extension built with
[maturin](https://www.maturin.rs). To build it from a checkout you need a Rust toolchain:

```bash
git clone https://github.com/peterrrock2/binary-ensemble
cd binary-ensemble/ben-py
pip install maturin
maturin develop --release      # builds the extension and installs it editable
```

## Command-line tools

This Python package wraps the same engine as the project's CLI tools (`ben`, `reben`,
`bendl`, `pcben`), which are distributed through Cargo:

```bash
cargo install binary-ensemble
```

The Python API mirrors the CLI's structure — see [The API map](../concepts/api-map.md).
