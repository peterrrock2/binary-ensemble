//! Bindings for transcoding a `.bendl` file's embedded stream between BEN and XBEN.
//!
//! These operations repackage a bundle with the opposite assignment format and the same decoded
//! assets (name, type, JSON flag, and payload). Asset storage compression is normalized to the
//! writer's default policy rather than preserved byte-for-byte.

use crate::common::{add_preserved, map_bundle_err, PreservedAsset, TempOutput};
use binary_ensemble::codec::decode::decode_xben_to_ben;
use binary_ensemble::codec::encode::encode_ben_to_xben;
use binary_ensemble::io::bundle::format::{AssignmentFormat, ASSET_FLAG_JSON};
use binary_ensemble::io::bundle::{BendlReader, BendlWriter};
use pyo3::exceptions::{PyException, PyIOError};
use pyo3::prelude::*;
use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;

/// Recompress the BEN stream of the bundle at `in_file` to XBEN, writing a new bundle at
/// `out_file`.
///
/// This is the raw core binding; prefer the :func:`binary_ensemble.bundle.compress_stream`
/// facade, which recompresses in place when ``out_file`` is omitted.
///
/// Args:
///     in_file (StrPath): Path to the source ``.bendl`` bundle (``str`` or ``os.PathLike``).
///     out_file (StrPath): Destination path for the recompressed bundle (``str`` or
///         ``os.PathLike``).
///     overwrite (bool, optional): Replace ``out_file`` if it already exists. Default is
///         ``False``.
///
/// Raises:
///     OSError: If ``out_file`` exists and ``overwrite`` is ``False``.
///     Exception: If the bundle is unfinalized or already holds an XBEN stream.
#[pyfunction]
#[pyo3(signature = (in_file, out_file, overwrite = false))]
#[pyo3(text_signature = "(in_file, out_file, overwrite=False)")]
pub fn recompress_bundle(
    py: Python<'_>,
    in_file: PathBuf,
    out_file: PathBuf,
    overwrite: bool,
) -> PyResult<()> {
    // Rust-only IO/CPU (whole-stream xz encode): run detached so other Python threads aren't
    // blocked for its duration.
    py.detach(move || recompress_bundle_impl(in_file, out_file, overwrite))
}

fn recompress_bundle_impl(in_file: PathBuf, out_file: PathBuf, overwrite: bool) -> PyResult<()> {
    transcode_bundle_impl(in_file, out_file, overwrite, AssignmentFormat::Xben)
}

/// Decompress the XBEN stream of the bundle at `in_file` to BEN, writing a new bundle at
/// `out_file`.
///
/// This is the raw core binding; prefer the :func:`binary_ensemble.bundle.decompress_stream`
/// facade, which decompresses in place when ``out_file`` is omitted.
///
/// Args:
///     in_file (StrPath): Path to the source ``.bendl`` bundle (``str`` or ``os.PathLike``).
///     out_file (StrPath): Destination path for the decompressed bundle (``str`` or
///         ``os.PathLike``).
///     overwrite (bool, optional): Replace ``out_file`` if it already exists. Default is
///         ``False``.
///
/// Raises:
///     OSError: If ``out_file`` exists and ``overwrite`` is ``False``.
///     Exception: If the bundle is unfinalized or does not hold an XBEN stream.
#[pyfunction]
#[pyo3(signature = (in_file, out_file, overwrite = false))]
#[pyo3(text_signature = "(in_file, out_file, overwrite=False)")]
pub fn decompress_bundle(
    py: Python<'_>,
    in_file: PathBuf,
    out_file: PathBuf,
    overwrite: bool,
) -> PyResult<()> {
    // Rust-only IO/CPU (whole-stream xz decode): run detached so other Python threads aren't
    // blocked for its duration.
    py.detach(move || decompress_bundle_impl(in_file, out_file, overwrite))
}

