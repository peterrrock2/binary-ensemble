use super::cursor::SampleCursor;
use super::helpers::{detect_is_bundle, warn_xben_startup};
use super::types::{DecoderMode, StreamSource};
use pyo3::exceptions::{PyException, PyIOError};
use pyo3::prelude::*;
use std::path::PathBuf;

/// Iterator over the assignments in a plain BEN or XBEN stream.
///
/// Iterate the decoder to yield one assignment at a time, each a ``list[int]`` of district
/// ids in dual-graph node order. ``len()`` reports the (expanded) sample count and is cheap
/// to call, so it is safe to use for a progress bar. The encoding variant is detected
/// automatically from the stream, so it is never passed when reading.
///
/// This decoder is stream-only: opening it on a ``.bendl`` bundle raises and points the
/// caller at :class:`~binary_ensemble.bundle.BendlDecoder`, which carries the bundle
/// inspection surface (assets, embedded graph, metadata). This mirrors the ``ben`` vs
/// ``bendl`` split of the command-line tools.
///
/// Args:
///     file_path (StrPath): Path to the input ``.ben`` or ``.xben`` file (``str`` or
///         ``os.PathLike``).
///     mode (AssignmentFormat, optional): Which reader to use — ``"ben"`` or ``"xben"``.
///         Opening an XBEN stream warns about a one-time decompression startup cost.
///         Default is ``"ben"``.
///
/// Raises:
///     Exception: If ``file_path`` is a ``.bendl`` bundle (use
///         :class:`~binary_ensemble.bundle.BendlDecoder` instead), or ``mode`` does not
///         match the file's actual format.
///     OSError: If the file cannot be opened or its banner is malformed.
///
/// Example:
///     >>> from binary_ensemble import BenDecoder
///     >>> for assignment in BenDecoder("plans.ben"):
///     ...     print(assignment[:8])
#[pyclass(module = "binary_ensemble", name = "BenDecoder", unsendable)]
pub struct PyBenDecoder {
    cursor: SampleCursor,
}

#[pymethods]
impl PyBenDecoder {
    /// Open a decoder on a plain ``.ben`` or ``.xben`` file.
    ///
    /// The file's leading bytes are sniffed and a ``.bendl`` bundle is rejected. ``mode``
    /// selects between the BEN and XBEN readers; opening an XBEN stream pays a one-time
    /// decompression startup cost.
    ///
    /// Args:
    ///     file_path (StrPath): Path to the input ``.ben`` or ``.xben`` file (``str`` or
    ///         ``os.PathLike``).
    ///     mode (AssignmentFormat, optional): Either ``"ben"`` or ``"xben"``. Default is
    ///         ``"ben"``.
    ///
    /// Raises:
    ///     Exception: If ``file_path`` is a ``.bendl`` bundle (use
    ///         :class:`~binary_ensemble.bundle.BendlDecoder` instead).
    ///     OSError: If the file cannot be opened or its banner is malformed.
    #[new]
    #[pyo3(signature = (file_path, mode = "ben"))]
    #[pyo3(text_signature = "(file_path, mode='ben')")]
    fn new(py: Python<'_>, file_path: PathBuf, mode: &str) -> PyResult<Self> {
        // Validate the mode string up front so "Unknown mode" is reported regardless of whether the
        // file exists or turns out to be a bundle.
        let parsed_mode = DecoderMode::parse(mode)?;
        let is_bundle = detect_is_bundle(&file_path).map_err(|e| {
            PyIOError::new_err(format!("Failed to open {}: {e}", file_path.display()))
        })?;

        if is_bundle {
            return Err(PyException::new_err(format!(
                "{} is a .bendl file, not a plain BEN/XBEN stream. Open it with \
                 binary_ensemble.bundle.BendlDecoder instead.",
                file_path.display()
            )));
        }

        if matches!(parsed_mode, DecoderMode::XBen) {
            warn_xben_startup(py)?;
        }

        // For plain streams, opening the file as a BEN/XBEN reader is the only way to learn the
        // variant — keep eager construction so a malformed-banner error surfaces at open time.
        let mut cursor = SampleCursor::new(StreamSource::Plain { path: file_path }, parsed_mode);
        cursor.prime_iter()?;
        Ok(Self { cursor })
    }

    /// Return ``self`` as a fresh iterator over the stream.
    ///
    /// Restarting rebuilds the underlying frame walker, so a decoder can be iterated more
    /// than once. Any subsample selection installed via a ``subsample_*`` method is
    /// reapplied on each restart.
    fn __iter__(mut slf: PyRefMut<Self>) -> PyResult<Py<Self>> {
        slf.cursor.restart()?;
        Ok(slf.into())
    }

