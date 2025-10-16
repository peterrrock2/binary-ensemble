from typing import Iterable, Iterator, Literal
from pathlib import Path

class PyBenDecoder:
    """Iterator over assignments in a BEN or XBEN file.
    Open a decoder over a BEN (`.ben`) or XBEN (`.xben`) file.

    Parameters
    ----------
    file_path :
        Path to the input file.
    mode : {"ben", "xben"}, default "ben"
        Select container format.

    Raises
    ------
    OSError
        If the file cannot be opened.
    Exception
        If the underlying decoder fails to initialize.
    """

    def __init__(
        self, file_path: str | Path, mode: Literal["ben", "xben"] = "ben"
    ) -> None: ...
    def __iter__(self) -> Iterator[list[int]]: ...
    def __next__(self) -> list[int]: ...
    def subsample_indices(self, indices: Iterable[int]) -> "PyBenDecoder":
        """Keep only the given **1-based** sample indices.

        Duplicates are ignored and order is irrelevant; the set is sorted & deduped internally.
        Returns the same decoder (fluent API).


        Arguments
        ---------
        indices :
            Iterable of 1-based sample indices to keep.

        Returns
        -------
        PyBenDecoder
            The same decoder (fluent API).
        """
        ...

    def subsample_range(self, start: int, end: int) -> "PyBenDecoder":
        """Keep only samples in the inclusive **1-based** range [start, end].

        Arguments
        ---------
        start :
            1-based index of the first sample to keep.
        end :
            1-based index of the last sample to keep.

        Returns
        -------
        PyBenDecoder
            The same decoder (fluent API).
        """
        ...

    def subsample_every(self, step: int, offset: int = 1) -> "PyBenDecoder":
        """Keep every `step`-th sample starting at **1-based** `offset`.
        Returns the same decoder (fluent API).

        Arguments
        ---------
        step :
            Step size (keep every `step`-th sample).
        offset :
            1-based index of the first sample to keep (default: 1).

        Returns
        -------
        PyBenDecoder
            The same decoder (fluent API).
        """
        ...

class PyBenEncoder:
    """Encoder for Binary Ensemble (.ben) files.


    The encoder supports writing assignments to a BEN file using a context manager and the `write`
    method.


    Example
    -------


    .. code-block:: python

        from pyben import PyBenEncoder

        assignments = [
            [1, 2, 1, 1, 2, 2],
            [2, 1, 1, 2, 2, 1],
            [1, 1, 2, 1, 2, 2],
        ]

        with PyBenEncoder("output.ben", overwrite=True) as encoder:
            for assignment in assignments:
                encoder.write(assignment)

    """

    def __init__(
        self,
        file_path: str | Path,
        overwrite: bool = False,
        variant: Literal["standard", "mkv_chain"] | None = None,
    ) -> None:
        """Initializes the encoder and opens the underlying file.

        Parameters
        ----------
        file_path :
            Path to the output BEN file.
        overwrite :
            Whether to overwrite the output file if it exists. Defaults to False.
        variant : {"standard", "markov"}, optional
            Select BEN variant. If None, defaults to "markov" (equivalent to "mkv_chain").

        Raises
        ------
        OSError
            If the file cannot be opened.
        Exception
            If the underlying encoder fails to initialize.
        """
        ...

    def write(self, assignment: list[int]) -> None:
        """Write a single assignment to the BEN file.

        Parameters
        ----------
        assignment :
            List of integers representing the assignment.
        """
        ...

    def close(self) -> None:
        """Closes the encoder and the underlying file.

        Also handles flushing any buffered data.
        """
        ...

    def __enter__(self) -> "PyBenEncoder": ...
    def __exit__(self, exc_type, exc, tb) -> bool: ...

