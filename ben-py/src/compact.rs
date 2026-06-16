//! Binding for compacting a `.bendl` file: rewriting it without unreferenced byte ranges.
//!
//! Thin wrapper over [`binary_ensemble::io::bundle::compact`], which owns the semantics: assets
//! are carried over verbatim (stored bytes, flags, and checksums unchanged, verified against
//! their stored CRC32C as they travel), the assignment stream is copied through a verified
//! reader, and the wire format (BEN or XBEN) is preserved. The `bendl` CLI's `remove`
//! and `compact` subcommands share the same core implementation.

use crate::common::{map_bundle_err, TempOutput};
use binary_ensemble::io::bundle::compact::{
    compact_bundle as core_compact_bundle, compact_bundle_in_place as core_compact_in_place,
    Compaction,
};
use binary_ensemble::io::bundle::BendlReader;
use pyo3::exceptions::{PyException, PyIOError};
use pyo3::prelude::*;
use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;

/// Rewrite the bundle at `in_file` without unreferenced byte ranges, writing the result to
/// `out_file`.
///
/// Raw surface for a bundle carrying dead space: one that arrives from other tooling, or one
/// grown by appends (each immediate-commit add supersedes the previous directory, leaving a
/// few dead bytes). The facade transforms (``remove_asset``, ``compress_stream``,
/// ``relabel_bundle``) emit compact bundles themselves. See also
/// :func:`compact_bundle_in_place`.
///
/// Args:
///     in_file (StrPath): Path to the source ``.bendl`` bundle (``str`` or ``os.PathLike``).
///     out_file (StrPath): Destination path for the compacted bundle (``str`` or
///         ``os.PathLike``).
///     overwrite (bool, optional): Replace ``out_file`` if it already exists. Default is
///         ``False``.
///
/// Raises:
///     OSError: If ``out_file`` exists and ``overwrite`` is ``False``.
///     Exception: If the bundle is unfinalized, or an asset or the stream fails its checksum.
#[pyfunction]
#[pyo3(signature = (in_file, out_file, overwrite = false))]
#[pyo3(text_signature = "(in_file, out_file, overwrite=False)")]
pub fn compact_bundle(
    py: Python<'_>,
    in_file: PathBuf,
    out_file: PathBuf,
    overwrite: bool,
) -> PyResult<()> {
    // Rust-only IO/CPU: run detached so other Python threads (and KeyboardInterrupt delivery)
    // aren't blocked for the rewrite's potentially multi-gigabyte duration.
    py.detach(move || compact_bundle_impl(in_file, out_file, overwrite))
}

fn compact_bundle_impl(in_file: PathBuf, out_file: PathBuf, overwrite: bool) -> PyResult<()> {
    let file = File::open(&in_file)
        .map_err(|e| PyIOError::new_err(format!("Failed to open {}: {e}", in_file.display())))?;
    let mut reader = BendlReader::open(BufReader::new(file)).map_err(|e| {
        PyException::new_err(format!(
            "Failed to parse bundle header in {}: {e}",
            in_file.display()
        ))
    })?;
    // Check before open_output so a doomed call cannot create or truncate the destination.
    if !reader.is_finalized() {
        return Err(PyException::new_err(
            "compact_bundle requires a finalized bundle",
        ));
    }

    let (guard, buf) = TempOutput::create(&out_file, overwrite)?;
    let out = core_compact_bundle(&mut reader, buf).map_err(map_bundle_err)?;
    guard.commit_writer(out)?;
    Ok(())
}

/// Compact the bundle at `path` in place, choosing the cheapest applicable strategy.
///
/// When every unreferenced byte lies after the assignment stream (the layout that asset
/// removals and appends produce), only the small post-stream tail is rebuilt: O(tail),
/// independent of stream size, no scratch space, stream never read. Otherwise the bundle is
/// rewritten wholesale through a temp file (stream checksum-verified during the copy) and
/// atomically swapped over `path`.
///
/// Raw surface, also used by :meth:`binary_ensemble.bundle.BendlEncoder.remove_asset`. The
/// facade transforms emit compact bundles themselves, so calling this directly is only needed
/// for a bundle that arrived with dead space from other tooling or accumulated superseded
/// directories from appends.
///
/// Args:
///     path (StrPath): Path to the ``.bendl`` bundle to compact (``str`` or ``os.PathLike``).
///
/// Returns:
///     str: Which strategy ran: ``"none"`` (already compact), ``"tail"`` (post-stream tail
///     rebuilt; stream untouched and not verified), or ``"full"`` (whole-bundle rewrite).
///
/// Raises:
///     Exception: If the bundle is unfinalized, or (on the full-rewrite path) an asset or
///         the stream fails its checksum.
#[pyfunction]
#[pyo3(signature = (path))]
#[pyo3(text_signature = "(path)")]
pub fn compact_bundle_in_place(py: Python<'_>, path: PathBuf) -> PyResult<&'static str> {
    // Rust-only IO/CPU: run detached so other Python threads (and KeyboardInterrupt delivery)
    // aren't blocked for the rewrite's potentially multi-gigabyte duration.
    py.detach(move || compact_bundle_in_place_impl(path))
}

fn compact_bundle_in_place_impl(path: PathBuf) -> PyResult<&'static str> {
    let kind = core_compact_in_place(&path).map_err(map_bundle_err)?;
    Ok(match kind {
        Compaction::None => "none",
        Compaction::TailRewrite => "tail",
        Compaction::FullRewrite => "full",
    })
}
