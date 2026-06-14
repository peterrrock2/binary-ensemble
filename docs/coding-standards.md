# Binary Ensemble — Coding Standards

This document describes the coding standards of the `binary-ensemble` workspace as they are actually
practiced in the code today. It is descriptive first (what the code does) and prescriptive second
(what new code should do to fit in). When in doubt, imitate the surrounding module.

The workspace is a Rust library + four CLI tools for compressing ensembles of districting plans (the
BEN / XBEN / BENDL formats), plus a PyO3 binding crate that ships the `binary_ensemble` Python
package.

A companion document, [`docs/glossary.md`](glossary.md), is the source of truth for **terminology**.
This document covers **mechanics**. The two are meant to be read together: the glossary tells you
what to call a thing, this tells you how to write the code around it.

---

## 1. Workspace layout

The repository is a Cargo workspace (`resolver = "2"`) with two members:

- **`ben/`** — package `binary-ensemble`, library name `binary_ensemble`. Contains the codec, I/O,
  ops, JSON-graph, format, and CLI logic, plus two thin binaries (`ben`, `bendl`) under
  `ben/src/bin/`. `ben` is a subcommand tree (`encode`, `decode`, `relabel`, `canonicalize`,
  `reencode`, `sort-graph`, `pcompress`, ...); `bendl` drives the `.bendl` container.
- **`ben-py/`** — package `ben-py`, cdylib `ben_py_core`. PyO3 bindings that depend on
  `binary-ensemble` by path and are published as the `binary_ensemble` Python package.

Conventions:

- **The version is shared.** Both crates use `version.workspace = true` from `[workspace.package]`;
  never set a per-crate version. Don't hard-code the version string anywhere in source or comments
  either — keep code self-contained and free of version pins.
- **Binaries stay thin, with a uniform `main`.** Each `ben/src/bin/*.rs` just calls
  `cli::<tool>::run()`, and on `Err` prints `Error: {err}` to stderr and exits non-zero. All real
  behavior lives in the library so it is testable without spawning a process. Each CLI module
  exposes `pub fn run() -> CliResult`, parses a `clap` `#[derive(Parser)]` `Args`, and dispatches to
  per-mode handlers; CLI failures use the crate's own `CliError`/`CliResult`, not bare `io::Error`.
- **The Python crate owns all PyO3.** Nothing in `ben/` depends on `pyo3`. Python concerns live
  entirely in `ben-py/`.

---

## 2. Toolchain, formatting, and the task runner

- **Edition 2021**, MIT licensed. No pinned `rust-toolchain.toml`; build on ambient stable.
- **`rustfmt` with an explicit config** (`rustfmt.toml`): `max_width = 100`, `comment_width = 100`,
  `wrap_comments = true`. Comments are auto-wrapped to 100 columns — write naturally and let
  `rustfmt` reflow. Always run `cargo fmt --all` (or `task format-rust`) before committing.
- **Quality gates are run locally via `Taskfile.yml`** (the `task` / `go-task` runner), not in CI.
  The GitHub workflow (`ci_cd.yml`) only builds and publishes wheels with `maturin`. Before pushing,
  run the same checks the maintainer does:
  - `task test` — Rust fast suite + `#[ignore]`-gated slow suite + Python `pytest`.
  - `task format` — `cargo fmt --all` + `ruff format` for Python.
  - `task lint` — `ruff check` for Python.
  - `task coverage-*` — `cargo llvm-cov` (the `bin/` wrappers are excluded from coverage; they're
    meant to be trivial).
- **Python tooling is `uv` + `maturin` + `ruff` + `pytest`.** Develop the extension with
  `task ben-py-develop` (runs `maturin develop` inside the `uv` env). Format/lint Python with
  `ruff`.

---

## 3. Module organization

- **Directory modules use `mod.rs`.** A module folder is fronted by `mod.rs` (e.g. `codec/mod.rs`,
  `format/mod.rs`, `io/mod.rs`), which declares the submodules and re-exports the module's common
  surface.
- **Re-export the public surface with `pub use`.** Parent modules flatten the names callers need
  (e.g. `codec/mod.rs` does `pub use frames::{BenDecodeFrame, BenEncodeFrame};`; `format/mod.rs`
  does `pub use errors::FormatError;`). Add new public items to the appropriate re-export rather
  than forcing callers down deep paths.
- **Every module opens with a `//!` doc comment** that says what the module is for and links
  siblings with intra-doc links (e.g. ``[`encode`]``, ``[`decode`]``, ``[`translate`]``). The
  `pub mod` declarations in `lib.rs` each carry a `///` one-liner.