fn decompress_bundle_impl(in_file: PathBuf, out_file: PathBuf, overwrite: bool) -> PyResult<()> {
    transcode_bundle_impl(in_file, out_file, overwrite, AssignmentFormat::Ben)
}

fn transcode_bundle_impl(
    in_file: PathBuf,
    out_file: PathBuf,
    overwrite: bool,
    target_format: AssignmentFormat,
) -> PyResult<()> {
    let operation = match target_format {
        AssignmentFormat::Ben => "decompress_stream",
        AssignmentFormat::Xben => "compress_stream",
    };
    let file = File::open(&in_file)
        .map_err(|e| PyIOError::new_err(format!("Failed to open {}: {e}", in_file.display())))?;
    let mut reader = BendlReader::open(BufReader::new(file)).map_err(|e| {
        PyException::new_err(format!(
            "Failed to parse bundle header in {}: {e}",
            in_file.display()
        ))
    })?;

    if !reader.is_finalized() {
        return Err(PyException::new_err(format!(
            "{operation} requires a finalized bundle"
        )));
    }
    let source_format = reader.assignment_format().ok_or_else(|| {
        PyException::new_err("bundle header has an unrecognized assignment format")
    })?;
    let required_format = match target_format {
        AssignmentFormat::Ben => AssignmentFormat::Xben,
        AssignmentFormat::Xben => AssignmentFormat::Ben,
    };
    if source_format != required_format {
        let required_name = match required_format {
            AssignmentFormat::Ben => "BEN",
            AssignmentFormat::Xben => "XBEN",
        };
        return Err(PyException::new_err(format!(
            "{operation} requires an embedded {required_name} stream"
        )));
    }
    let sample_count = reader.header().sample_count;
    let stream_len = reader.header().stream_len;
    let empty = stream_len == 0 && sample_count == 0;

    // Read every asset's decoded payload up front (each read borrows the reader exclusively).
    let entries: Vec<_> = reader.assets().to_vec();
    let mut assets = Vec::with_capacity(entries.len());
    for entry in &entries {
        let payload = reader.asset_bytes(entry).map_err(|e| {
            PyIOError::new_err(format!("Failed to read asset {:?}: {e}", entry.name))
        })?;
        assets.push(PreservedAsset {
            asset_type: entry.asset_type,
            name: entry.name.clone(),
            is_json: entry.asset_flags & ASSET_FLAG_JSON != 0,
            payload,
        });
    }

    // Build the new bundle with the opposite stream format.
    let (guard, buf) = TempOutput::create(&out_file, overwrite)?;
    let mut writer = BendlWriter::new(buf, target_format)
        .map_err(|e| PyIOError::new_err(format!("Failed to initialize bundle writer: {e}")))?;
    for asset in &assets {
        add_preserved(&mut writer, asset).map_err(map_bundle_err)?;
    }

    // An empty stream has no banner to re-encode, so finalize the assets-only bundle directly.
    let out = if empty {
        writer.finish().map_err(map_bundle_err)?
    } else {
        // Transcode straight into the new stream instead of buffering either potentially
        // multi-gigabyte representation in memory. Reading to EOF also verifies the source CRC.
        let mut session = writer.into_stream_session().map_err(map_bundle_err)?;
        let stream = reader
            .assignment_stream_reader()
            .map_err(|e| PyException::new_err(format!("Failed to open stream region: {e}")))?;
        match target_format {
            AssignmentFormat::Ben => decode_xben_to_ben(BufReader::new(stream), &mut session),
            AssignmentFormat::Xben => {
                encode_ben_to_xben(BufReader::new(stream), &mut session, None, None, None, None)
            }
        }
        .map_err(|e| PyException::new_err(format!("Failed to {operation}: {e}")))?;
        let writer = session.finish_into_writer(sample_count);
        writer.finish().map_err(map_bundle_err)?
    };
    guard.commit_writer(out)?;

    Ok(())
}
