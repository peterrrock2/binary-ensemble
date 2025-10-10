use ben::encode::{encode_ben_to_xben, encode_jsonl_to_ben, encode_jsonl_to_xben, BenEncoder};
use ben::BenVariant;
use pyo3::exceptions::{PyException, PyIOError};
use pyo3::prelude::PyResult;
use pyo3::{pyclass, pyfunction, pymethods};
use std::fs::File;
use std::io::{BufReader, BufWriter};
use std::path::{Path, PathBuf};

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
    // Basic validations
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

    encode_ben_to_xben(reader, writer, n_threads, compression_level).map_err(|e| {
        PyIOError::new_err(format!(
            "Failed to convert BEN to XBEN from {} to {}: {e}",
            in_file.display(),
            out_file.display()
        ))
    })?;

    Ok(())
}

#[pyfunction]
#[pyo3(signature = (in_file, out_file, overwrite=false, variant="markov"))]
#[pyo3(text_signature = "(in_file, out_file, overwrite=false, variant='markov')")]
pub fn compress_jsonl_to_ben(
    in_file: PathBuf,
    out_file: PathBuf,
    overwrite: bool,
    variant: &str,
) -> PyResult<()> {
    let ben_var = match variant {
        "standard" => BenVariant::Standard,
        "mkv_chain" | "markov" => BenVariant::MkvChain,
        other => {
            eprintln!(
                "Warning: Unknown variant '{}', defaulting to 'markov'",
                other
            );
            BenVariant::MkvChain
        }
    };

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
#[pyo3(signature = (in_file, out_file, overwrite=false, variant="markov", n_threads=None, compression_level=None))]
#[pyo3(
    text_signature = "(in_file, out_file, overwrite=false, variant='markov', n_threads=None, compression_level=None)"
)]
pub fn compress_jsonl_to_xben(
    in_file: PathBuf,
    out_file: PathBuf,
    overwrite: bool,
    variant: &str,
    n_threads: Option<u32>,
    compression_level: Option<u32>,
) -> PyResult<()> {
    let ben_var = match variant {
        "standard" => BenVariant::Standard,
        "mkv_chain" | "markov" => BenVariant::MkvChain,
        other => {
            eprintln!(
                "Warning: Unknown variant '{}', defaulting to 'markov'",
                other
            );
            BenVariant::MkvChain
        }
    };

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
    encode_jsonl_to_xben(reader, writer, ben_var, n_threads, compression_level).map_err(|e| {
        PyIOError::new_err(format!(
            "Failed to convert JSONL to BEN from {} to {}: {e}",
            in_file.display(),
            out_file.display()
        ))
    })?;
    Ok(())
}