- **Errors live in `errors.rs`.** A module that defines its own error type puts it in a sibling
  `errors.rs` (e.g. `format/errors.rs`, `codec/translate/errors.rs`, `json/graph/errors.rs`) and
  re-exports it from `mod.rs`.
- **Tests live next to what they test** under `#[cfg(test)] mod tests`. Small modules use a sibling
  `tests.rs`; larger ones use a `tests/` subdirectory split by topic (e.g.
  `codec/decode/tests/{standard,mkvchain,twodelta}.rs`,
  `io/bundle/tests/{reader,writer,format}.rs`). Cross-cutting, process-level, and stability tests go
  in `ben/tests/` integration files.
- **Shared test helpers go in `test_utils`**, declared `#[doc(hidden)] pub mod test_utils;` so
  they're reusable across the crate's test trees without polluting the public docs.
- **Guard platform assumptions explicitly.** `lib.rs` rejects non-64-bit targets with
  `compile_error!`. Encode invariants you rely on rather than letting them fail silently.

---

## 4. Types and domain modeling

- **Model domain concepts as enums/structs with doc-commented variants.** E.g.
  `BenVariant { Standard, MkvChain, TwoDelta }`, each variant documented with what it stores and
  when it applies.
- **Push invariants into the type system when you can.** `XBenVariant` is a deliberately restricted
  subset (`Standard`, `MkvChain`) that _cannot_ represent `TwoDelta`, so functions parameterised by
  `XBenVariant` are uncallable for TwoDelta at compile time. Prefer this kind of
  make-illegal-states-unrepresentable design over runtime `assert!`s.
- **Provide `From`/`TryFrom` between related types**, and when a conversion can fail, return a
  dedicated, named error type (e.g. `TwoDeltaNotXBenError`) rather than a bare `()` or a string —
  even a tiny marker struct gets `Display` + `std::error::Error` impls.
- **Derive the obvious traits.** Small value types carry
  `#[derive(Debug, Clone, Copy, PartialEq, Eq)]` (and `Serialize` / `Deserialize` where they cross
  the JSON boundary). Give public types a `Debug` impl by default.
- **Mark extensible public enums/structs `#[non_exhaustive]`.** Options and transform types that may
  grow new variants/fields (e.g. `RelabelTransform`, `RunPolicy`, `RelabelOptions`) are
  `#[non_exhaustive]` so adding to them isn't a breaking change.
- **Build complex options with a constructor-plus-`with_*` builder.** `RelabelOptions` is created by
  an intent-named constructor (`first_seen`, `node_permutation`, `convert_to`) and then refined with
  `with_*` methods, rather than exposing a wide public constructor or public fields.
- **Reserve unused bits/fields explicitly** for forward compatibility (e.g. named `RESERVED_BIT_*`
  constants) rather than leaving holes undocumented.

---

## 5. Error handling

- **Library errors are `thiserror` enums, one per module, in `errors.rs`.** Each variant has a
  descriptive `#[error("...")]` message that includes the relevant values (e.g. `UnknownBanner`
  prints the actual bytes seen _and_ the expected set). Wrap source errors with `#[from]` (e.g.
  `Io(#[from] io::Error)`).
- **Bridge domain errors to `io::Error` at streaming boundaries.** The pattern is an explicit
  `impl From<DomainError> for io::Error` that forwards a real IO error unchanged and maps everything
  else to `io::ErrorKind::InvalidData`. This lets streaming readers/writers keep `io::Result`
  signatures while still carrying precise error context.
- **`expect()` only for genuinely infallible cases, with a message that states the invariant** (e.g.
  `.expect("valid fallback log filter")`). Avoid `unwrap()`/`expect()` on real IO, parsing, or
  caller-supplied data in library paths — return a `Result` and propagate with `?`.
- **`?` is the default control flow** for fallible calls. Reserve `panic!`/`unreachable!` for true
  logic invariants, not expected failures.

---

## 6. Logging

- **Use `tracing`, not `log` and not `println!`.** Emit diagnostics with the `tracing` macros. In
  practice the codebase logs almost entirely at `trace!` (fine-grained internal flow) with the
  occasional `warn!`; reach for higher levels only when a message genuinely belongs there.
- **Subscriber init is centralized and idempotent.** `logging::init_logging()` sets the global
  subscriber exactly once via `std::sync::Once`, reads `RUST_LOG` (defaulting to `off`), writes to
  **stderr**, and uses a compact format with time/target/level/ANSI disabled. Don't stand up ad-hoc
  subscribers elsewhere.
