from typing import Any, Optional

from binary_ensemble._core import BendlDecoder as BendlDecoder
from binary_ensemble._core import BendlStreamSession as BendlStreamSession

__all__ = [
    "BendlEncoder",
    "BendlDecoder",
    "BendlStreamSession",
    "compress_stream",
    "relabel_bundle",
]

class BendlEncoder:
    def __init__(self, file_path, overwrite: bool = False) -> None: ...
    @classmethod
    def append(cls, file_path) -> "BendlEncoder": ...
    def add_graph(
        self, graph: Any, sort: Optional[str] = "mlc", key: Optional[str] = None
    ) -> Any: ...
    def add_metadata(self, metadata: Any) -> None: ...
    def add_asset(
        self,
        name: str,
        payload: Any,
        content_type: str,
    ) -> None: ...
    def stream(
        self, format: str = "ben", variant: Optional[str] = None
    ) -> BendlStreamSession: ...
    def close(self) -> None: ...
    def __enter__(self) -> "BendlEncoder": ...
    def __exit__(self, exc_type, exc, tb) -> bool: ...

def compress_stream(
    path,
    out_file=None,
    in_place: bool = False,
) -> None: ...
def relabel_bundle(
    path,
    out_file=None,
    sort: str = "mlc",
    key: Optional[str] = None,
    in_place: bool = False,
) -> None: ...
