//! Binding for relabeling a `.bendl` bundle: reorder its dual graph and rewrite the embedded BEN
//! assignment stream into the new node order, producing a fresh bundle.
//!
//! This is the bundle-level form of the CLI's `reben` ordering flow. The reordered `graph.json` and
//! a `node_permutation_map.json` are stored as canonical assets so the reordering stays reversible;
//! every other asset (metadata, custom blobs) is carried over by decoded payload, name, type, and
//! JSON flag.

use crate::common::open_output;
use crate::graph::helpers::{reorder_graph_to_bytes, require_reorder};
use binary_ensemble::io::bundle::format::{
    AssignmentFormat, KnownAssetKind, ASSET_FLAG_JSON, ASSET_TYPE_GRAPH, ASSET_TYPE_METADATA,
    ASSET_TYPE_NODE_PERMUTATION_MAP,
};
use binary_ensemble::io::bundle::{AddAssetOptions, BendlReader, BendlWriteError, BendlWriter};
use binary_ensemble::ops::relabel::{relabel_ben_file, RelabelOptions};
use pyo3::exceptions::{PyException, PyIOError, PyValueError};
use pyo3::prelude::*;
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, Cursor, Read, Write};
use std::path::PathBuf;

fn map_bundle_err(err: BendlWriteError) -> PyErr {
    match err {
        BendlWriteError::Io(e) => PyIOError::new_err(format!("{e}")),
        other => PyException::new_err(format!("{other}")),
    }
}

/// A metadata/custom asset carried over unchanged from the source bundle.
struct PreservedAsset {
    asset_type: u16,
    name: String,
    is_json: bool,
    payload: Vec<u8>,
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
    // Keep canonical known assets (e.g. metadata.json) canonical; everything else is custom.
    match asset.asset_type {
        ASSET_TYPE_METADATA => {
            writer.add_known_asset(KnownAssetKind::Metadata, &asset.payload, opts)
        }
        _ => writer.add_custom_asset(&asset.name, &asset.payload, opts),
    }
}

/// Invert a stored `node_permutation_old_to_new` object into the dense `new -> old` map that
/// `relabel_ben_file` consumes.
fn new_to_old_from_map_bytes(map_bytes: &[u8]) -> PyResult<HashMap<usize, usize>> {
    let value: serde_json::Value = serde_json::from_slice(map_bytes)
        .map_err(|e| PyException::new_err(format!("permutation map is not valid JSON: {e}")))?;
    let obj = value
        .get("node_permutation_old_to_new")
        .and_then(|v| v.as_object())
        .ok_or_else(|| {
            PyException::new_err("permutation map missing node_permutation_old_to_new")
        })?;
    let mut new_to_old = HashMap::with_capacity(obj.len());
    for (old_text, new_val) in obj {
        let old = old_text
            .parse::<usize>()
            .map_err(|e| PyException::new_err(format!("invalid node index {old_text:?}: {e}")))?;
        let new = new_val
            .as_u64()
            .ok_or_else(|| PyException::new_err("permutation map value is not an integer"))?
            as usize;
        new_to_old.insert(new, old);
    }
    Ok(new_to_old)
}

/// Relabel the bundle at `in_file` by reordering its graph (via `sort` / `key`), writing a fresh
/// BEN bundle at `out_file`.
#[pyfunction]
#[pyo3(signature = (in_file, out_file, sort = Some("mlc".to_string()), key = None, overwrite = false))]
#[pyo3(text_signature = "(in_file, out_file, sort='mlc', key=None, overwrite=False)")]
pub fn relabel_bundle(
    in_file: PathBuf,
    out_file: PathBuf,
    sort: Option<String>,
    key: Option<String>,
    overwrite: bool,
) -> PyResult<()> {
    let plan = require_reorder(sort.as_deref(), key.as_deref())?;
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
            "relabel_bundle requires a finalized bundle",
        ));
    }
    if !matches!(reader.assignment_format(), Some(AssignmentFormat::Ben)) {
        return Err(PyValueError::new_err(
            "relabel_bundle only supports BEN bundles; relabel before compressing to XBEN",
        ));
    }
    let sample_count = reader.header().sample_count;
    if reader.header().stream_len == 0 && sample_count == 0 {
        return Err(PyValueError::new_err(
            "relabel_bundle requires a non-empty assignment stream",
        ));
    }

    // The graph asset is required: it defines the ordering we permute into.
    let graph_entry = reader
        .find_asset_by_type(ASSET_TYPE_GRAPH)
        .cloned()
        .ok_or_else(|| PyValueError::new_err("bundle has no graph.json to reorder"))?;
    let graph_bytes = reader
        .asset_bytes(&graph_entry)
        .map_err(|e| PyIOError::new_err(format!("Failed to read graph asset: {e}")))?;

    // Reorder the graph and derive the new->old permutation for the stream.
    let (reordered_graph, map_bytes) = reorder_graph_to_bytes(&graph_bytes, &plan)?;
    let new_to_old = new_to_old_from_map_bytes(&map_bytes)?;

    // Carry over every other asset (skip the old graph and any old permutation map; we rewrite
    // those as canonical assets below).
    let entries: Vec<_> = reader.assets().to_vec();
    let mut preserved = Vec::new();
    for entry in &entries {
        if entry.asset_type == ASSET_TYPE_GRAPH
            || entry.asset_type == ASSET_TYPE_NODE_PERMUTATION_MAP
        {
            continue;
        }
        let payload = reader.asset_bytes(entry).map_err(|e| {
            PyIOError::new_err(format!("Failed to read asset {:?}: {e}", entry.name))
        })?;
        preserved.push(PreservedAsset {
            asset_type: entry.asset_type,
            name: entry.name.clone(),
            is_json: entry.asset_flags & ASSET_FLAG_JSON != 0,
            payload,
        });
    }

    // Read the BEN stream and relabel it into the new node order.
    let mut ben_bytes = Vec::new();
    reader
        .assignment_stream_reader()
        .map_err(|e| PyException::new_err(format!("Failed to open stream region: {e}")))?
        .read_to_end(&mut ben_bytes)
        .map_err(|e| PyIOError::new_err(format!("Failed to read BEN stream: {e}")))?;
    let mut relabeled = Vec::new();
    relabel_ben_file(
        Cursor::new(ben_bytes),
        &mut relabeled,
        RelabelOptions::node_permutation(new_to_old),
    )
    .map_err(|e| PyException::new_err(format!("Failed to relabel BEN stream: {e}")))?;

    // Write the new bundle: reordered graph + permutation map (canonical), then the rest.
    let buf = open_output(&out_file, overwrite)?;
    let mut writer = BendlWriter::new(buf, AssignmentFormat::Ben)
        .map_err(|e| PyIOError::new_err(format!("Failed to initialize bundle writer: {e}")))?;
    writer
        .add_known_asset(
            KnownAssetKind::Graph,
            &reordered_graph,
            AddAssetOptions::defaults().json(),
        )
        .map_err(map_bundle_err)?;
    writer
        .add_known_asset(
            KnownAssetKind::NodePermutationMap,
            &map_bytes,
            AddAssetOptions::defaults().json(),
        )
        .map_err(map_bundle_err)?;
    for asset in &preserved {
        add_preserved(&mut writer, asset).map_err(map_bundle_err)?;
    }

    let mut session = writer.into_stream_session().map_err(map_bundle_err)?;
    session
        .write_all(&relabeled)
        .map_err(|e| PyIOError::new_err(format!("Failed to write relabeled stream: {e}")))?;
    let writer = session.finish_into_writer(sample_count);
    writer.finish().map_err(map_bundle_err)?;

    Ok(())
}
