use ben::decode::BenDecoder;
use pyo3::exceptions::{PyException, PyIOError};
use pyo3::prelude::PyResult;
use pyo3::{pyclass, pymethods, pymodule, Py, PyRefMut};
use std::fs::File;
use std::io::BufReader;

#[pyclass]
pub struct PyBenDecoder {
    decoder: BenDecoder<BufReader<File>>,
    current_assignment: Option<Vec<u16>>,
    remaining_count: u16,
}

#[pymethods]
impl PyBenDecoder {
    #[new]
    fn new(file_path: String) -> PyResult<Self> {
        let file = File::open(&file_path)
            .map_err(|e| PyIOError::new_err(format!("Failed to open file {}: {}", file_path, e)))?;
        let decoder = BenDecoder::new(BufReader::new(file))
            .map_err(|e| PyException::new_err(format!("Failed to create BenDecoder: {}", e)))?;
        Ok(PyBenDecoder {
            decoder: decoder,
            current_assignment: None,
            remaining_count: 0,
        })
    }

    fn __iter__(slf: PyRefMut<Self>) -> PyResult<Py<Self>> {
        Ok(slf.into())
    }

    fn __next__(mut slf: PyRefMut<Self>) -> PyResult<Option<Vec<u16>>> {
        if slf.remaining_count > 0 {
            // If there are remaining items, return the current assignment
            slf.remaining_count -= 1;
            let assgn = slf.current_assignment.as_ref().unwrap().clone();
            return Ok(Some(assgn));
        }

        match slf.decoder.next() {
            Some(Ok((assignment, count))) => {
                assert!(
                    count > 0,
                    "Found a non-positive count in the data. The data may be corrupted."
                );
                slf.current_assignment = Some(assignment.clone());
                slf.remaining_count = count - 1;
                Ok(Some(assignment))
            }
            Some(Err(e)) => Err(PyException::new_err(format!(
                "Error decoding next item: {}",
                e
            ))),
            None => Ok(None),
        }
    }
}

#[pymodule(name = "pyben")]
mod pyben {
    #[pymodule_export]
    use super::PyBenDecoder;
}
