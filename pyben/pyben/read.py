from typing import Optional
from ._core.read import read_single_assignment as _impl  # type: ignore


def read_single_assignment(file_path: str, sample_number: int) -> Optional[list[int]]:
    """
    Read a single assignment from a `.ben` file.

    Parameters
    ----------
    file_path : str
        Path to the `.ben` file.
    sample_number : int
        0-based sample index to extract.

    Returns
    -------
    Optional[list[int]]
        The assignment vector if present; otherwise None.
    """
    return _impl(file_path, sample_number)


__all__ = ["read_single_assignment"]
