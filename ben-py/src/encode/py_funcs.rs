use crate::common::{open_input, open_output, parse_variant, validate_input_output_paths};
use binary_ensemble::codec::encode::{
    cpus_from_signed, encode_ben_to_xben as core_encode_ben_to_xben,
    encode_jsonl_to_ben as core_encode_jsonl_to_ben,
    encode_jsonl_to_xben as core_encode_jsonl_to_xben,
};
use pyo3::exceptions::PyIOError;
use pyo3::prelude::*;
use std::path::PathBuf;

#[pyfunction]
#[pyo3(signature = (in_file, out_file, overwrite=false, n_threads=None, compression_level=None, xz_block_size=None))]
#[pyo3(
    text_signature = "(in_file, out_file, overwrite=False, n_threads=None, compression_level=None, xz_block_size=None)"
)]
pub fn encode_ben_to_xben(
    in_file: PathBuf,
    out_file: PathBuf,
    overwrite: bool,
    n_threads: Option<i32>,
    compression_level: Option<u32>,
    xz_block_size: Option<u64>,
) -> PyResult<()> {
    validate_input_output_paths(&in_file, &out_file)?;
    let reader = open_input(&in_file)?;
    let writer = open_output(&out_file, overwrite)?;

    core_encode_ben_to_xben(
        reader,
        writer,
        n_threads.map(cpus_from_signed),
        compression_level,
        None,
        xz_block_size,
    )
    .map_err(|e| {
        PyIOError::new_err(format!(
            "Failed to convert BEN to XBEN from {} to {}: {e}",
            in_file.display(),
            out_file.display()
        ))
    })?;

    Ok(())
}

#[pyfunction]
#[pyo3(signature = (in_file, out_file, overwrite=false, variant="twodelta"))]
#[pyo3(text_signature = "(in_file, out_file, overwrite=False, variant='twodelta')")]
pub fn encode_jsonl_to_ben(
    in_file: PathBuf,
    out_file: PathBuf,
    overwrite: bool,
    variant: &str,
) -> PyResult<()> {
    let ben_var = parse_variant(Some(variant))?;
    validate_input_output_paths(&in_file, &out_file)?;
    let reader = open_input(&in_file)?;
    let writer = open_output(&out_file, overwrite)?;

    core_encode_jsonl_to_ben(reader, writer, ben_var).map_err(|e| {
        PyIOError::new_err(format!(
            "Failed to convert JSONL to BEN from {} to {}: {e}",
            in_file.display(),
            out_file.display()
        ))
    })?;
    Ok(())
}

#[pyfunction]
#[pyo3(signature = (in_file, out_file, overwrite=false, variant="twodelta", n_threads=None, compression_level=None, xz_block_size=None))]
#[pyo3(
    text_signature = "(in_file, out_file, overwrite=False, variant='twodelta', n_threads=None, compression_level=None, xz_block_size=None)"
)]
pub fn encode_jsonl_to_xben(
    in_file: PathBuf,
    out_file: PathBuf,
    overwrite: bool,
    variant: &str,
    n_threads: Option<i32>,
    compression_level: Option<u32>,
    xz_block_size: Option<u64>,
) -> PyResult<()> {
    let ben_var = parse_variant(Some(variant))?;
    validate_input_output_paths(&in_file, &out_file)?;
    let reader = open_input(&in_file)?;
    let writer = open_output(&out_file, overwrite)?;

    core_encode_jsonl_to_xben(
        reader,
        writer,
        ben_var,
        n_threads.map(cpus_from_signed),
        compression_level,
        None,
        xz_block_size,
    )
    .map_err(|e| {
        PyIOError::new_err(format!(
            "Failed to convert JSONL to XBEN from {} to {}: {e}",
            in_file.display(),
            out_file.display()
        ))
    })?;
    Ok(())
}
