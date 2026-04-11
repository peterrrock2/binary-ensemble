//! Python bindings for the `.bendl` bundle container.
//!
//! Exposes a [`PyBundleReader`] that wraps
//! [`binary_ensemble::io::bundle::BendlReader`] and provides a small
//! Python-facing surface:
//!
//! - `is_complete()`, `sample_count()`, `assignment_format()`
//! - `asset_names()` / `list_assets()`
//! - `read_asset_bytes(name)` — raw (decoded) bytes as `bytes`
//! - `read_json_asset(name)` — parsed JSON as a Python object
//! - `read_graph()` / `read_metadata()` / `read_relabel_map()` — canonical-name helpers
//! - `extract_stream(out_path, overwrite=False)` — copy the embedded
//!   assignment stream to a `.ben` / `.xben` file the caller can then
//!   open with `PyBenDecoder`.

use std::fs::{File, OpenOptions};
use std::io::{self, BufReader, BufWriter};
use std::path::PathBuf;

use binary_ensemble::io::bundle::format::{
    AssignmentFormat, ASSET_FLAG_CHECKSUM, ASSET_FLAG_JSON, ASSET_FLAG_XZ, ASSET_TYPE_GRAPH,
    ASSET_TYPE_METADATA, ASSET_TYPE_RELABEL_MAP,
};
use binary_ensemble::io::bundle::BendlReader;
use pyo3::exceptions::{PyException, PyIOError, PyKeyError};
use pyo3::prelude::*;
use pyo3::types::PyDict;

/// Python-facing wrapper around a `BendlReader<BufReader<File>>`.
#[pyclass(module = "binary_ensemble", unsendable, name = "PyBundleReader")]
pub struct PyBundleReader {
    inner: BendlReader<BufReader<File>>,
    path: PathBuf,
}

#[pymethods]
impl PyBundleReader {
    /// Open a `.bendl` file for reading.
    #[new]
    #[pyo3(text_signature = "(file_path)")]
    fn new(file_path: PathBuf) -> PyResult<Self> {
        let file = File::open(&file_path).map_err(|e| {
            PyIOError::new_err(format!("Failed to open {}: {e}", file_path.display()))
        })?;
        let inner = BendlReader::open(BufReader::new(file)).map_err(|e| {
            PyException::new_err(format!(
                "Failed to parse bundle header in {}: {e}",
                file_path.display()
            ))
        })?;
        Ok(Self {
            inner,
            path: file_path,
        })
    }

    /// Return the bundle's format version as a `(major, minor)` tuple.
    #[pyo3(text_signature = "(self)")]
    fn version(&self) -> (u16, u16) {
        let h = self.inner.header();
        (h.major_version, h.minor_version)
    }

    /// Whether the bundle was successfully finalized.
    #[pyo3(text_signature = "(self)")]
    fn is_complete(&self) -> bool {
        self.inner.is_complete()
    }

    /// Authoritative sample count from the header, or `None` when the
    /// bundle is incomplete.
    #[pyo3(text_signature = "(self)")]
    fn sample_count(&self) -> Option<i64> {
        self.inner.sample_count()
    }

