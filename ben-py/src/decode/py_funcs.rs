use crate::common::{open_input, open_output, validate_input_output_paths};
use binary_ensemble::codec::decode::{
    decode_ben_to_jsonl as core_decode_ben_to_jsonl,
    decode_xben_to_ben as core_decode_xben_to_ben,
    decode_xben_to_jsonl as core_decode_xben_to_jsonl,
};
use pyo3::exceptions::PyIOError;
use pyo3::prelude::*;
use std::path::PathBuf;

#[pyfunction]
#[pyo3(signature = (in_file, out_file, overwrite=false))]
#[pyo3(text_signature = "(in_file, out_file, overwrite=False)")]
pub fn decode_xben_to_ben(
    in_file: PathBuf,
    out_file: PathBuf,
    overwrite: bool,
) -> PyResult<()> {
    validate_input_output_paths(&in_file, &out_file)?;
    let reader = open_input(&in_file)?;
    let writer = open_output(&out_file, overwrite)?;

    core_decode_xben_to_ben(reader, writer).map_err(|e| {
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
pub fn decode_xben_to_jsonl(
    in_file: PathBuf,
    out_file: PathBuf,
    overwrite: bool,
) -> PyResult<()> {
    validate_input_output_paths(&in_file, &out_file)?;
    let reader = open_input(&in_file)?;
    let writer = open_output(&out_file, overwrite)?;

    core_decode_xben_to_jsonl(reader, writer).map_err(|e| {
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
pub fn decode_ben_to_jsonl(
    in_file: PathBuf,
    out_file: PathBuf,
    overwrite: bool,
) -> PyResult<()> {
    validate_input_output_paths(&in_file, &out_file)?;
    let reader = open_input(&in_file)?;
    let writer = open_output(&out_file, overwrite)?;

    core_decode_ben_to_jsonl(reader, writer).map_err(|e| {
        PyIOError::new_err(format!(
            "Failed to convert BEN to JSONL from {} to {}: {e}",
            in_file.display(),
            out_file.display()
        ))
    })?;

    Ok(())
}
