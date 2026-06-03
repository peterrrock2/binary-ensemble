use binary_ensemble::BenVariant;
use pyo3::exceptions::{PyException, PyIOError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict, PyList};
use std::fs::File;
use std::io::{BufReader, BufWriter};
use std::path::PathBuf;

pub fn parse_variant(variant: Option<&str>) -> PyResult<BenVariant> {
    match variant {
        Some("standard") => Ok(BenVariant::Standard),
        Some("mkv_chain") | Some("markov") | None => Ok(BenVariant::MkvChain),
        Some("twodelta") | Some("two_delta") => Ok(BenVariant::TwoDelta),
        Some(other) => Err(PyValueError::new_err(format!(
            "Unknown variant: {other}. Supported variants are 'standard', 'mkv_chain', and 'twodelta'."
        ))),
    }
}

pub fn validate_input_output_paths(in_file: &PathBuf, out_file: &PathBuf) -> PyResult<()> {
    if in_file == out_file {
        return Err(PyIOError::new_err("Input and output paths must differ."));
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

/// Normalize a user-supplied graph argument into raw UTF-8 JSON bytes.
///
/// Accepted forms:
///
/// - `dict` / `list`: serialized via `json.dumps`.
/// - `bytes` / `bytearray`: used verbatim.
/// - any object with a `.read()` method (e.g. `io.BytesIO`, open files): `.read()` is called and
///   the result is coerced to bytes.
/// - `pathlib.Path` or `str`: treated as a filesystem path to read.
pub fn parse_graph_input(py: Python<'_>, obj: &Bound<'_, PyAny>) -> PyResult<Vec<u8>> {
    // Dict / list → json.dumps.
    if obj.is_instance_of::<PyDict>() || obj.is_instance_of::<PyList>() {
        let json_mod = py.import("json")?;
        let dumped = json_mod.call_method1("dumps", (obj,))?;
        let s: String = dumped.extract()?;
        return Ok(s.into_bytes());
    }

    // Raw bytes / bytearray.
    if let Ok(b) = obj.downcast::<PyBytes>() {
        return Ok(b.as_bytes().to_vec());
    }
    if let Ok(b) = obj.extract::<Vec<u8>>() {
        return Ok(b);
    }

    // File-like: must have .read(). Check before str/path, since a plain `str` / `Path` has no
    // `.read()` attribute and will fall through.
    if obj.hasattr("read")? {
        let data = obj.call_method0("read")?;
        if let Ok(b) = data.downcast::<PyBytes>() {
            return Ok(b.as_bytes().to_vec());
        }
        if let Ok(b) = data.extract::<Vec<u8>>() {
            return Ok(b);
        }
        if let Ok(s) = data.extract::<String>() {
            return Ok(s.into_bytes());
        }
        return Err(PyException::new_err(
            "graph .read() must return bytes or str",
        ));
    }

    // Path / str → read the file at that path.
    let path: PathBuf = obj.extract().map_err(|_| {
        PyValueError::new_err(
            "graph must be a dict/list, bytes, a file-like with .read(), or a path",
        )
    })?;
    std::fs::read(&path).map_err(|e| {
        PyIOError::new_err(format!("Failed to read graph file {}: {e}", path.display()))
    })
}

/// Build a live NetworkX graph from an already-parsed adjacency-format JSON object.
///
/// The shared tail behind every API that hands a graph back to the caller —
/// `BendlEncoder.add_graph`, `BendlDecoder.read_graph`, and the `graph` reordering utilities — so
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