    /// Container format of the embedded assignment stream: `"ben"` or
    /// `"xben"`, or `None` when the header byte is unrecognized.
    #[pyo3(text_signature = "(self)")]
    fn assignment_format(&self) -> Option<&'static str> {
        self.inner.assignment_format().map(|f| match f {
            AssignmentFormat::Ben => "ben",
            AssignmentFormat::Xben => "xben",
        })
    }

    /// Names of all directory entries, in directory order.
    #[pyo3(text_signature = "(self)")]
    fn asset_names(&self) -> Vec<String> {
        self.inner
            .assets()
            .iter()
            .map(|e| e.name.clone())
            .collect()
    }

    /// Return the full directory as a list of dicts with keys
    /// `name`, `type`, `offset`, `len`, and `flags` (a list of string tags).
    #[pyo3(text_signature = "(self)")]
    fn list_assets<'py>(&self, py: Python<'py>) -> PyResult<Vec<Bound<'py, PyDict>>> {
        let mut out = Vec::with_capacity(self.inner.assets().len());
        for entry in self.inner.assets() {
            let d = PyDict::new(py);
            d.set_item("name", &entry.name)?;
            d.set_item("type", entry.asset_type)?;
            d.set_item("offset", entry.payload_offset)?;
            d.set_item("len", entry.payload_len)?;
            let mut flags: Vec<&str> = Vec::new();
            if entry.asset_flags & ASSET_FLAG_JSON != 0 {
                flags.push("json");
            }
            if entry.asset_flags & ASSET_FLAG_XZ != 0 {
                flags.push("xz");
            }
            if entry.asset_flags & ASSET_FLAG_CHECKSUM != 0 {
                flags.push("checksum");
            }
            d.set_item("flags", flags)?;
            out.push(d);
        }
        Ok(out)
    }

    /// Read the (decoded) bytes of an asset by name and return them as
    /// a Python `bytes` object.
    #[pyo3(text_signature = "(self, name, /)")]
    fn read_asset_bytes(&mut self, name: &str) -> PyResult<Vec<u8>> {
        let entry = self
            .inner
            .find_asset_by_name(name)
            .cloned()
            .ok_or_else(|| PyKeyError::new_err(format!("no asset named {name:?} in bundle")))?;
        self.inner
            .asset_bytes(&entry)
            .map_err(|e| PyIOError::new_err(format!("Failed to read asset {name:?}: {e}")))
    }

    /// Parse a JSON asset into a Python object (dict, list, …). Fails
    /// if the asset does not exist or the decoded bytes are not JSON.
    #[pyo3(text_signature = "(self, name, /)")]
    fn read_json_asset<'py>(&mut self, py: Python<'py>, name: &str) -> PyResult<Py<PyAny>> {
        let bytes = self.read_asset_bytes(name)?;
        let json_mod = py.import("json")?;
        let text = std::str::from_utf8(&bytes).map_err(|e| {
            PyException::new_err(format!("asset {name:?} is not valid UTF-8: {e}"))
        })?;
        let parsed = json_mod.call_method1("loads", (text,))?;
        Ok(parsed.into())
    }

    /// Read the bundle's `graph.json` asset as a parsed JSON object.
    /// Returns `None` if the bundle does not carry a graph asset.
    #[pyo3(text_signature = "(self)")]
    fn read_graph<'py>(&mut self, py: Python<'py>) -> PyResult<Option<Py<PyAny>>> {
        if self
            .inner
            .find_asset_by_type(ASSET_TYPE_GRAPH)
            .is_none()
        {
            return Ok(None);
        }
        Ok(Some(self.read_json_asset(py, "graph.json")?))
    }

    /// Read the bundle's `metadata.json` asset as a parsed JSON object,
    /// or `None` if absent.
    #[pyo3(text_signature = "(self)")]
    fn read_metadata<'py>(&mut self, py: Python<'py>) -> PyResult<Option<Py<PyAny>>> {
        if self
            .inner
            .find_asset_by_type(ASSET_TYPE_METADATA)
            .is_none()
        {
            return Ok(None);
        }
        Ok(Some(self.read_json_asset(py, "metadata.json")?))
    }

    /// Read the bundle's `relabel_map.json` asset as a parsed JSON
    /// object, or `None` if absent.
    #[pyo3(text_signature = "(self)")]
    fn read_relabel_map<'py>(&mut self, py: Python<'py>) -> PyResult<Option<Py<PyAny>>> {
        if self
            .inner
            .find_asset_by_type(ASSET_TYPE_RELABEL_MAP)
            .is_none()
        {
            return Ok(None);
        }
        Ok(Some(self.read_json_asset(py, "relabel_map.json")?))
    }

    /// Copy the embedded assignment stream region verbatim to
    /// `out_path`. The resulting file can be opened directly with
    /// `PyBenDecoder(out_path, mode=assignment_format())`.
    #[pyo3(signature = (out_path, overwrite=false))]
    #[pyo3(text_signature = "(self, out_path, overwrite=False)")]
    fn extract_stream(&mut self, out_path: PathBuf, overwrite: bool) -> PyResult<()> {
        if out_path.exists() && !overwrite {
            return Err(PyIOError::new_err(format!(
                "Output file {} already exists (use overwrite=True to replace).",
                out_path.display()
            )));
        }
        let out = if overwrite {
            OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(&out_path)
        } else {
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&out_path)
        }
        .map_err(|e| {
            PyIOError::new_err(format!("Failed to create {}: {e}", out_path.display()))
        })?;
        let mut out = BufWriter::new(out);

        let mut stream = self.inner.assignment_stream_reader().map_err(|e| {
            PyException::new_err(format!("Failed to open stream region: {e}"))
        })?;
        io::copy(&mut stream, &mut out).map_err(|e| {
            PyIOError::new_err(format!("Failed to copy stream bytes: {e}"))
        })?;
        Ok(())
    }

    fn __repr__(&self) -> String {
        format!(
            "PyBundleReader(path={:?}, complete={}, format={:?}, samples={:?}, assets={})",
            self.path.display(),
            self.inner.is_complete(),
            self.inner.assignment_format(),
            self.inner.sample_count(),
            self.inner.assets().len(),
        )
    }
}
