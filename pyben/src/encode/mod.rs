use ben::encode::BenEncoder;
use ben::BenVariant;
use pyo3::exceptions::{PyException, PyIOError};
use pyo3::prelude::PyResult;
use pyo3::{pyclass, pymethods};
use std::fs::File;
use std::io::BufWriter;
use std::path::{Path, PathBuf};

#[pyclass]
pub struct PyBenEncoder {
    encoder: Option<BenEncoder<BufWriter<File>>>,
}

#[pymethods]
impl PyBenEncoder {
    #[new]
    #[pyo3(signature = (file_path, overwrite = false, variant = None))]
    fn new(file_path: PathBuf, overwrite: bool, variant: Option<String>) -> PyResult<Self> {
        let ben_var = match variant.as_deref() {
            Some("standard") => BenVariant::Standard,
            Some("mkv_chain") => BenVariant::MkvChain,
            Some(other) => {
                return Err(PyException::new_err(format!(
                    "Unknown variant: {}. Supported variants are 'standard' and 'mkv_chain'.",
                    other
                )))
            }
            _ => BenVariant::MkvChain,
        };

        let path = Path::new(&file_path);
        let file = if overwrite {
            File::options()
                .write(true)
                .create(true)
                .truncate(true)
                .open(&file_path)
                .map_err(|e| {
                    PyIOError::new_err(format!("Failed to create file {:?}: {}", file_path, e))
                })?
        } else {
            if path.exists() {
                return Err(PyIOError::new_err(format!(
                    "File {:?} already exists. Use overwrite=True to overwrite it.",
                    file_path
                )));
            }
            File::options()
                .write(true)
                .create_new(true)
                .open(&file_path)
                .map_err(|e| {
                    PyIOError::new_err(format!("Failed to create file {:?}: {}", file_path, e))
                })?
        };

        let encoder = BenEncoder::new(BufWriter::new(file), ben_var);
        Ok(PyBenEncoder {
            encoder: Some(encoder),
        })
    }

    fn write(&mut self, assignment: Vec<u16>) -> PyResult<()> {
        if let Some(enc) = self.encoder.as_mut() {
            enc.write_assignment(assignment)
                .map_err(|e| PyIOError::new_err(format!("Failed to encode assignment: {}", e)))?;
            Ok(())
        } else {
            Err(PyIOError::new_err("Encoder has already been closed."))
        }
    }

    // the finsish is wrong here and double writes the last line

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

// use ben::encode::ben_encode_xben;
//
// #[pyfunction]
// pub fn convert_ben_to_xben(in_file: String, out_file: String) -> PyResult<()> {
//     Ok(())
// }