- **`stdout` is for program output only** (decoded data, version banners, inspect listings) — never
  for logging. **`stderr`** carries logs and progress.
- **Long streaming operations report progress with `indicatif`.**

---

## 7. I/O and performance

- **Stream; don't slurp.** The crate processes ensembles too large to hold in memory. Functions take
  buffered readers/writers and work frame-by-frame / line-by-line.
- **Be generic over IO** with `R: Read`/`R: BufRead` and `W: Write` bounds so the same code serves
  files, pipes, in-memory buffers, and test fixtures.
- **Binary fields use `byteorder` with an explicit endianness** — never rely on native byte order
  for on-disk data.
- **Integrity is checked with CRC32C (`crc32c` crate).** That crate was chosen deliberately over
  `crc32fast` (it can't be misconfigured into IEEE CRC-32); the rationale is recorded in
  `ben/Cargo.toml`. Keep integrity checks on payloads and the assignment stream.
- **Preserve the lazy-decode property of BEN frames** — a frame keeps its raw bytes without eagerly
  unpacking runs, which is what makes subsample-by-skip and random-access reads fast. Don't
  introduce a unified frame representation that forces eager bit-unpacking on read.

---

## 8. Documentation

Documentation is treated as part of the code, not an afterthought.

- **`//!` on every module, `///` on public items.** Item docs use the conventional rustdoc sections
  already prevalent here: `# Arguments`, `# Returns`, `# Examples`, `# Errors`, `# Panics`. Format
  illustrations in docs use fenced ` ```text ` blocks.
- **Comments explain intent and stay self-contained and timeless.** Don't reference planning-doc
  filenames, plan section numbers, or version numbers from source/inline comments. Pointing a reader
  at the stable `docs/glossary.md` for terminology is fine and done in the code; pointing at a
  transient plan is not.
- **Substantive design goes in `docs/`.** Architecture/context in `CONTEXT.md`; vocabulary in
  `docs/glossary.md`; the on-disk contract in the format spec; plans in `docs/<topic>-plan.md`
  _before_ implementation.

---

## 9. Naming

- **Standard Rust casing:** `snake_case` items, `UpperCamelCase` types, `SCREAMING_SNAKE_CASE`
  consts.
- **Names follow the glossary's lexicon exactly.** This is an explicit, enforced standard: use
  `plan` / `assignment` / `sample` / `ensemble` / `variant` / `banner` / `frame` / `stream` with
  their glossary meanings, and the verbs `encode` / `decode` (never `compress` / `decompress`).
  Spell format names out in identifiers (`ben`, `ben32`, `xben`, `bendl`, `jsonl`). If prose and an
  identifier disagree, the glossary wins and the identifier is what changes.
- **Functions are named descriptively after their transform**, encoding direction and operands. Two
  suffix/affix conventions are consistent and worth following:
  - Direction is spelled out with `_to_` / `_from_` (`decode_xben_to_jsonl`, `encode_jsonl_to_ben`,
    `ben_to_ben32_lines`).
  - A `_path` suffix marks the convenience wrapper that takes a file path over the streaming core
    (`decode_ben_to_jsonl` vs `decode_ben_to_jsonl_path`).
  - An `_unverified` suffix marks the variant that **skips integrity checks** (`asset_bytes` vs
    `asset_bytes_unverified`). The default name verifies; the escape hatch is explicitly labeled.
    Prefer clarity over brevity.
- **Name magic values once as consts** (banners, magic bytes, header sizes, asset-type/flag values);
  never inline a protocol literal at a use site.

---

## 10. Testing

- **Property-based testing with `proptest`** for invariants — above all the round-trip property
  (encode → decode reproduces the original assignments). `.proptest-regressions` files are committed
  so found counterexamples stay covered.
- **Determinism in randomized tests:** seed with `rand_chacha` / explicit seeds; use `lipsum` for
  synthetic text. Tests must be reproducible.
- **Slow / stress tests are gated with `#[ignore]`** and run separately (`cargo test -- --ignored`,
  i.e. `task test-rust-slow`). Keep the default `cargo test` fast.
- **Filesystem tests must be hermetic** — use temp files, never repo-relative scratch paths.
- **Test the behavior, at the right layer.** Unit tests live beside their module; format-stability,
  CLI, and full-pipeline behavior live in `ben/tests/`. The CLI is exercised end-to-end
  (`test_cli.rs`), and format stability is pinned by golden tests — treat an on-disk format change
  that breaks them as a deliberate, documented decision.

---

## 11. Python bindings (`ben-py`)

