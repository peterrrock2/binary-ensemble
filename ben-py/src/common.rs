use binary_ensemble::io::bundle::format::{
    KnownAssetKind, ASSET_TYPE_GRAPH, ASSET_TYPE_METADATA, ASSET_TYPE_NODE_PERMUTATION_MAP,
};
use binary_ensemble::io::bundle::{AddAssetOptions, BendlWriteError, BendlWriter};
use binary_ensemble::BenVariant;
use pyo3::exceptions::{PyException, PyIOError, PyKeyError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyByteArray, PyBytes, PyDict, PyList, PyMemoryView};
use std::fs::{self, File};
use std::io::{BufReader, BufWriter};
use std::io::{Seek, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Map a core bundle write error onto the Python exception taxonomy, identically at every
/// entry point, so the same core failure never surfaces as different exception types depending
/// on which transform raised it. Unknown names are ``KeyError`` (matching the decoder's lookup
/// errors), reserved names are ``ValueError``, IO is ``OSError``, the rest a generic
/// ``Exception``.
pub fn map_bundle_err(err: BendlWriteError) -> PyErr {
    match err {
        BendlWriteError::Io(e) => PyIOError::new_err(format!("{e}")),
        BendlWriteError::UnknownAssetName(name) => {
            PyKeyError::new_err(format!("no asset named {name:?} in bundle"))
        }
        err @ BendlWriteError::ReservedAssetName { .. } => PyValueError::new_err(format!("{err}")),
        other => PyException::new_err(format!("{other}")),
    }
}

/// A single asset read back from a source bundle by decoded payload, ready to be re-added to a
/// new one. The whole-bundle transforms that re-encode payloads (recompress, relabel) carry
/// assets this way; verbatim stored-form preservation lives in the core compaction.
pub struct PreservedAsset {
    pub asset_type: u16,
    pub name: String,
    pub is_json: bool,
    pub payload: Vec<u8>,
}

fn known_kind(asset_type: u16) -> Option<KnownAssetKind> {
    match asset_type {
        ASSET_TYPE_METADATA => Some(KnownAssetKind::Metadata),
        ASSET_TYPE_GRAPH => Some(KnownAssetKind::Graph),
        ASSET_TYPE_NODE_PERMUTATION_MAP => Some(KnownAssetKind::NodePermutationMap),
        _ => None,
    }
}

/// Re-add a preserved asset under its original kind: known singleton types stay canonical,
/// everything else is re-added as a custom asset.
pub fn add_preserved<W: Write + Seek>(
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

pub fn parse_variant(variant: Option<&str>) -> PyResult<BenVariant> {
    match variant {
        Some("standard") => Ok(BenVariant::Standard),
        Some("mkv_chain") | Some("markov") => Ok(BenVariant::MkvChain),
        Some("twodelta") | Some("two_delta") | None => Ok(BenVariant::TwoDelta),
        Some(other) => Err(PyValueError::new_err(format!(
            "Unknown variant: {other}. Supported variants are 'standard', 'mkv_chain', and 'twodelta'."
        ))),
    }
}

pub fn validate_input_output_paths(in_file: &PathBuf, out_file: &PathBuf) -> PyResult<()> {
    if in_file == out_file {
        // A path collision is a bad-argument error, not an I/O failure. (In-place transforms that
        // intentionally support `src == dest` route through a temp file instead of this helper.)
        return Err(PyValueError::new_err("Input and output paths must differ."));
    }
    if !in_file.exists() {
        return Err(PyIOError::new_err(format!(
            "Input file {} does not exist.",
            in_file.display()
        )));
    }
    Ok(())
}

pub fn open_input(in_file: &PathBuf) -> PyResult<BufReader<File>> {
    let infile = File::open(in_file)
        .map_err(|e| PyIOError::new_err(format!("Failed to open {}: {e}", in_file.display())))?;
    Ok(BufReader::new(infile))
}

pub fn open_output(out_file: &PathBuf, overwrite: bool) -> PyResult<BufWriter<File>> {
    if out_file.exists() && !overwrite {
        return Err(PyIOError::new_err(format!(
            "Output file {} already exists (use overwrite=True to replace).",
            out_file.display()
        )));
    }

    let out_open = if overwrite {
        File::options()
            .write(true)
            .create(true)
            .truncate(true)
            .open(out_file)
    } else {
        File::options().write(true).create_new(true).open(out_file)
    };
    let outfile = out_open
        .map_err(|e| PyIOError::new_err(format!("Failed to create {}: {e}", out_file.display())))?;
    Ok(BufWriter::new(outfile))
}

/// A one-shot output destination written via temp-file-then-rename.
///
/// The destination is never visible half-written: bytes go to a uniquely named hidden temp file
/// in the destination's directory, and only an explicit commit (after an fsync) renames it
/// into place. If the guard drops uncommitted (any error path), the temp file is removed and an
/// existing destination is left exactly as it was, even with `overwrite = true`. When the
/// destination already exists (including the in-place case where it is also the source), the
/// temp file inherits its permissions so the swap never changes the file's mode.
///
/// [`open_output`] remains the right call for *long-lived* outputs (the encoders), whose file
/// must live at its real path while it is being written.
pub struct TempOutput {
    tmp: PathBuf,
    dest: PathBuf,
    committed: bool,
}

static TEMP_OUTPUT_COUNTER: AtomicU64 = AtomicU64::new(0);

impl TempOutput {
    /// Refuse an existing, unoverwritable destination, then create the temp file beside it.
    pub fn create(dest: &Path, overwrite: bool) -> PyResult<(Self, BufWriter<File>)> {
        if dest.exists() && !overwrite {
            return Err(PyIOError::new_err(format!(
                "Output file {} already exists (use overwrite=True to replace).",
                dest.display()
            )));
        }
        let file_name = dest
            .file_name()
            .ok_or_else(|| {
                PyIOError::new_err(format!("Output path {} has no file name", dest.display()))
            })?
            .to_string_lossy()
            .into_owned();
        let tmp = dest.with_file_name(format!(
            ".{file_name}.{}-{}.tmp",
            std::process::id(),
            TEMP_OUTPUT_COUNTER.fetch_add(1, Ordering::Relaxed),
        ));
        let file = File::options()
            .write(true)
            .create_new(true)
            .open(&tmp)
            .map_err(|e| PyIOError::new_err(format!("Failed to create {}: {e}", tmp.display())))?;
        // Inherit an existing destination's permissions before any byte is written, so the swap
        // never changes the file's mode (and a private file's contents never sit world-readable).
        if let Ok(meta) = fs::metadata(dest) {
            let _ = fs::set_permissions(&tmp, meta.permissions());
        }
        Ok((
            TempOutput {
                tmp,
                dest: dest.to_path_buf(),
                committed: false,
            },
            BufWriter::new(file),
        ))
    }

    /// Flush and sync the finished writer, then rename the temp file over the destination.
    pub fn commit_writer(self, writer: BufWriter<File>) -> PyResult<()> {
        let file = writer.into_inner().map_err(|e| {
            PyIOError::new_err(format!("Failed to flush {}: {e}", self.tmp.display()))
        })?;
        file.sync_all().map_err(|e| {
            PyIOError::new_err(format!("Failed to sync {}: {e}", self.tmp.display()))
        })?;
        drop(file);
        self.rename_into_place()
    }

    /// Sync the (already fully written and flushed) temp file by reopening it, then rename it
    /// over the destination. For call sites whose writer was consumed by the core routine.
    pub fn commit(self) -> PyResult<()> {
        let file = File::options().write(true).open(&self.tmp).map_err(|e| {
            PyIOError::new_err(format!("Failed to reopen {}: {e}", self.tmp.display()))
        })?;
        file.sync_all().map_err(|e| {
            PyIOError::new_err(format!("Failed to sync {}: {e}", self.tmp.display()))
        })?;
        drop(file);
        self.rename_into_place()
    }

    fn rename_into_place(mut self) -> PyResult<()> {
        fs::rename(&self.tmp, &self.dest).map_err(|e| {
            PyIOError::new_err(format!(
                "Failed to move {} into place at {}: {e}",
                self.tmp.display(),
                self.dest.display()
            ))
        })?;
        self.committed = true;
        // Make the rename itself durable where the platform allows it; the data already is.
        #[cfg(unix)]
        if let Some(parent) = self.dest.parent().filter(|p| !p.as_os_str().is_empty()) {
            if let Ok(dir) = File::open(parent) {
                let _ = dir.sync_all();
            }
        }
        Ok(())
    }
}

impl Drop for TempOutput {
    fn drop(&mut self) {
        if !self.committed {
            let _ = fs::remove_file(&self.tmp);
        }
    }
}

/// Validate that `bytes` parse as JSON without building a value tree (cheap even for large
/// graphs), so a payload stored with the JSON flag can never be silently invalid.
fn validated_json(bytes: Vec<u8>, what: &str) -> PyResult<Vec<u8>> {
    serde_json::from_slice::<serde::de::IgnoredAny>(&bytes)
        .map_err(|e| PyValueError::new_err(format!("{what} is not valid JSON: {e}")))?;
    Ok(bytes)
}

/// Normalize a user-supplied JSON payload argument into raw UTF-8 JSON bytes, validating that
/// the result really is JSON (dict/list inputs are serialized via `json.dumps` and need no
/// re-validation; every other form is checked).
///
/// `what` names the argument in error messages; `accepted` describes the accepted forms in the
/// final unsupported-type error. Accepted forms:
///
/// - `dict` / `list`: serialized via `json.dumps`.
/// - `bytes` / `bytearray`: used verbatim.
/// - any object with a `.read()` method (e.g. `io.BytesIO`, open files): `.read()` is called and
///   the result is coerced to bytes.
/// - `pathlib.Path` or `str`: treated as a filesystem path to read.
pub fn parse_json_input(
    py: Python<'_>,
    obj: &Bound<'_, PyAny>,
    what: &str,
    accepted: &str,
) -> PyResult<Vec<u8>> {
    // Dict / list → json.dumps.
    if obj.is_instance_of::<PyDict>() || obj.is_instance_of::<PyList>() {
        let json_mod = py.import("json")?;
        let dumped = json_mod.call_method1("dumps", (obj,))?;
        let s: String = dumped.extract()?;
        return Ok(s.into_bytes());
    }

    // Raw bytes / bytearray. Deliberately strict downcasts: a generic Vec<u8> extraction would
    // also accept any sequence of small ints (e.g. iterating a NetworkX graph's node ids) and
    // silently store garbage.
    if let Ok(b) = obj.cast::<PyBytes>() {
        return validated_json(b.as_bytes().to_vec(), what);
    }
    if let Ok(b) = obj.cast::<PyByteArray>() {
        return validated_json(b.to_vec(), what);
    }
    if obj.cast::<PyMemoryView>().is_ok() {
        let data = obj.call_method0("tobytes")?;
        let b = data.cast::<PyBytes>().map_err(PyErr::from)?;
        return validated_json(b.as_bytes().to_vec(), what);
    }

    // File-like: must have .read(). Check before str/path, since a plain `str` / `Path` has no
    // `.read()` attribute and will fall through.
    if obj.hasattr("read")? {
        let data = obj.call_method0("read")?;
        if let Ok(b) = data.cast::<PyBytes>() {
            return validated_json(b.as_bytes().to_vec(), what);
        }
        if let Ok(b) = data.extract::<Vec<u8>>() {
            return validated_json(b, what);
        }
        if let Ok(s) = data.extract::<String>() {
            return validated_json(s.into_bytes(), what);
        }
        return Err(PyException::new_err(format!(
            "{what} .read() must return bytes or str"
        )));
    }

    // Path / str → read the file at that path.
    let path: PathBuf = obj
        .extract()
        .map_err(|_| PyValueError::new_err(format!("{what} must be {accepted}")))?;
    let bytes = std::fs::read(&path).map_err(|e| {
        PyIOError::new_err(format!(
            "Failed to read {what} file {}: {e}",
            path.display()
        ))
    })?;
    validated_json(bytes, what)
}

/// Normalize a user-supplied metadata argument into raw UTF-8 JSON bytes.
pub fn parse_metadata_input(py: Python<'_>, obj: &Bound<'_, PyAny>) -> PyResult<Vec<u8>> {
    parse_json_input(
        py,
        obj,
        "metadata",
        "a dict/list, bytes, a file-like with .read(), or a path",
    )
}

/// Convert a live NetworkX graph into adjacency-format JSON bytes, or return `None` if `obj` is
/// not a NetworkX graph (subclasses such as `gerrychain.Graph` count).
fn networkx_graph_to_json_bytes(
    py: Python<'_>,
    obj: &Bound<'_, PyAny>,
) -> PyResult<Option<Vec<u8>>> {
    let networkx = py.import("networkx")?;
    let graph_cls = networkx.getattr("Graph")?;
    if !obj.is_instance(&graph_cls)? {
        return Ok(None);
    }
    // adjacency_data preserves the graph's node iteration order, so a raw (sort=None) embed
    // stores exactly the order the caller's graph already has.
    let json_graph = py.import("networkx.readwrite.json_graph")?;
    let data = json_graph.call_method1("adjacency_data", (obj,))?;
    let json_mod = py.import("json")?;
    let dumped = json_mod.call_method1("dumps", (&data,))?;
    let s: String = dumped.extract()?;
    Ok(Some(s.into_bytes()))
}

/// Normalize a user-supplied graph argument into raw adjacency-format UTF-8 JSON bytes.
///
/// Accepts everything [`parse_json_input`] does, plus a live NetworkX graph (serialized via
/// `networkx.readwrite.json_graph.adjacency_data`, preserving its node order).
pub fn parse_graph_input(py: Python<'_>, obj: &Bound<'_, PyAny>) -> PyResult<Vec<u8>> {
    if let Some(bytes) = networkx_graph_to_json_bytes(py, obj)? {
        return Ok(bytes);
    }
    parse_json_input(
        py,
        obj,
        "graph",
        "a networkx.Graph, dict/list, bytes, a file-like with .read(), or a path",
    )
}

/// Build a live NetworkX graph from an already-parsed adjacency-format JSON object.
///
/// The shared tail behind every API that hands a graph back to the caller
/// (`BendlEncoder.add_graph`, `BendlDecoder.read_graph`, and the `graph` reordering utilities), so
/// they all return graphs in the same shape.
pub fn networkx_graph_from_json(py: Python<'_>, data: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
    let json_graph = py.import("networkx.readwrite.json_graph")?;
    Ok(json_graph.call_method1("adjacency_graph", (data,))?.into())
}

/// Build a live NetworkX graph from adjacency-format JSON bytes.
pub fn networkx_graph_from_bytes(py: Python<'_>, bytes: &[u8]) -> PyResult<Py<PyAny>> {
    let json_mod = py.import("json")?;
    let text = std::str::from_utf8(bytes)
        .map_err(|e| PyException::new_err(format!("graph is not valid UTF-8: {e}")))?;
    let data = json_mod.call_method1("loads", (text,))?;
    networkx_graph_from_json(py, &data)
}

/// Count the number of nodes declared in a NetworkX adjacency-format graph's `nodes` array.
///
/// Used to validate that each assignment written to a bundle stream matches the embedded graph's
/// node count.
pub fn graph_node_count(graph_bytes: &[u8]) -> PyResult<usize> {
    let value: serde_json::Value = serde_json::from_slice(graph_bytes)
        .map_err(|e| PyValueError::new_err(format!("graph is not valid JSON: {e}")))?;
    value
        .get("nodes")
        .and_then(|n| n.as_array())
        .map(|a| a.len())
        .ok_or_else(|| PyValueError::new_err("graph JSON has no 'nodes' array to count"))
}
