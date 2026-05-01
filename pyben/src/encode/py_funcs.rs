use crate::common::{open_input, open_output, parse_variant, validate_input_output_paths};
use binary_ensemble::codec::encode::{encode_ben_to_xben, encode_jsonl_to_ben, encode_jsonl_to_xben};
use pyo3::exceptions::PyIOError;
use pyo3::prelude::*;
use std::path::PathBuf;

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

    encode_jsonl_to_xben(reader, writer, ben_var, n_threads, compression_level, None).map_err(
        |e| {
            PyIOError::new_err(format!(
                "Failed to convert JSONL to XBEN from {} to {}: {e}",
                in_file.display(),
                out_file.display()
            ))
        },
    )?;
    Ok(())
}
