use crate::common::{open_output, parse_variant};
use binary_ensemble::io::writer::BenStreamWriter;
use pyo3::exceptions::PyIOError;
use pyo3::prelude::*;
use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::PathBuf;

/// Encoder for plain Binary Ensemble (`.ben`) streams.
///
/// This encoder writes a plain BEN stream with no bundle framing. To produce a `.bendl` bundle
/// (with an embedded graph, metadata, or other assets) use `binary_ensemble.bundle.BendlEncoder`.
#[pyclass(module = "binary_ensemble", name = "BenEncoder", unsendable)]
pub struct PyBenEncoder {
    writer: Option<BenStreamWriter<BufWriter<File>>>,
}

impl PyBenEncoder {
    fn map_io_err(err: io::Error) -> PyErr {
        PyIOError::new_err(format!("{err}"))
    }
}

#[pymethods]
impl PyBenEncoder {
    /// Open a new encoder that writes a plain `.ben` stream.
    ///
    /// # Arguments
    ///
    /// * `file_path` - Output path. Must not exist unless `overwrite=True`.
    /// * `overwrite` - Replace an existing file at `file_path`.
    /// * `variant` - BEN variant for the assignment stream (`"standard"`, `"mkv_chain"`, or
    ///   `"twodelta"`). Defaults to `"twodelta"` when `None`.
    #[new]
    #[pyo3(signature = (file_path, overwrite = false, variant = None))]
    #[pyo3(text_signature = "(file_path, overwrite=False, variant=None)")]
    fn new(file_path: PathBuf, overwrite: bool, variant: Option<String>) -> PyResult<Self> {
        let ben_var = parse_variant(variant.as_deref())?;
        let buf = open_output(&file_path, overwrite)?;
        let writer = BenStreamWriter::for_ben(buf, ben_var).map_err(Self::map_io_err)?;
        Ok(Self {
            writer: Some(writer),
        })
    }

    /// Encode a single assignment and append it to the output stream.
    #[pyo3(signature = (assignment))]
    #[pyo3(text_signature = "(assignment)")]
    fn write(&mut self, assignment: Vec<u16>) -> PyResult<()> {
        let writer = self
            .writer
            .as_mut()
            .ok_or_else(|| PyIOError::new_err("Encoder has already been closed."))?;
        writer
            .write_assignment(assignment)
            .map_err(Self::map_io_err)
    }

    /// Flush the assignment stream and close the underlying file. Idempotent.
    fn close(&mut self) -> PyResult<()> {
        let Some(writer) = self.writer.take() else {
            return Ok(());
        };
        let mut buf = writer.finish_into_inner().map_err(Self::map_io_err)?;
        buf.flush().map_err(Self::map_io_err)?;
        Ok(())
    }

    fn __enter__(slf: PyRefMut<Self>) -> PyRefMut<Self> {
        slf
    }

    fn __exit__(
        &mut self,
        _exc_type: Option<&Bound<'_, PyAny>>,
        _exc_value: Option<&Bound<'_, PyAny>>,
        _traceback: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<bool> {
        self.close()?;
        Ok(false)
    }
}
