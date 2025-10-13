use ben::decode::{
    build_frame_iter, decode_ben_to_jsonl, decode_xben_to_ben, decode_xben_to_jsonl, BenDecoder,
    MkvRecord, Selection, SubsampleFrameDecoder, XBenDecoder,
};
use pyo3::exceptions::{PyException, PyIOError};
use pyo3::prelude::*;
use std::fs::File;
use std::io::{self, BufReader, BufWriter};
use std::path::PathBuf;

pub mod read;

type DynIter = Box<dyn Iterator<Item = io::Result<MkvRecord>> + Send>;

#[pyclass(module = "pyben", unsendable)]
pub struct PyBenDecoder {
    iter: DynIter,
    current_assignment: Option<Vec<u16>>,
    remaining_count: u16,
    src_path: PathBuf,
    mode: String,
}

#[pymethods]
impl PyBenDecoder {
    #[new]
    #[pyo3(signature = (file_path, mode = "ben"))]
    #[pyo3(text_signature = "(file_path, mode='ben')")]
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
            current_assignment: None,
            remaining_count: 0,
            src_path: file_path,
            mode: mode.to_string(),
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
            None => Ok(None),
        }
    }

    #[pyo3(text_signature = "(self, indices, /)")]
    fn subsample_indices<'py>(
        mut slf: PyRefMut<'py, Self>,
        mut indices: Vec<usize>,
    ) -> PyResult<Py<Self>> {
        indices.sort_unstable();
        indices.dedup();
        let sel = Selection::Indices(indices.into_iter().peekable());

        let frames = build_frame_iter(&slf.src_path, &slf.mode).map_err(|e| {
            PyException::new_err(format!(
                "Failed to create frame iterator from {}: {e}",
                slf.src_path.display()
            ))
        })?;

        let frame_decoder = SubsampleFrameDecoder::new(frames, sel);

        slf.iter = Box::new(frame_decoder);
        slf.current_assignment = None;
        slf.remaining_count = 0;
        Ok(slf.into())
    }

    #[pyo3(text_signature = "(self, start, end, /)")]
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

        let frames = build_frame_iter(&slf.src_path, &slf.mode).map_err(|e| {
            PyException::new_err(format!(
                "Failed to create frame iterator from {}: {e}",
                slf.src_path.display()
            ))
        })?;

        let frame_decoder = SubsampleFrameDecoder::new(frames, sel);

        slf.iter = Box::new(frame_decoder);
        slf.current_assignment = None;
        slf.remaining_count = 0;
        Ok(slf.into())
    }

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

        let frames = build_frame_iter(&slf.src_path, &slf.mode).map_err(|e| {
            PyException::new_err(format!(
                "Failed to create frame iterator from {}: {e}",
                slf.src_path.display()
            ))
        })?;

        let frame_decoder = SubsampleFrameDecoder::new(frames, sel);

        slf.iter = Box::new(frame_decoder);
        slf.current_assignment = None;
        slf.remaining_count = 0;
        Ok(slf.into())
    }
}

#[pyfunction]
#[pyo3(signature = (in_file, out_file, overwrite=false))]
#[pyo3(text_signature = "(in_file, out_file, overwrite=False)")]
pub fn decompress_xben_to_ben(
    in_file: PathBuf,
    out_file: PathBuf,
    overwrite: bool,
) -> PyResult<()> {
    if in_file == out_file {
        return Err(PyIOError::new_err("Input and output paths must differ."));
    }
    if !in_file.exists() {
        return Err(PyIOError::new_err(format!(
            "Input file {} does not exist.",
            in_file.display()
        )));
    }
    if out_file.exists() && !overwrite {
        return Err(PyIOError::new_err(format!(
            "Output file {} already exists (use overwrite=True to replace).",
            out_file.display()
        )));
    }
    // Open input (read-only, buffered)
    let infile = File::open(&in_file)
        .map_err(|e| PyIOError::new_err(format!("Failed to open {}: {e}", in_file.display())))?;
    let reader = BufReader::new(infile);

    // Open/create output according to overwrite flag
    let out_open = if overwrite {
        File::options()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&out_file)
    } else {
        File::options().write(true).create_new(true).open(&out_file)
    };
    let outfile = out_open
        .map_err(|e| PyIOError::new_err(format!("Failed to create {}: {e}", out_file.display())))?;
    let writer = BufWriter::new(outfile);

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
    if in_file == out_file {
        return Err(PyIOError::new_err("Input and output paths must differ."));
    }
    if !in_file.exists() {
        return Err(PyIOError::new_err(format!(
            "Input file {} does not exist.",
            in_file.display()
        )));
    }
    if out_file.exists() && !overwrite {
        return Err(PyIOError::new_err(format!(
            "Output file {} already exists (use overwrite=True to replace).",
            out_file.display()
        )));
    }
    // Open input (read-only, buffered)
    let infile = File::open(&in_file)
        .map_err(|e| PyIOError::new_err(format!("Failed to open {}: {e}", in_file.display())))?;
    let reader = BufReader::new(infile);

    // Open/create output according to overwrite flag
    let out_open = if overwrite {
        File::options()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&out_file)
    } else {
        File::options().write(true).create_new(true).open(&out_file)
    };
    let outfile = out_open
        .map_err(|e| PyIOError::new_err(format!("Failed to create {}: {e}", out_file.display())))?;
    let writer = BufWriter::new(outfile);

    decode_xben_to_jsonl(reader, writer).map_err(|e| {
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
pub fn decompress_ben_to_jsonl(
    in_file: PathBuf,
    out_file: PathBuf,
    overwrite: bool,
) -> PyResult<()> {
    if in_file == out_file {
        return Err(PyIOError::new_err("Input and output paths must differ."));
    }
    if !in_file.exists() {
        return Err(PyIOError::new_err(format!(
            "Input file {} does not exist.",
            in_file.display()
        )));
    }
    if out_file.exists() && !overwrite {
        return Err(PyIOError::new_err(format!(
            "Output file {} already exists (use overwrite=True to replace).",
            out_file.display()
        )));
    }
    // Open input (read-only, buffered)
    let infile = File::open(&in_file)
        .map_err(|e| PyIOError::new_err(format!("Failed to open {}: {e}", in_file.display())))?;
    let reader = BufReader::new(infile);

    // Open/create output according to overwrite flag
    let out_open = if overwrite {
        File::options()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&out_file)
    } else {
        File::options().write(true).create_new(true).open(&out_file)
    };
    let outfile = out_open
        .map_err(|e| PyIOError::new_err(format!("Failed to create {}: {e}", out_file.display())))?;
    let writer = BufWriter::new(outfile);

    decode_ben_to_jsonl(reader, writer).map_err(|e| {
        PyIOError::new_err(format!(
            "Failed to convert XBEN to BEN from {} to {}: {e}",
            in_file.display(),
            out_file.display()
        ))
    })?;

    Ok(())
}
