from typing import TYPE_CHECKING, Iterable, Iterator, Literal, Self
from pathlib import Path

# Runtime objects from the compiled extension
from ._core import PyBenDecoder, PyBenEncoder
from . import read

# LSP/type-checker facades (zero runtime cost) for better hover docs/signatures
if TYPE_CHECKING:

    class PyBenEncoder:
        """Encoder for Binary Ensemble (`.ben`) files.

        Example
        -------
        >>> from pyben import PyBenEncoder
        >>> with PyBenEncoder("out.ben", overwrite=True, variant="mkv_chain") as enc:
        ...     enc.write([1, 1, 2, 2])
        """

        def __init__(
            self,
            file_path: str | Path,
            overwrite: bool = False,
            variant: str | None = None,
        ) -> None: ...
        def write(self, assignment: list[int]) -> None: ...
        def close(self) -> None: ...
        def __enter__(self) -> "PyBenEncoder": ...
        def __exit__(self, exc_type, exc, tb) -> bool: ...

    class PyBenDecoder:
        """Iterator over assignments in a `.ben` file."""

        def __init__(
            self,
            file_path: str | Path,
            mode: Literal["ben", "xben"] = "ben",
        ) -> None: ...

        # Fluent subsampling API (returns self)
        def subsample_indices(self, indices: Iterable[int]) -> Self: ...
        def subsample_range(self, start: int, end: int) -> Self: ...
        def subsample_every(self, step: int, offset: int = 1) -> Self: ...

        # Iterator protocol
        def __iter__(self) -> Iterator[list[int]]: ...
        def __next__(self) -> list[int]: ...


__all__ = ["PyBenDecoder", "PyBenEncoder", "read"]
