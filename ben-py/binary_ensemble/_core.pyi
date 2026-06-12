"""Type stubs for the compiled ``binary_ensemble._core`` extension.

These describe the raw PyO3 surface. End users should import the ergonomic facades from
:mod:`binary_ensemble.stream`, :mod:`binary_ensemble.bundle`, :mod:`binary_ensemble.codec`, and
:mod:`binary_ensemble.graph` instead.
"""

from collections.abc import Iterator, Sequence
from types import TracebackType
from typing import Any, Literal

import networkx as nx

from binary_ensemble.types import (
    AssetEntry,
    AssignmentFormat,
    GraphInput,
    MetadataInput,
    NodePermutationMap,
    SortMethod,
    StrPath,
    Variant,
)

# ---------------------------------------------------------------------------
# Stream decoder / encoder (plain .ben / .xben)
# ---------------------------------------------------------------------------

class BenDecoder:
    """Iterator over assignments in a plain BEN or XBEN stream.

    Stream-only: opening this on a ``.bendl`` bundle raises and points at :class:`BendlDecoder`.
    Sample counting is lazy and cached.
    """

    def __init__(self, file_path: StrPath, mode: AssignmentFormat = "ben") -> None: ...
    def __iter__(self) -> Iterator[list[int]]: ...
    def __next__(self) -> list[int]: ...
    def __len__(self) -> int: ...
    def count_samples(self) -> int: ...
    def subsample_indices(self, indices: Sequence[int]) -> "BenDecoder": ...
    def subsample_range(self, start: int, end: int) -> "BenDecoder": ...
    def subsample_every(self, step: int, offset: int = 1) -> "BenDecoder": ...
    def assignment_format(self) -> AssignmentFormat: ...

class BenEncoder:
    """Encoder for plain Binary Ensemble (`.ben`) streams."""

    def __init__(
        self,
        file_path: StrPath,
        overwrite: bool = False,
        variant: Variant = "twodelta",
    ) -> None: ...
    def write(self, assignment: Sequence[int]) -> None: ...
    def close(self) -> None: ...
    def __enter__(self) -> "BenEncoder": ...
    def __exit__(
        self,
        exc_type: type[BaseException] | None,
        exc: BaseException | None,
        tb: TracebackType | None,
    ) -> bool: ...

# ---------------------------------------------------------------------------
# Bundle decoder / encoder (.bendl)
# ---------------------------------------------------------------------------

class BendlDecoder:
    """Reader and iterator for a ``.bendl`` bundle.

    Bundle-only: opening this on a plain ``.ben``/``.xben`` stream raises and points at
    :class:`BenDecoder`. Iteration walks the embedded assignment stream; the bundle directory and
    asset payloads are exposed through the inspection methods. A finalized assets-only bundle
    (empty stream) iterates to nothing with ``len == 0``.
    """

    def __init__(self, file_path: StrPath) -> None: ...
    def __iter__(self) -> Iterator[list[int]]: ...
    def __next__(self) -> list[int]: ...
    def __len__(self) -> int: ...
    def count_samples(self) -> int: ...
    def subsample_indices(self, indices: Sequence[int]) -> "BendlDecoder": ...
    def subsample_range(self, start: int, end: int) -> "BendlDecoder": ...
    def subsample_every(self, step: int, offset: int = 1) -> "BendlDecoder": ...
    def assignment_format(self) -> AssignmentFormat: ...
    def version(self) -> tuple[int, int]: ...
    # On-disk byte length of the embedded stream region, straight from the header (no decoding;
    # the same bytes extract_stream copies out). 0 for an assets-only bundle.
    def stream_size(self) -> int: ...
    # On-disk byte length of a named asset's stored payload, straight from the directory. For
    # xz-flagged assets this is the compressed size; len(read_asset_bytes(name)) is the decoded
    # size. Raises KeyError for an unknown name.
    def asset_size(self, name: str) -> int: ...
    def is_complete(self) -> bool: ...
    def asset_names(self) -> list[str]: ...
    def list_assets(self) -> list[AssetEntry]: ...
    # Verifies every asset checksum and the stream checksum against the raw on-disk bytes (no
    # decoding). Iteration/subsampling do not check checksums; call this when integrity matters.
    # Raises on any mismatch or on an unfinalized bundle.
    def verify(self) -> None: ...
    def read_asset_bytes(self, name: str) -> bytes: ...
    def read_json_asset(self, name: str) -> Any: ...
    # Returns a NetworkX graph rebuilt from the stored adjacency JSON, or ``None`` if absent.
    # Use ``read_json_asset("graph.json")`` for the raw parsed dict.
    def read_graph(self) -> nx.Graph | None: ...
    def read_metadata(self) -> Any | None: ...
    def read_node_permutation_map(self) -> NodePermutationMap | None: ...
    def extract_stream(
        self,
        out_path: StrPath,
        overwrite: bool = False,
        allow_unfinalized: bool = False,
    ) -> None: ...

