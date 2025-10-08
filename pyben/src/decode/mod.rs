use ben::decode::{BenDecoder, MkvRecord, Selection, SubsampleDecoder, XBenDecoder};
use pyo3::exceptions::{PyException, PyIOError};
use pyo3::prelude::*;
use std::fs::File;
use std::io::{self, BufReader};
use std::path::PathBuf;

pub mod read;

type DynIter = Box<dyn Iterator<Item = io::Result<MkvRecord>> + Send>;

#[pyclass(module = "pyben", unsendable)]
pub struct PyBenDecoder {
    iter: DynIter,
    current_assignment: Option<Vec<u16>>,
    remaining_count: u16,
}

impl PyBenDecoder {
    fn take_iter(&mut self) -> DynIter {
        // replace with a correctly-typed empty iterator
        std::mem::replace(
            &mut self.iter,
            Box::new(std::iter::empty::<io::Result<MkvRecord>>()),
        )
    }
}

#[pymethods]
impl PyBenDecoder {
    #[new]
    #[pyo3(signature = (file_path, mode = "ben"))]
    fn new(file_path: PathBuf, mode: &str) -> PyResult<Self> {
        let file = File::options().read(true).open(&file_path).map_err(|e| {
            PyIOError::new_err(format!("Failed to open {}: {e}", file_path.display()))
        })?;
        let reader = BufReader::new(file);

        let iter: DynIter = match mode {
            "ben" => {
                let ben = BenDecoder::new(reader).map_err(|e| {
                    PyException::new_err(format!("Failed to create BenDecoder: {e}"))
                })?;
                Box::new(ben)
            }
            "xben" => {
                let xben = XBenDecoder::new(reader).map_err(|e| {
                    PyException::new_err(format!("Failed to create XBenDecoder: {e}"))
                })?;
                Box::new(xben)
            }
            _ => {
                return Err(PyException::new_err(
                    "Unknown mode. Supported modes are 'ben' and 'xben'.",
                ));
            }
        };

        Ok(Self {
            iter,
            current_assignment: ::std::option::Option::None,
            remaining_count: 0,
        })
    }

    fn __iter__(slf: PyRefMut<Self>) -> PyResult<Py<Self>> {
        Ok(slf.into())
    }

    fn __next__(mut slf: PyRefMut<Self>) -> PyResult<Option<Vec<u16>>> {
        if slf.remaining_count > 0 {
            slf.remaining_count -= 1;
            let a = slf.current_assignment.as_ref().unwrap().clone();
            return Ok(Some(a));
        }
        match slf.iter.next() {
            Some(Ok((assignment, count))) => {
                assert!(count > 0, "non-positive count; data may be corrupted");
                slf.current_assignment = Some(assignment.clone());
                slf.remaining_count = count - 1;
                Ok(Some(assignment))
            }
            Some(Err(e)) => Err(PyException::new_err(format!(
                "Error decoding next item: {e}"
            ))),
            ::std::option::Option::None => Ok(::std::option::Option::None),
        }
    }

    /// Keep only explicit 1-based indices (sorted & deduped internally).
    fn subsample_indices<'py>(
        mut slf: PyRefMut<'py, Self>,
        mut indices: Vec<usize>,
    ) -> PyResult<Py<Self>> {
        indices.sort_unstable();
        indices.dedup();
        let sel = Selection::Indices(indices.into_iter().peekable());

        let inner = slf.take_iter();
        slf.iter = Box::new(SubsampleDecoder::new(inner, sel));

        Ok(slf.into())
    }

    /// Keep only samples in inclusive 1-based range [start, end].
    fn subsample_range<'py>(
        mut slf: PyRefMut<'py, Self>,
        start: usize,
        end: usize,
    ) -> PyResult<Py<Self>> {
        if start == 0 || end < start {
            return Err(PyException::new_err(
                "range must be 1-based and end >= start",
            ));
        }
        let sel = Selection::Range { start, end };
        let inner = slf.take_iter();
        slf.iter = Box::new(SubsampleDecoder::new(inner, sel));
        Ok(slf.into())
    }

    /// Keep every `step`-th sample starting at 1-based `offset`.
    #[pyo3(signature = (step, offset=1))]
    fn subsample_every<'py>(
        mut slf: PyRefMut<'py, Self>,
        step: usize,
        offset: usize,
    ) -> PyResult<Py<Self>> {
        if step == 0 || offset == 0 {
            return Err(PyException::new_err("step and offset must be >= 1"));
        }
        let sel = Selection::Every { step, offset };
        let inner = slf.take_iter();
        slf.iter = Box::new(SubsampleDecoder::new(inner, sel));
        Ok(slf.into())
    }
}
