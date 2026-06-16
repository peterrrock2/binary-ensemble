use crate::common::{open_input, parse_variant, validate_input_output_paths, TempOutput};
use binary_ensemble::codec::encode::{
    cpus_from_signed, encode_ben_to_xben as core_encode_ben_to_xben,
    encode_jsonl_to_ben as core_encode_jsonl_to_ben,
    encode_jsonl_to_xben as core_encode_jsonl_to_xben,
};
use pyo3::exceptions::PyIOError;
use pyo3::prelude::*;
use std::path::PathBuf;

/// Compress a BEN stream into an XBEN file with LZMA2.
///
/// XBEN is the smallest format and is meant for storage and transfer. Compression can be slow
/// for large block-level ensembles; relabel and reorder first (see
/// :func:`~binary_ensemble.bundle.relabel_bundle`) for the best ratios.
///
/// Args:
///     in_file (StrPath): Path to the input ``.ben`` file (``str`` or ``os.PathLike``).
///     out_file (StrPath): Path to write the ``.xben`` output (``str`` or ``os.PathLike``).
///     overwrite (bool, optional): Replace ``out_file`` if it already exists. Default is
///         ``False``.
///     n_threads (int | None, optional): Number of worker threads. Default is ``None``
///         which uses all available cores.
///     compression_level (int | None, optional): LZMA2 level from 0 (fastest) to 9
///         (smallest). Default is ``None`` which uses level 9.
///     xz_block_size (int | None, optional): Override the xz block size in bytes. Default
///         is ``None`` which uses the xz default.
///
/// Raises:
///     OSError: If ``out_file`` exists and ``overwrite`` is ``False``, or the conversion fails.
///     ValueError: If ``in_file`` and ``out_file`` are the same path.
#[pyfunction]
#[pyo3(signature = (in_file, out_file, overwrite=false, n_threads=None, compression_level=None, xz_block_size=None))]
#[pyo3(
    text_signature = "(in_file, out_file, overwrite=False, n_threads=None, compression_level=None, xz_block_size=None)"
)]
pub fn encode_ben_to_xben(
    py: Python<'_>,
    in_file: PathBuf,
    out_file: PathBuf,
    overwrite: bool,
    n_threads: Option<i32>,
    compression_level: Option<u32>,
    xz_block_size: Option<u64>,
) -> PyResult<()> {
    // Rust-only IO/CPU: run detached so other Python threads aren't blocked for the
    // conversion's duration.
    py.detach(move || {
        validate_input_output_paths(&in_file, &out_file)?;
        let reader = open_input(&in_file)?;
        let (guard, writer) = TempOutput::create(&out_file, overwrite)?;

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
        guard.commit()?;

        Ok(())
    })
}

/// Encode a canonicalized JSONL ensemble into a BEN stream.
///
/// Expects one ``{"assignment": [...], "sample": n}`` object per line. BEN is the fast working
/// format; encode further to XBEN with :func:`encode_ben_to_xben` for storage.
///
/// Args:
///     in_file (StrPath): Path to the input ``.jsonl`` file (``str`` or ``os.PathLike``).
///     out_file (StrPath): Path to write the ``.ben`` output (``str`` or ``os.PathLike``).
///     overwrite (bool, optional): Replace ``out_file`` if it already exists. Default is
///         ``False``.
///     variant (Variant, optional): BEN encoding variant: ``"standard"``, ``"mkv_chain"``,
///         or ``"twodelta"``. Default is ``"twodelta"``.
///
/// Raises:
///     OSError: If ``out_file`` exists and ``overwrite`` is ``False``, or the conversion fails.
///     ValueError: If ``variant`` is not a recognized variant name, or ``in_file`` and
///         ``out_file`` are the same path.
#[pyfunction]
#[pyo3(signature = (in_file, out_file, overwrite=false, variant="twodelta"))]
#[pyo3(text_signature = "(in_file, out_file, overwrite=False, variant='twodelta')")]
pub fn encode_jsonl_to_ben(
    py: Python<'_>,
    in_file: PathBuf,
    out_file: PathBuf,
    overwrite: bool,
    variant: &str,
) -> PyResult<()> {
    // Rust-only IO/CPU: run detached so other Python threads aren't blocked for the
    // conversion's duration.
    py.detach(move || {
        let ben_var = parse_variant(Some(variant))?;
        validate_input_output_paths(&in_file, &out_file)?;
        let reader = open_input(&in_file)?;
        let (guard, writer) = TempOutput::create(&out_file, overwrite)?;

        core_encode_jsonl_to_ben(reader, writer, ben_var).map_err(|e| {
            PyIOError::new_err(format!(
                "Failed to convert JSONL to BEN from {} to {}: {e}",
                in_file.display(),
                out_file.display()
            ))
        })?;
        guard.commit()?;

        Ok(())
    })
}

/// Encode a canonicalized JSONL ensemble directly into an XBEN file.
///
/// A one-step shortcut for :func:`encode_jsonl_to_ben` followed by
/// :func:`encode_ben_to_xben`. Expects one ``{"assignment": [...], "sample": n}`` object per
/// line. Compression can be slow for large block-level ensembles.
///
/// Args:
///     in_file (StrPath): Path to the input ``.jsonl`` file (``str`` or ``os.PathLike``).
///     out_file (StrPath): Path to write the ``.xben`` output (``str`` or ``os.PathLike``).
///     overwrite (bool, optional): Replace ``out_file`` if it already exists. Default is
///         ``False``.
///     variant (Variant, optional): BEN encoding variant: ``"standard"``, ``"mkv_chain"``,
///         or ``"twodelta"``. Default is ``"twodelta"``.
///     n_threads (int | None, optional): Number of worker threads. Default is ``None``
///         which uses all available cores.
///     compression_level (int | None, optional): LZMA2 level from 0 (fastest) to 9
///         (smallest). Default is ``None`` which uses level 9.
///     xz_block_size (int | None, optional): Override the xz block size in bytes. Default
///         is ``None`` which uses the xz default.
///
/// Raises:
///     OSError: If ``out_file`` exists and ``overwrite`` is ``False``, or the conversion fails.
///     ValueError: If ``variant`` is not a recognized variant name, or ``in_file`` and
///         ``out_file`` are the same path.
#[pyfunction]
#[pyo3(signature = (in_file, out_file, overwrite=false, variant="twodelta", n_threads=None, compression_level=None, xz_block_size=None))]
#[pyo3(
    text_signature = "(in_file, out_file, overwrite=False, variant='twodelta', n_threads=None, compression_level=None, xz_block_size=None)"
)]
#[allow(clippy::too_many_arguments)] // The py token pushed the Python-visible six over the lint.
pub fn encode_jsonl_to_xben(
    py: Python<'_>,
    in_file: PathBuf,
    out_file: PathBuf,
    overwrite: bool,
    variant: &str,
    n_threads: Option<i32>,
    compression_level: Option<u32>,
    xz_block_size: Option<u64>,
) -> PyResult<()> {
    // Rust-only IO/CPU: run detached so other Python threads aren't blocked for the
    // conversion's duration.
    py.detach(move || {
        let ben_var = parse_variant(Some(variant))?;
        validate_input_output_paths(&in_file, &out_file)?;
        let reader = open_input(&in_file)?;
        let (guard, writer) = TempOutput::create(&out_file, overwrite)?;

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
        guard.commit()?;

        Ok(())
    })
}