class BendlStreamSession:
    """Single-use context manager over a bundle's assignment stream.

    Obtained from :meth:`BendlEncoder.stream`; finalizes the bundle on a clean close and leaves
    it unfinalized if the context exits via an exception.
    """

    def write(self, assignment: Sequence[int]) -> None: ...
    def close(self) -> None: ...
    def __enter__(self) -> "BendlStreamSession": ...
    def __exit__(
        self,
        exc_type: type[BaseException] | None,
        exc: BaseException | None,
        tb: TracebackType | None,
    ) -> bool: ...

class BendlEncoder:
    """Writer for a ``.bendl`` bundle (create mode) or appender (append mode)."""

    def __init__(self, file_path: StrPath, overwrite: bool = False) -> None: ...
    @staticmethod
    def append(file_path: StrPath) -> "BendlEncoder": ...
    # The raw core surface takes the payload as already-coerced bytes; the bundle facade accepts
    # richer payload shapes (and content_type="file").
    def add_asset(
        self, name: str, payload: bytes, content_type: Literal["json", "text", "binary"]
    ) -> None: ...
    # Drops the directory entry only (payload bytes become dead space until the next
    # whole-bundle rewrite compacts them); frees the name for re-add. KeyError if absent.
    def remove_asset(self, name: str) -> None: ...
    # Drops the entry and reclaims its bytes as one operation; on error the bundle is left
    # untouched with the asset still present. The facade's remove_asset calls this.
    def remove_asset_compacting(self, name: str) -> None: ...
    def add_metadata(self, metadata: MetadataInput) -> None: ...
    # Returns the (possibly reordered) graph as a NetworkX graph, matching
    # BendlDecoder.read_graph. sort defaults to "mlc"; sort="key" sorts by `key`; sort=None
    # stores raw.
    def add_graph(
        self, graph: GraphInput, sort: SortMethod | None = "mlc", key: str | None = None
    ) -> nx.Graph: ...
    # The embedded stream is always BEN at write time; XBEN bundles are produced by recompressing
    # a finished bundle (see binary_ensemble.bundle.compress_stream).
    def stream(self, *, variant: Variant = "twodelta") -> BendlStreamSession: ...
    def close(self) -> None: ...
    def __enter__(self) -> "BendlEncoder": ...
    def __exit__(
        self,
        exc_type: type[BaseException] | None,
        exc: BaseException | None,
        tb: TracebackType | None,
    ) -> bool: ...

# ---------------------------------------------------------------------------
# Whole-file stream / JSONL transforms
# ---------------------------------------------------------------------------

def encode_jsonl_to_ben(
    in_file: StrPath,
    out_file: StrPath,
    overwrite: bool = False,
    variant: Variant = "twodelta",
) -> None: ...
def encode_jsonl_to_xben(
    in_file: StrPath,
    out_file: StrPath,
    overwrite: bool = False,
    variant: Variant = "twodelta",
    n_threads: int | None = None,
    compression_level: int | None = None,
    xz_block_size: int | None = None,
) -> None: ...
def encode_ben_to_xben(
    in_file: StrPath,
    out_file: StrPath,
    overwrite: bool = False,
    n_threads: int | None = None,
    compression_level: int | None = None,
    xz_block_size: int | None = None,
) -> None: ...
def decode_ben_to_jsonl(in_file: StrPath, out_file: StrPath, overwrite: bool = False) -> None: ...
def decode_xben_to_jsonl(in_file: StrPath, out_file: StrPath, overwrite: bool = False) -> None: ...
def decode_xben_to_ben(in_file: StrPath, out_file: StrPath, overwrite: bool = False) -> None: ...

# ---------------------------------------------------------------------------
# Graph reordering and bundle recompression
# ---------------------------------------------------------------------------

def graph_reorder(
    graph: GraphInput, sort: SortMethod = "mlc", key: str | None = None
) -> tuple[nx.Graph, NodePermutationMap]: ...

# Rewrites the bundle without unreferenced byte ranges (dead space from remove_asset and
# superseded directories). Assets carried by decoded payload; stream bytes copied verbatim
# (checksum-verified); wire format preserved.
def compact_bundle(in_file: StrPath, out_file: StrPath, overwrite: bool = False) -> None: ...

# In-place compaction choosing the cheapest strategy. Returns "none" (already compact),
# "tail" (post-stream tail rebuilt; stream untouched and not verified), or "full"
# (whole-bundle verified rewrite via temp file + atomic swap).
def compact_bundle_in_place(path: StrPath) -> Literal["none", "tail", "full"]: ...
def recompress_bundle(in_file: StrPath, out_file: StrPath, overwrite: bool = False) -> None: ...
def relabel_bundle(
    in_file: StrPath,
    out_file: StrPath,
    sort: SortMethod = "mlc",
    key: str | None = None,
    overwrite: bool = False,
) -> None: ...