- **All PyO3 code is isolated in the `ben-py` crate** and built against the **stable ABI**
  (`abi3-py311`, `extension-module`). The core library has no Python dependency.
- **Match the prevailing PyO3 version's idioms** (the bound API: `Bound<'_, _>`,
  `wrap_pyfunction!`). Don't mix older and newer PyO3 styles within the crate.
- **Names align across the language boundary.** Rust structs carry a `Py` prefix internally (e.g.
  `PyBenEncoder`) but are exposed to Python with the prefix stripped via
  `#[pyclass(name = "BenEncoder")]`; Python methods use the same `encode_*` / `decode_*` verbs as
  Rust.
- **Spell out Python-visible signatures** with `#[pyo3(signature = ...)]` for defaults and a
  matching `#[pyo3(text_signature = "...")]` so the signature shows up in Python help/IDE tooling.
- **Map Rust errors to specific Python exceptions** at the boundary via small private `map_*_err`
  helpers that match on the source error and pick the right exception (`PyIOError` for IO,
  `PyValueError` for bad input, `PyKeyError` for missing keys, `PyException` as fallback). A panic
  must never cross the FFI line.
- **Ship typing metadata:** the package includes `py.typed` and a `_core.pyi` stub; keep the stub in
  sync with the exported surface. Python users import re-exported names from the `binary_ensemble`
  package, not from `_core` directly.
- **Type the surface precisely, with shared aliases.** Public payload shapes live in
  `binary_ensemble.types` (`GraphInput`, `StrPath`, `Variant`, `SortMethod`, the asset-payload
  unions, and the `NodePermutationMap` / `AssetEntry` TypedDicts) and are used by the facades and
  every `.pyi` stub — no `Any` where the accepted shapes are known. Use modern hints (`X | None`,
  builtin generics, `collections.abc`); the floor is Python 3.11. Type checking is two-stage —
  `ty` then `pyright` (`task typecheck-python`, part of `task lint`) — and
  `tests/typing_assertions.py` pins the surface from the consumer side: `assert_type` for
  positives, bare `# type: ignore` for calls that must NOT type-check (kept honest by pyright's
  `reportUnnecessaryTypeIgnoreComment`).
- **Python-visible docstrings document every argument** (facade `.py` files and the Rust `///`
  docs alike, Google style): each `Args:` entry carries its type in parentheses — the shared alias
  name where one exists, e.g. `graph (GraphInput):` — with custom-type shapes spelled out in the
  description. Defaulted parameters are marked `(<type>, optional)` and state the default as
  "Default is `X`." — or, when `None` is meaningful, "Default is `None` which ⟨meaning⟩."

---

## 12. Dependencies

- **Conservative, single-purpose crates, each justified.** Current set includes `byteorder`
  (explicit-endian binary IO), `crc32c` (integrity), `xz2` (LZMA2 for XBEN), `serde`/`serde_json`,
  `clap` (derive CLI), `indicatif` (progress), `petgraph` + `rustworkx-core` (graph ordering for
  relabeling), `pipe` (in-memory streaming), `pcompress` (foreign-format bridge), `thiserror`, and
  `tracing`/`tracing-subscriber`. Dev-only: `proptest`, `lipsum`, `rand` + `rand_chacha` +
  `rand_distr`.
- **Record non-obvious choices in `Cargo.toml`.** The `crc32c`-vs- `crc32fast` decision is
  documented inline; do the same for any future "why this crate" decision.
- **Prefer reusing a present dependency** over adding a new one.

---

## Quick checklist for a new change

- [ ] `cargo fmt --all` clean; `task test` green (fast + `--ignored` + Python);
      `ruff check`/`ruff format` clean for any Python.
- [ ] New public items have `//!`/`///` docs with the standard sections; comments are self-contained
      (no plan/section/version references).
- [ ] Errors are `thiserror` enums in `errors.rs` with informative messages; boundaries bridge to
      `io::Error`; no stray `unwrap()`/`expect()` on real IO or input.
- [ ] Diagnostics via `tracing` (stderr); program output only on stdout; progress via `indicatif`.
- [ ] Streaming over buffered, generic IO; explicit endianness; BEN frame lazy-decode preserved;
      integrity (CRC32C) intact.
- [ ] Identifiers match `docs/glossary.md`; magic values named as consts.
- [ ] Round-trip / invariant covered by `proptest`; randomness seeded; slow tests `#[ignore]`d; temp
      files for FS tests.
- [ ] PyO3 changes stay in `ben-py`, keep `abi3`, map errors to typed Python exceptions, and update
      the `_core.pyi` stub.
