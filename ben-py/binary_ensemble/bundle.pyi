from types import TracebackType
from typing import Literal, overload

import networkx as nx

from binary_ensemble._core import BendlDecoder as BendlDecoder
from binary_ensemble._core import BendlStreamSession as BendlStreamSession
from binary_ensemble.types import (
    BinaryAssetPayload,
    GraphInput,
    JsonAssetPayload,
    MetadataInput,
    SortMethod,
    StrPath,
    TextAssetPayload,
    Variant,
)

__all__ = [
    "BendlEncoder",
    "BendlDecoder",
    "BendlStreamSession",
    "compress_stream",
    "relabel_bundle",
]

class BendlEncoder:
    def __init__(self, file_path: StrPath, overwrite: bool = False) -> None: ...
    @classmethod
    def append(cls, file_path: StrPath) -> "BendlEncoder": ...
    def add_graph(
        self,
        graph: GraphInput,
        sort: SortMethod | None = "mlc",
        key: str | None = None,
        *,
        compress: bool | None = None,
        compression_level: int | None = None,
    ) -> nx.Graph: ...
    def add_metadata(
        self,
        metadata: MetadataInput,
        *,
        compress: bool | None = None,
        compression_level: int | None = None,
    ) -> None: ...
    @overload
    def add_asset(
        self,
        name: str,
        payload: JsonAssetPayload,
        content_type: Literal["json"],
        *,
        compress: bool | None = None,
        compression_level: int | None = None,
    ) -> None: ...
    @overload
    def add_asset(
        self,
        name: str,
        payload: TextAssetPayload,
        content_type: Literal["text"],
        *,
        compress: bool | None = None,
        compression_level: int | None = None,
    ) -> None: ...
    @overload
    def add_asset(
        self,
        name: str,
        payload: BinaryAssetPayload,
        content_type: Literal["binary"],
        *,
        compress: bool | None = None,
        compression_level: int | None = None,
    ) -> None: ...
    @overload
    def add_asset(
        self,
        name: str,
        payload: StrPath,
        content_type: Literal["file"],
        *,
        compress: bool | None = None,
        compression_level: int | None = None,
    ) -> None: ...
    # Drops the directory entry and compacts the bundle in place, so the payload bytes are
    # actually reclaimed; frees the name for re-add. KeyError if absent. (The raw
    # _core.BendlEncoder.remove_asset is the cheap, directory-only form.)
    def remove_asset(self, name: str) -> None: ...
    def ben_stream(self, *, variant: Variant = "twodelta") -> BendlStreamSession: ...
    def close(self) -> None: ...
    def __enter__(self) -> "BendlEncoder": ...
    def __exit__(
        self,
        exc_type: type[BaseException] | None,
        exc: BaseException | None,
        tb: TracebackType | None,
    ) -> bool: ...

# out_file=None means in place: the result is atomically swapped over `path`.
def compress_stream(
    path: StrPath,
    out_file: StrPath | None = None,
    overwrite: bool = False,
) -> None: ...
def relabel_bundle(
    path: StrPath,
    out_file: StrPath | None = None,
    sort: SortMethod = "mlc",
    key: str | None = None,
    overwrite: bool = False,
) -> None: ...
