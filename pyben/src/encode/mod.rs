use crate::common::{open_input, open_output, parse_variant, validate_input_output_paths};
use binary_ensemble::codec::encode::{
    encode_ben_to_xben, encode_jsonl_to_ben, encode_jsonl_to_xben,
};
use binary_ensemble::io::writer::BenEncoder;
use pyo3::exceptions::PyIOError;
use pyo3::prelude::PyResult;
use pyo3::{pyclass, pyfunction, pymethods};
use std::fs::File;
use std::io::BufWriter;
use std::path::PathBuf;

#[pyclass]
pub struct PyBenEncoder {
    encoder: Option<BenEncoder<BufWriter<File>>>,
}

#[pymethods]
impl PyBenEncoder {
    #[new]
    #[pyo3(signature = (file_path, overwrite = false, variant = None))]
    #[pyo3(text_signature = "(file_path, overwrite=False, variant=None)")]
    fn new(file_path: PathBuf, overwrite: bool, variant: Option<String>) -> PyResult<Self> {
        let ben_var = parse_variant(variant.as_deref())?;
        let writer = open_output(&file_path, overwrite)?;

        let encoder = BenEncoder::new(writer, ben_var);
        Ok(PyBenEncoder {
            encoder: Some(encoder),
        })
    }

    #[pyo3(signature = (assignment))]
    #[pyo3(text_signature = "(assignment)")]
    fn write(&mut self, assignment: Vec<u16>) -> PyResult<()> {
        if let Some(enc) = self.encoder.as_mut() {
            enc.write_assignment(assignment)
                .map_err(|e| PyIOError::new_err(format!("Failed to encode assignment: {}", e)))?;
            Ok(())
        } else {
            Err(PyIOError::new_err("Encoder has already been closed."))
        }
    }

    fn close(&mut self) -> PyResult<()> {
        if let Some(mut enc) = self.encoder.take() {
            enc.finish().map_err(|e| {
                PyIOError::new_err(format!("Failed to flush encoder when closing: {}", e))
            })?;
        }
        Ok(())
    }

    fn __enter__(slf: pyo3::PyRefMut<Self>) -> pyo3::PyRefMut<Self> {
        slf
    }

    fn __exit__(
        &mut self,
        _exc_type: Option<&pyo3::Bound<'_, pyo3::types::PyAny>>,
        _exc_value: Option<&pyo3::Bound<'_, pyo3::types::PyAny>>,
        _traceback: Option<&pyo3::Bound<'_, pyo3::types::PyAny>>,
    ) -> PyResult<bool> {
        self.close()?;
        Ok(false)
    }
}

#[pyfunction]
#[pyo3(signature = (in_file, out_file, overwrite=false, n_threads = None, compression_level = None))]
#[pyo3(
    text_signature = "(in_file, out_file, overwrite=false, n_threads=None, compression_level=None)"
)]
pub fn compress_ben_to_xben(
    in_file: PathBuf,
    out_file: PathBuf,
    overwrite: bool,
    n_threads: Option<u32>,
    compression_level: Option<u32>,
) -> PyResult<()> {
    validate_input_output_paths(&in_file, &out_file)?;
    let reader = open_input(&in_file)?;
    let writer = open_output(&out_file, overwrite)?;

    encode_ben_to_xben(reader, writer, n_threads, compression_level, None).map_err(|e| {
        PyIOError::new_err(format!(
            "Failed to convert BEN to XBEN from {} to {}: {e}",
            in_file.display(),
            out_file.display()
        ))
    })?;

    Ok(())
}

#[pyfunction]
#[pyo3(signature = (in_file, out_file, overwrite=false, variant="mkv_chain"))]
#[pyo3(text_signature = "(in_file, out_file, overwrite=false, variant='mkv_chain')")]
pub fn compress_jsonl_to_ben(
    in_file: PathBuf,
    out_file: PathBuf,
    overwrite: bool,
    variant: &str,
) -> PyResult<()> {
    let ben_var = parse_variant(Some(variant))?;
    validate_input_output_paths(&in_file, &out_file)?;
    let reader = open_input(&in_file)?;
    let writer = open_output(&out_file, overwrite)?;

    encode_jsonl_to_ben(reader, writer, ben_var).map_err(|e| {
        PyIOError::new_err(format!(
            "Failed to convert JSONL to BEN from {} to {}: {e}",
            in_file.display(),
            out_file.display()
        ))
    })?;
    Ok(())
}

#[pyfunction]
#[pyo3(signature = (in_file, out_file, overwrite=false, variant="mkv_chain", n_threads=None, compression_level=None))]
#[pyo3(
    text_signature = "(in_file, out_file, overwrite=false, variant='mkv_chain', n_threads=None, compression_level=None)"
)]
pub fn compress_jsonl_to_xben(
    in_file: PathBuf,
    out_file: PathBuf,
    overwrite: bool,
    variant: &str,
    n_threads: Option<u32>,
    compression_level: Option<u32>,
) -> PyResult<()> {
    let ben_var = parse_variant(Some(variant))?;
    validate_input_output_paths(&in_file, &out_file)?;
    let reader = open_input(&in_file)?;
    let writer = open_output(&out_file, overwrite)?;

    encode_jsonl_to_xben(reader, writer, ben_var, n_threads, compression_level, None).map_err(|e| {
        PyIOError::new_err(format!(
            "Failed to convert JSONL to XBEN from {} to {}: {e}",
            in_file.display(),
            out_file.display()
        ))
    })?;
    Ok(())
}
