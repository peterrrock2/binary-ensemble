use ben::decode::read::extract_assignment_ben;
use pyo3::{pyfunction, PyResult};
use std::fs::File;

#[pyfunction]
#[pyo3(text_signature = "(file_path, sample_number)")]
pub fn read_single_assignment(file_path: String, sample_number: usize) -> PyResult<Vec<u16>> {
    let file = File::options().read(true).open(&file_path).map_err(|e| {
        pyo3::exceptions::PyIOError::new_err(format!("Failed to open file {}: {}", file_path, e))
    })?;
    let assignment = extract_assignment_ben(&file, sample_number).map_err(|e| {
        pyo3::exceptions::PyRuntimeError::new_err(format!("Failed to extract assignment: {}", e))
    })?;

    return Ok(assignment);
}
