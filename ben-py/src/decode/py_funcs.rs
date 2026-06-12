use crate::common::{open_input, open_output, validate_input_output_paths};
use binary_ensemble::codec::decode::{
    decode_ben_to_jsonl as core_decode_ben_to_jsonl, decode_xben_to_ben as core_decode_xben_to_ben,
    decode_xben_to_jsonl as core_decode_xben_to_jsonl,
};
use pyo3::exceptions::PyIOError;
use pyo3::prelude::*;
use std::path::PathBuf;

/// Decompress an XBEN file into a plain BEN stream.
///
/// XBEN decompression is fast; converting to BEN gives you a stream you can read, replay, and
/// subsample. The encoding variant is preserved and detected automatically on the next read.
///
/// Args:
///     in_file (StrPath): Path to the input ``.xben`` file (``str`` or ``os.PathLike``).
///     out_file (StrPath): Path to write the ``.ben`` output (``str`` or ``os.PathLike``).
///     overwrite (bool, optional): Replace ``out_file`` if it already exists. Default is ``False``.
///
/// Raises:
///     OSError: If ``out_file`` exists and ``overwrite`` is ``False``, or the conversion fails.
#[pyfunction]
#[pyo3(signature = (in_file, out_file, overwrite=false))]
#[pyo3(text_signature = "(in_file, out_file, overwrite=False)")]
pub fn decode_xben_to_ben(in_file: PathBuf, out_file: PathBuf, overwrite: bool) -> PyResult<()> {
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

/// Decode an XBEN file back to canonicalized JSONL.
///
/// Produces one ``{"assignment": [...], "sample": n}`` object per line, with sample numbers
/// starting at 1.
///
/// Args:
///     in_file (StrPath): Path to the input ``.xben`` file (``str`` or ``os.PathLike``).
///     out_file (StrPath): Path to write the ``.jsonl`` output (``str`` or ``os.PathLike``).
///     overwrite (bool, optional): Replace ``out_file`` if it already exists. Default is ``False``.
///
/// Raises:
///     OSError: If ``out_file`` exists and ``overwrite`` is ``False``, or the conversion fails.
#[pyfunction]
#[pyo3(signature = (in_file, out_file, overwrite=false))]
#[pyo3(text_signature = "(in_file, out_file, overwrite=False)")]
pub fn decode_xben_to_jsonl(in_file: PathBuf, out_file: PathBuf, overwrite: bool) -> PyResult<()> {
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

/// Decode a BEN stream back to canonicalized JSONL.
///
/// Produces one ``{"assignment": [...], "sample": n}`` object per line, with sample numbers
/// starting at 1. This is the inverse of :func:`encode_jsonl_to_ben`.
///
/// Args:
///     in_file (StrPath): Path to the input ``.ben`` file (``str`` or ``os.PathLike``).
///     out_file (StrPath): Path to write the ``.jsonl`` output (``str`` or ``os.PathLike``).
///     overwrite (bool, optional): Replace ``out_file`` if it already exists. Default is ``False``.
///
/// Raises:
///     OSError: If ``out_file`` exists and ``overwrite`` is ``False``, or the conversion fails.
#[pyfunction]
#[pyo3(signature = (in_file, out_file, overwrite=false))]
#[pyo3(text_signature = "(in_file, out_file, overwrite=False)")]
pub fn decode_ben_to_jsonl(in_file: PathBuf, out_file: PathBuf, overwrite: bool) -> PyResult<()> {
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
