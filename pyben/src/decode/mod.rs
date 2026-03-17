use crate::common::{open_input, open_output, validate_input_output_paths};
use binary_ensemble::codec::decode::{
    decode_ben_to_jsonl, decode_xben_to_ben, decode_xben_to_jsonl,
};
use binary_ensemble::io::reader::{
    build_frame_iter, count_samples_from_file, BenDecoder, MkvRecord, Selection,
    SubsampleFrameDecoder, XBenDecoder,
};
use pyo3::exceptions::{PyException, PyIOError, PyUserWarning};
use pyo3::prelude::*;
use pyo3::types::PyDict;
use std::io;
use std::path::PathBuf;

type DynIter = Box<dyn Iterator<Item = io::Result<MkvRecord>> + Send>;

#[derive(Clone)]
enum DecoderMode {
    Ben,
    XBen,
}

impl DecoderMode {
    fn parse(mode: &str) -> PyResult<Self> {
        match mode {
            "ben" => Ok(Self::Ben),
            "xben" => Ok(Self::XBen),
            _ => Err(PyException::new_err(
                "Unknown mode. Supported modes are 'ben' and 'xben'.",
            )),
        }
    }

    fn as_str(&self) -> &'static str {
        match self {
            Self::Ben => "ben",
            Self::XBen => "xben",
        }
    }
}

#[derive(Clone)]
struct DecoderSource {
    path: PathBuf,
    mode: DecoderMode,
}

#[pyclass(module = "binary_ensemble", unsendable)]
pub struct PyBenDecoder {
    source: DecoderSource,
    iter: DynIter,
    current_assignment: Option<Vec<u16>>,
    remaining_count: u16,
    base_len: Option<usize>,
    len_hint: Option<usize>,
}

#[pymethods]
impl PyBenDecoder {
    #[new]
    #[pyo3(signature = (file_path, mode = "ben"))]
    #[pyo3(text_signature = "(file_path, mode='ben')")]
    fn new(py: Python<'_>, file_path: PathBuf, mode: &str) -> PyResult<Self> {
        let mode = DecoderMode::parse(mode)?;
        let source = DecoderSource {
            path: file_path,
            mode,
        };
        let iter = build_iter(py, &source)?;

        Ok(Self {
            source,
            iter,
            current_assignment: None,
            remaining_count: 0,
            base_len: None,
            len_hint: None,
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
                if count == 0 {
                    return Err(PyException::new_err(
                        "Decoder yielded a zero-count record; data may be corrupted.",
                    ));
                }
                slf.current_assignment = Some(assignment.clone());
                slf.remaining_count = count - 1;
                Ok(Some(assignment))
            }
            Some(Err(e)) => Err(PyException::new_err(format!(
                "Error decoding next item: {e}"
            ))),
            None => Ok(None),
        }
    }

    // Because we want progress bars!!!
    fn __len__(mut slf: PyRefMut<Self>, py: Python<'_>) -> PyResult<usize> {
        if let Some(len_hint) = slf.len_hint {
            return Ok(len_hint);
        }

        let base_len = ensure_base_len(&mut slf, py)?;
        slf.len_hint = Some(base_len);
        Ok(base_len)
    }

    #[pyo3(text_signature = "(self)")]
    fn count_samples(mut slf: PyRefMut<Self>, py: Python<'_>) -> PyResult<usize> {
        let base_len = ensure_base_len(&mut slf, py)?;
        slf.len_hint = Some(base_len);
        Ok(base_len)
    }

    #[pyo3(text_signature = "(self, indices, /)")]
    fn subsample_indices<'py>(
        mut slf: PyRefMut<'py, Self>,
        mut indices: Vec<usize>,
        py: Python<'_>,
    ) -> PyResult<Py<Self>> {
        if !indices.iter().is_sorted() {
            // We need to sort and deduplicate the indices
            // This is a bit annoying, but it is necessary to ensure that we can
            // efficiently iterate over the underlying data.
            // We use unstable sort because we don't care about the order of equal elements
            // and it is faster than stable sort.
            let warnings = py.import("warnings")?;
            let kwargs = PyDict::new(py);
            // kwargs.set_item("stacklevel", 2)?;

            warnings.call_method(
                "warn",
                (
                    "Indices must be sorted and unique; sorting and deduplicating.",
                    py.get_type::<PyUserWarning>(),
                ),
                Some(&kwargs),
            )?;
        }
        indices.sort_unstable();
        indices.dedup();

        if indices.is_empty() {
            return Err(PyException::new_err("indices must not be empty"));
        }
        let base_len = ensure_base_len(&mut slf, py)?;
        if indices[0] <= 0 {
            return Err(PyException::new_err("indices must be 1-based"));
        }
        if indices.last().unwrap() > &base_len {
            return Err(PyException::new_err(format!(
                "indices must be <= number of samples in base data ({})",
                base_len
            )));
        }
        let len_hint = indices.len();

        let sel = Selection::Indices(indices.into_iter().peekable());
        reset_with_selection(&mut slf, sel, len_hint)?;
        Ok(slf.into())
    }

    #[pyo3(text_signature = "(self, start, end, /)")]
    fn subsample_range<'py>(
        mut slf: PyRefMut<'py, Self>,
        start: usize,
        end: usize,
        py: Python<'_>,
    ) -> PyResult<Py<Self>> {
        if start == 0 || end < start {
            return Err(PyException::new_err(
                "range must be 1-based and end >= start",
            ));
        }
        let base_len = ensure_base_len(&mut slf, py)?;
        if end > base_len {
            return Err(PyException::new_err(format!(
                "end must be <= number of samples in base data ({})",
                base_len
            )));
        }

        let sel = Selection::Range { start, end };
        let len_hint = end - start + 1;
        reset_with_selection(&mut slf, sel, len_hint)?;
        Ok(slf.into())
    }

    #[pyo3(signature = (step, offset=1))]
    fn subsample_every<'py>(
        mut slf: PyRefMut<'py, Self>,
        step: usize,
        offset: usize,
        py: Python<'_>,
    ) -> PyResult<Py<Self>> {
        if step == 0 || offset == 0 {
            return Err(PyException::new_err("step and offset must be >= 1"));
        }
        let base_len = ensure_base_len(&mut slf, py)?;
        if offset > base_len {
            return Err(PyException::new_err(format!(
                "offset must be <= number of samples in base data ({})",
                base_len
            )));
        }
        let sel = Selection::Every { step, offset };
        let len_hint = (base_len + step - 1 - (offset - 1)) / step;
        reset_with_selection(&mut slf, sel, len_hint)?;
        Ok(slf.into())
    }
}