    /// Return the next assignment, or raise ``StopIteration`` at the end of the stream.
    fn __next__(&mut self) -> PyResult<Option<Vec<u16>>> {
        self.cursor.next()
    }

    /// Return the (expanded) number of samples, for use as a progress-bar total.
    fn __len__(&mut self, py: Python<'_>) -> PyResult<usize> {
        self.cursor.len(py)
    }

    /// Count the samples in the stream.
    ///
    /// The result is the *expanded* sample count: a frame that repeats five identical
    /// samples contributes five. The first call walks the stream to count; the result is
    /// cached, so repeated calls (and ``len()``) are cheap afterwards.
    ///
    /// Returns:
    ///     int: The number of samples in the stream.
    #[pyo3(text_signature = "(self)")]
    fn count_samples(&mut self, py: Python<'_>) -> PyResult<usize> {
        self.cursor.count_samples(py)
    }

    /// Restrict iteration to the samples at the given 1-indexed positions.
    ///
    /// Skipped samples are never materialized as Python lists, and where the encoding
    /// variant allows it (``standard``, ``mkv_chain``) whole frames are skipped without
    /// being unpacked, so this stays fast on large ensembles.
    ///
    /// Args:
    ///     indices (Sequence[int]): The 1-indexed sample numbers to keep. Duplicates are dropped;
    /// an         unsorted list is sorted, with a ``UserWarning``.
    ///
    /// Returns:
    ///     BenDecoder: ``self``, so the call can be chained directly into a ``for`` loop.
    ///
    /// Raises:
    ///     Exception: If ``indices`` is empty, contains ``0`` (indices are 1-based), or
    ///         contains an index greater than the number of samples in the stream.
    ///
    /// Example:
    ///     >>> for plan in BenDecoder("plans.ben").subsample_indices([1, 500, 9999]):
    ///     ...     ...
    #[pyo3(text_signature = "(self, indices, /)")]
    fn subsample_indices<'py>(
        mut slf: PyRefMut<'py, Self>,
        indices: Vec<usize>,
        py: Python<'_>,
    ) -> PyResult<Py<Self>> {
        slf.cursor.subsample_indices(indices, py)?;
        Ok(slf.into())
    }

    /// Restrict iteration to a contiguous, 1-indexed inclusive range of samples.
    ///
    /// Args:
    ///     start (int): First sample number to keep (1-indexed, inclusive).
    ///     end (int): Last sample number to keep (1-indexed, inclusive).
    ///
    /// Returns:
    ///     BenDecoder: ``self``, for chaining into a ``for`` loop.
    ///
    /// Raises:
    ///     Exception: If ``start`` is ``0``, ``end`` is less than ``start``, or ``end``
    ///         is greater than the number of samples in the stream.
    ///
    /// Example:
    ///     >>> list(BenDecoder("plans.ben").subsample_range(10, 15))
    ///     # samples 10, 11, 12, 13, 14, and 15
    #[pyo3(text_signature = "(self, start, end, /)")]
    fn subsample_range<'py>(
        mut slf: PyRefMut<'py, Self>,
        start: usize,
        end: usize,
        py: Python<'_>,
    ) -> PyResult<Py<Self>> {
        slf.cursor.subsample_range(start, end, py)?;
        Ok(slf.into())
    }

    /// Restrict iteration to every ``step``-th sample.
    ///
    /// Args:
    ///     step (int): Stride between kept samples (e.g. ``10`` keeps every tenth sample).
    ///     offset (int, optional): 1-indexed position of the first kept sample. Default is ``1``.
    ///
    /// Returns:
    ///     BenDecoder: ``self``, for chaining into a ``for`` loop.
    ///
    /// Raises:
    ///     Exception: If ``step`` or ``offset`` is ``0`` (both are 1-based).
    ///
    /// Example:
    ///     >>> for plan in BenDecoder("plans.ben").subsample_every(1000):
    ///     ...     ...
    #[pyo3(signature = (step, offset=1))]
    #[pyo3(text_signature = "(self, step, offset=1)")]
    fn subsample_every<'py>(
        mut slf: PyRefMut<'py, Self>,
        step: usize,
        offset: usize,
        py: Python<'_>,
    ) -> PyResult<Py<Self>> {
        slf.cursor.subsample_every(step, offset, py)?;
        Ok(slf.into())
    }

    /// Return the container format of the underlying stream as ``"ben"`` or ``"xben"``.
    #[pyo3(text_signature = "(self)")]
    fn assignment_format(&self) -> &'static str {
        self.cursor.mode().as_str()
    }
}