def decompress_ben_to_jsonl(
    in_file: str | Path, out_file: str | Path, overwrite: bool = False
) -> None:
    """Converts a BEN file to a JSONL file.

    Parameters
    ----------
    in_file :
        Path to the input BEN file.
    out_file :
        Path to the output JSONL file.
    overwrite :
        Whether to overwrite the output file if it exists. Defaults to False.

    Raises
    ------
    OSError
        If the input file cannot be opened or the output file cannot be created.
    """
    ...

def decompress_xben_to_jsonl(
    in_file: str | Path, out_file: str | Path, overwrite: bool = False
) -> None:
    """Converts an XBEN file to a JSONL file.

    Parameters
    ----------
    in_file :
        Path to the input XBEN file.
    out_file :
        Path to the output JSONL file.
    overwrite :
        Whether to overwrite the output file if it exists. Defaults to False.

    Raises
    ------
    OSError
        If the input file cannot be opened or the output file cannot be created.
    """
    ...

def decompress_xben_to_ben(
    in_file: str | Path, out_file: str | Path, overwrite: bool = False
) -> None:
    """Converts an XBEN file to a BEN file.

    Parameters
    ----------
    in_file :
        Path to the input XBEN file.
    out_file :
        Path to the output BEN file.
    overwrite :
        Whether to overwrite the output file if it exists. Defaults to False.

    Raises
    ------
    OSError
        If the input file cannot be opened or the output file cannot be created.
    """
    ...

def compress_jsonl_to_ben(
    in_file: str | Path,
    out_file: str | Path,
    overwrite: bool = False,
    variant: Literal["standard", "mkv_chain"] | None = None,
) -> None:
    """Converts a JSONL file to a BEN file.

    Parameters
    ----------
    in_file :
        Path to the input JSONL file.
    out_file :
        Path to the output BEN file.
    overwrite :
        Whether to overwrite the output file if it exists. Defaults to False.
    variant : {"standard", "markov"}, optional
        Select BEN variant. If None, defaults to "markov" (equivalent to "mkv_chain").

    Raises
    ------
    OSError
        If the input file cannot be opened or the output file cannot be created.
    ValueError
        If the input file is not a valid JSONL file or if the variant cannot be inferred.
    """
    ...

def compress_jsonl_to_xben(
    in_file: str | Path,
    out_file: str | Path,
    overwrite: bool = False,
    variant: Literal["standard", "mkv_chain"] | None = None,
    n_threads: int | None = None,
    compression_level: int | None = None,
) -> None:
    """Converts a JSONL file to an XBEN file.

    Parameters
    ----------
    in_file :
        Path to the input JSONL file.
    out_file :
        Path to the output XBEN file.
    overwrite :
        Whether to overwrite the output file if it exists. Defaults to False.
    variant : {"standard", "markov"}, optional
        Select BEN variant. If None, defaults to "markov" (equivalent to "mkv_chain").
    n_threads :
        Number of threads to use for compression. If None, defaults to the number of CPU cores.
    compression_level :
        Compression level to use for LZMA compression (0-9). If None, defaults to 9 (highest).

    Raises
    ------
    OSError
        If the input file cannot be opened or the output file cannot be created.
    ValueError
        If the input file is not a valid JSONL file or if the variant cannot be inferred.
    """
    ...

def compress_ben_to_xben(
    in_file: str | Path,
    out_file: str | Path,
    overwrite: bool = False,
    n_threads: int | None = None,
    compression_level: int | None = None,
) -> None:
    """Converts a BEN file to an XBEN file.

    Parameters
    ----------
    in_file :
        Path to the input BEN file.
    out_file :
        Path to the output XBEN file.
    overwrite :
        Whether to overwrite the output file if it exists. Defaults to False.
    n_threads :
        Number of threads to use for compression. If None, defaults to the number of CPU cores.
    compression_level :
        Compression level to use for LZMA compression (0-9). If None, defaults to 9 (highest).

    Raises
    ------
    OSError
        If the input file cannot be opened or the output file cannot be created.
    """
    ...
