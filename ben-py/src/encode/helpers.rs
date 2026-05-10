use pyo3::exceptions::{PyException, PyIOError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict, PyList};
use std::path::PathBuf;

/// Normalize a user-supplied graph argument into raw UTF-8 JSON bytes.
///
/// Accepted forms:
///
/// - `dict` / `list`: serialized via `json.dumps`.
/// - `bytes` / `bytearray`: used verbatim.
/// - any object with a `.read()` method (e.g. `io.BytesIO`, open files):
///   `.read()` is called and the result is coerced to bytes.
/// - `pathlib.Path` or `str`: treated as a filesystem path to read.
pub(super) fn parse_graph_input(py: Python<'_>, obj: &Bound<'_, PyAny>) -> PyResult<Vec<u8>> {
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

    // File-like: must have .read(). Check before str/path, since a plain
    // `str` / `Path` has no `.read()` attribute and will fall through.
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