fn warn_xben_startup(py: Python<'_>) -> PyResult<()> {
    let warnings = py.import("warnings")?;
    let kwargs = PyDict::new(py);

    warnings.call_method(
        "warn",
        (
            "XBEN may take a second to start decoding.",
            py.get_type::<PyUserWarning>(),
        ),
        Some(&kwargs),
    )?;

    Ok(())
}

fn build_iter(py: Python<'_>, source: &DecoderSource) -> PyResult<DynIter> {
    let reader = open_input(&source.path)?;
    match source.mode {
        DecoderMode::Ben => {
            let ben = BenDecoder::new(reader)
                .map_err(|e| PyException::new_err(format!("Failed to create BenDecoder: {e}")))?;
            Ok(Box::new(ben))
        }
        DecoderMode::XBen => {
            warn_xben_startup(py)?;
            let xben = XBenDecoder::new(reader)
                .map_err(|e| PyException::new_err(format!("Failed to create XBenDecoder: {e}")))?;
            Ok(Box::new(xben))
        }
    }
}

fn build_frames(source: &DecoderSource) -> PyResult<binary_ensemble::io::reader::FrameIter> {
    build_frame_iter(&source.path, source.mode.as_str()).map_err(|e| {
        PyException::new_err(format!(
            "Failed to create frame iterator from {}: {e}",
            source.path.display()
        ))
    })
}

fn reset_with_selection(
    decoder: &mut PyBenDecoder,
    selection: Selection,
    len_hint: usize,
) -> PyResult<()> {
    let frames = build_frames(&decoder.source)?;
    let frame_decoder = SubsampleFrameDecoder::new(frames, selection);
    decoder.iter = Box::new(frame_decoder);
    decoder.current_assignment = None;
    decoder.remaining_count = 0;
    decoder.len_hint = Some(len_hint);
    Ok(())
}

fn ensure_base_len(decoder: &mut PyBenDecoder, py: Python<'_>) -> PyResult<usize> {
    if let Some(base_len) = decoder.base_len {
        return Ok(base_len);
    }

    let path = decoder.source.path.clone();
    let mode = decoder.source.mode.as_str().to_string();
    let base_len = py
        .detach(|| count_samples_from_file(&path, &mode))
        .map_err(|e| {
            PyException::new_err(format!(
                "Failed to count samples in {}: {e}",
                path.display()
            ))
        })?;
    decoder.base_len = Some(base_len);
    Ok(base_len)
}

#[pyfunction]
#[pyo3(signature = (in_file, out_file, overwrite=false))]
#[pyo3(text_signature = "(in_file, out_file, overwrite=False)")]
pub fn decompress_xben_to_ben(
    in_file: PathBuf,
    out_file: PathBuf,
    overwrite: bool,
) -> PyResult<()> {
    validate_input_output_paths(&in_file, &out_file)?;
    let reader = open_input(&in_file)?;
    let writer = open_output(&out_file, overwrite)?;

    decode_xben_to_ben(reader, writer).map_err(|e| {
        PyIOError::new_err(format!(
            "Failed to convert XBEN to BEN from {} to {}: {e}",
            in_file.display(),
            out_file.display()
        ))
    })?;

    Ok(())
}

#[pyfunction]
#[pyo3(signature = (in_file, out_file, overwrite=false))]
#[pyo3(text_signature = "(in_file, out_file, overwrite=False)")]
pub fn decompress_xben_to_jsonl(
    in_file: PathBuf,
    out_file: PathBuf,
    overwrite: bool,
) -> PyResult<()> {
    validate_input_output_paths(&in_file, &out_file)?;
    let reader = open_input(&in_file)?;
    let writer = open_output(&out_file, overwrite)?;

    decode_xben_to_jsonl(reader, writer).map_err(|e| {
        PyIOError::new_err(format!(
            "Failed to convert XBEN to JSONL from {} to {}: {e}",
            in_file.display(),
            out_file.display()
        ))
    })?;

    Ok(())
}

#[pyfunction]
#[pyo3(signature = (in_file, out_file, overwrite=false))]
#[pyo3(text_signature = "(in_file, out_file, overwrite=False)")]
pub fn decompress_ben_to_jsonl(
    in_file: PathBuf,
    out_file: PathBuf,
    overwrite: bool,
) -> PyResult<()> {
    validate_input_output_paths(&in_file, &out_file)?;
    let reader = open_input(&in_file)?;
    let writer = open_output(&out_file, overwrite)?;

    decode_ben_to_jsonl(reader, writer).map_err(|e| {
        PyIOError::new_err(format!(
            "Failed to convert BEN to JSONL from {} to {}: {e}",
            in_file.display(),
            out_file.display()
        ))
    })?;

    Ok(())
}
