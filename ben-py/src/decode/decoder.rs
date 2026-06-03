use super::cursor::SampleCursor;
use super::helpers::{detect_is_bundle, warn_xben_startup};
use super::types::{DecoderMode, StreamSource};
use pyo3::exceptions::{PyException, PyIOError};
use pyo3::prelude::*;
use std::path::PathBuf;

/// Iterator over assignments in a plain BEN or XBEN stream.
///
/// This decoder is stream-only: opening it on a `.bendl` bundle raises and points the caller at
/// `BendlDecoder`. Bundle inspection (assets, directory, embedded-stream extraction) lives on
/// `BendlDecoder`, mirroring the `ben` vs `bendl` CLI split.
#[pyclass(module = "binary_ensemble", name = "BenDecoder", unsendable)]
pub struct PyBenDecoder {
    cursor: SampleCursor,
}

#[pymethods]
impl PyBenDecoder {
    /// Open a decoder on a plain `.ben` or `.xben` file.
    ///
    /// The file's leading bytes are sniffed; a `.bendl` bundle is rejected with a pointer at
    /// `BendlDecoder`. `mode` selects between the BEN and XBEN readers and defaults to `"ben"`.
    ///
    /// # Arguments
    ///
    /// * `file_path` - Path to the input file.
    /// * `mode` - Either `"ben"` or `"xben"`.
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
                "{} is a .bendl bundle, not a plain BEN/XBEN stream. Open it with \
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

    /// Return `self` as an iterator, rebuilding the underlying frame walker so iteration can be
    /// restarted. A subsample selection installed via `subsample_*` is reapplied on each restart.
    fn __iter__(mut slf: PyRefMut<Self>) -> PyResult<Py<Self>> {
        slf.cursor.restart()?;
        Ok(slf.into())
    }

    fn __next__(&mut self) -> PyResult<Option<Vec<u16>>> {
        self.cursor.next()
    }

    // Because we want progress bars!!!
    fn __len__(&mut self, py: Python<'_>) -> PyResult<usize> {
        self.cursor.len(py)
    }

    #[pyo3(text_signature = "(self)")]
    fn count_samples(&mut self, py: Python<'_>) -> PyResult<usize> {
        self.cursor.count_samples(py)
    }

    #[pyo3(text_signature = "(self, indices, /)")]
    fn subsample_indices<'py>(
        mut slf: PyRefMut<'py, Self>,
        indices: Vec<usize>,
        py: Python<'_>,
    ) -> PyResult<Py<Self>> {
        slf.cursor.subsample_indices(indices, py)?;
        Ok(slf.into())
    }

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

    #[pyo3(signature = (step, offset=1))]
    fn subsample_every<'py>(
        mut slf: PyRefMut<'py, Self>,
        step: usize,
        offset: usize,
        py: Python<'_>,
    ) -> PyResult<Py<Self>> {
        slf.cursor.subsample_every(step, offset, py)?;
        Ok(slf.into())
    }

    /// Return the container format of the underlying stream as `"ben"` or `"xben"`.
    #[pyo3(text_signature = "(self)")]
    fn assignment_format(&self) -> &'static str {
        self.cursor.mode().as_str()
    }
}
