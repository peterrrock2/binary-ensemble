//! Binding for recompressing a `.bendl` file's embedded BEN stream to XBEN.
//!
//! This repackages a bundle: it reads back every asset's decoded payload and the BEN assignment
//! stream, re-encodes the stream as XBEN, and writes a fresh `Xben`-format bundle with the same
//! assets (name, type, JSON flag, decoded bytes). Storage compression is normalized to the writer's
//! default policy — the decoded payload bytes are preserved, not the byte-for-byte on-disk form.

use crate::common::TempOutput;
use binary_ensemble::codec::encode::encode_ben_to_xben;
use binary_ensemble::io::bundle::format::{
    AssignmentFormat, KnownAssetKind, ASSET_FLAG_JSON, ASSET_TYPE_GRAPH, ASSET_TYPE_METADATA,
    ASSET_TYPE_NODE_PERMUTATION_MAP,
};
use binary_ensemble::io::bundle::{AddAssetOptions, BendlReader, BendlWriteError, BendlWriter};
use pyo3::exceptions::{PyException, PyIOError};
use pyo3::prelude::*;
use std::fs::File;
use std::io::{BufReader, Cursor, Read, Write};
use std::path::PathBuf;

fn map_bundle_err(err: BendlWriteError) -> PyErr {
    match err {
        BendlWriteError::Io(e) => PyIOError::new_err(format!("{e}")),
        other => PyException::new_err(format!("{other}")),
    }
}

/// A single asset read back from the source bundle, ready to be re-added to the new one.
struct PreservedAsset {
    asset_type: u16,
    name: String,
    is_json: bool,
    payload: Vec<u8>,
}

fn known_kind(asset_type: u16) -> Option<KnownAssetKind> {
    match asset_type {
        ASSET_TYPE_METADATA => Some(KnownAssetKind::Metadata),
        ASSET_TYPE_GRAPH => Some(KnownAssetKind::Graph),
        ASSET_TYPE_NODE_PERMUTATION_MAP => Some(KnownAssetKind::NodePermutationMap),
        _ => None,
    }
}

fn add_preserved<W: Write + std::io::Seek>(
    writer: &mut BendlWriter<W>,
    asset: &PreservedAsset,
) -> Result<(), BendlWriteError> {
    let opts = if asset.is_json {
        AddAssetOptions::defaults().json()
    } else {
        AddAssetOptions::defaults()
    };
    match known_kind(asset.asset_type) {
        Some(kind) => writer.add_known_asset(kind, &asset.payload, opts),
        None => writer.add_custom_asset(&asset.name, &asset.payload, opts),
    }
}

/// Recompress the BEN stream of the bundle at `in_file` to XBEN, writing a new bundle at
/// `out_file`.
///
/// This is the raw core binding; prefer the :func:`binary_ensemble.bundle.compress_stream`
/// facade, which adds the ``in_place`` atomic-swap mode.
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
pub fn recompress_bundle(in_file: PathBuf, out_file: PathBuf, overwrite: bool) -> PyResult<()> {
    let file = File::open(&in_file)
        .map_err(|e| PyIOError::new_err(format!("Failed to open {}: {e}", in_file.display())))?;
    let mut reader = BendlReader::open(BufReader::new(file)).map_err(|e| {
        PyException::new_err(format!(
            "Failed to parse bundle header in {}: {e}",
            in_file.display()
        ))
    })?;

    if !reader.is_finalized() {
        return Err(PyException::new_err(
            "compress_stream requires a finalized bundle",
        ));
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

    // Recompress the BEN stream to XBEN bytes (skipped for an empty stream — there is no banner).
    let xben_bytes = if empty {
        Vec::new()
    } else {
        let mut ben_bytes = Vec::new();
        let mut stream = reader
            .assignment_stream_reader()
            .map_err(|e| PyException::new_err(format!("Failed to open stream region: {e}")))?;
        stream
            .read_to_end(&mut ben_bytes)
            .map_err(|e| PyIOError::new_err(format!("Failed to read BEN stream: {e}")))?;
        let mut out = Vec::new();
        encode_ben_to_xben(Cursor::new(ben_bytes), &mut out, None, None, None, None).map_err(
            |e| PyException::new_err(format!("Failed to recompress BEN stream to XBEN: {e}")),
        )?;
        out
    };

    // Build the new XBEN bundle.
    let (guard, buf) = TempOutput::create(&out_file, overwrite)?;
    let mut writer = BendlWriter::new(buf, AssignmentFormat::Xben)
        .map_err(|e| PyIOError::new_err(format!("Failed to initialize bundle writer: {e}")))?;
    for asset in &assets {
        add_preserved(&mut writer, asset).map_err(map_bundle_err)?;
    }

    let out = if empty {
        writer.finish().map_err(map_bundle_err)?
    } else {
        let mut session = writer.into_stream_session().map_err(map_bundle_err)?;
        session
            .write_all(&xben_bytes)
            .map_err(|e| PyIOError::new_err(format!("Failed to write XBEN stream: {e}")))?;
        let writer = session.finish_into_writer(sample_count);
        writer.finish().map_err(map_bundle_err)?
    };
    guard.commit_writer(out)?;

    Ok(())
}
