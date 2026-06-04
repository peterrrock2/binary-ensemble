use super::cursor::SampleCursor;
use super::helpers::{detect_is_bundle, warn_xben_startup};
use super::types::{DecoderMode, StreamSource};
use binary_ensemble::io::bundle::format::{
    ASSET_FLAG_CHECKSUM, ASSET_FLAG_JSON, ASSET_FLAG_XZ, ASSET_TYPE_GRAPH, ASSET_TYPE_METADATA,
    ASSET_TYPE_NODE_PERMUTATION_MAP,
};
use binary_ensemble::io::bundle::BendlReader;
use pyo3::exceptions::{PyException, PyIOError, PyKeyError};
use pyo3::prelude::*;
use pyo3::types::PyDict;
use std::fs::{File, OpenOptions};
use std::io::{self, BufReader, BufWriter, Write};
use std::path::PathBuf;

/// Reader and iterator for a `.bendl` bundle.
///
/// This decoder is bundle-only: opening it on a plain `.ben`/`.xben` stream raises and points the
/// caller at `BenDecoder`. It exposes the bundle inspection surface (`version`, `is_complete`,
/// `asset_names`, `list_assets`, canonical and generic asset getters, `extract_stream`) and
/// iterates the embedded assignment stream.
#[pyclass(module = "binary_ensemble", name = "BendlDecoder", unsendable)]
pub struct PyBendlDecoder {
    path: PathBuf,
    reader: BendlReader<BufReader<File>>,
    cursor: SampleCursor,
}

#[pymethods]
impl PyBendlDecoder {
    /// Open a decoder on a `.bendl` bundle.
    ///
    /// The file's leading bytes are sniffed; a plain `.ben`/`.xben` stream is rejected with a
    /// pointer at `BenDecoder`. The bundle header decides the embedded BEN/XBEN format.
    #[new]
    #[pyo3(signature = (file_path))]
    #[pyo3(text_signature = "(file_path)")]
    fn new(py: Python<'_>, file_path: PathBuf) -> PyResult<Self> {
        let is_bundle = detect_is_bundle(&file_path).map_err(|e| {
            PyIOError::new_err(format!("Failed to open {}: {e}", file_path.display()))
        })?;
        if !is_bundle {
            return Err(PyException::new_err(format!(
                "{} is not a .bendl bundle (missing BENDL magic). Open plain BEN/XBEN \
                 streams with binary_ensemble.stream.BenDecoder instead.",
                file_path.display()
            )));
        }

        let file = File::open(&file_path).map_err(|e| {
            PyIOError::new_err(format!("Failed to open {}: {e}", file_path.display()))
        })?;
        let mut reader = BendlReader::open(BufReader::new(file)).map_err(|e| {
            PyException::new_err(format!(
                "Failed to parse bundle header in {}: {e}",
                file_path.display()
            ))
        })?;
        let fmt = reader.assignment_format().ok_or_else(|| {
            PyException::new_err("Bundle header has an unrecognized assignment_format field.")
        })?;
        let mode = DecoderMode::from_assignment_format(fmt);
        let (stream_offset, stream_len) = reader.assignment_stream_range().map_err(|e| {
            PyException::new_err(format!(
                "Failed to determine stream region in {}: {e}",
                file_path.display()
            ))
        })?;

        // Emit the XBEN startup warning once, up front.
        if matches!(mode, DecoderMode::XBen) {
            warn_xben_startup(py)?;
        }

        let header_sample_count = reader.sample_count();
        let empty = reader.is_finalized() && stream_len == 0;
        let source = StreamSource::Bundle {
            path: file_path.clone(),
            stream_offset,
            stream_len,
            header_sample_count,
            empty,
        };

        Ok(Self {
            path: file_path,
            reader,
            cursor: SampleCursor::new(source, mode),
        })
    }

    // -----------------------------------------------------------------
    // Iteration over the embedded stream.
    // -----------------------------------------------------------------

    fn __iter__(mut slf: PyRefMut<Self>) -> PyResult<Py<Self>> {
        slf.cursor.restart()?;
        Ok(slf.into())
    }

    fn __next__(&mut self) -> PyResult<Option<Vec<u16>>> {
        self.cursor.next()
    }

    fn __len__(&mut self, py: Python<'_>) -> PyResult<usize> {
        self.cursor.len(py)
    }

    #[pyo3(text_signature = "(self)")]
    fn count_samples(&mut self, py: Python<'_>) -> PyResult<usize> {
        self.cursor.count_samples(py)
    }

    #[pyo3(text_signature = "(self, indices, /)")]
    fn subsample_indices<'py>(
        mut slf: PyRefMut<'py, Self>,
        indices: Vec<usize>,
        py: Python<'_>,
    ) -> PyResult<Py<Self>> {
        slf.cursor.subsample_indices(indices, py)?;
        Ok(slf.into())
    }

    #[pyo3(text_signature = "(self, start, end, /)")]
    fn subsample_range<'py>(
        mut slf: PyRefMut<'py, Self>,
        start: usize,
        end: usize,
        py: Python<'_>,
    ) -> PyResult<Py<Self>> {
        slf.cursor.subsample_range(start, end, py)?;
        Ok(slf.into())
    }

    #[pyo3(signature = (step, offset=1))]
    fn subsample_every<'py>(
        mut slf: PyRefMut<'py, Self>,
        step: usize,
        offset: usize,
        py: Python<'_>,
    ) -> PyResult<Py<Self>> {
        slf.cursor.subsample_every(step, offset, py)?;
        Ok(slf.into())
    }

    // -----------------------------------------------------------------
    // Bundle inspection surface.
    // -----------------------------------------------------------------

    /// Return the container format of the embedded assignment stream as `"ben"` or `"xben"`.
    #[pyo3(text_signature = "(self)")]
    fn assignment_format(&self) -> &'static str {
        self.cursor.mode().as_str()
    }

    /// Return the bundle's format version as a `(major, minor)` tuple.
    #[pyo3(text_signature = "(self)")]
    fn version(&self) -> (u16, u16) {
        let h = self.reader.header();
        (h.major_version, h.minor_version)
    }

    /// Whether the bundle was successfully finalized.
    #[pyo3(text_signature = "(self)")]
    fn is_complete(&self) -> bool {
        self.reader.is_finalized()
    }

    /// Names of every entry in the bundle's directory, in directory order.
    #[pyo3(text_signature = "(self)")]
    fn asset_names(&self) -> Vec<String> {
        self.reader
            .assets()
            .iter()
            .map(|e| e.name.clone())
            .collect()
    }

    /// Return the full bundle directory as a list of dicts with keys `name`, `type`, `offset`,
    /// `len`, and `flags` (a list of string tags).
    #[pyo3(text_signature = "(self)")]
    fn list_assets<'py>(&self, py: Python<'py>) -> PyResult<Vec<Bound<'py, PyDict>>> {
        let entries = self.reader.assets();
        let mut out = Vec::with_capacity(entries.len());
        for entry in entries {
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

    /// Read the (decoded) bytes of a named asset as a Python `bytes` object.
    #[pyo3(text_signature = "(self, name, /)")]
    fn read_asset_bytes(&mut self, name: &str) -> PyResult<Vec<u8>> {
        let entry = self
            .reader
            .find_asset_by_name(name)
            .cloned()
            .ok_or_else(|| PyKeyError::new_err(format!("no asset named {name:?} in bundle")))?;
        self.reader
            .asset_bytes(&entry)
            .map_err(|e| PyIOError::new_err(format!("Failed to read asset {name:?}: {e}")))
    }

    /// Parse a JSON asset into a Python object (dict, list, …).
    #[pyo3(text_signature = "(self, name, /)")]
    fn read_json_asset<'py>(&mut self, py: Python<'py>, name: &str) -> PyResult<Py<PyAny>> {
        let bytes = self.read_asset_bytes(name)?;
        json_loads(py, &bytes, name)
    }

    /// Read the bundle's `graph.json` asset as a NetworkX graph, or `None` if absent.
    ///
    /// The stored adjacency-format JSON is rebuilt into a live graph via
    /// `networkx.readwrite.json_graph.adjacency_graph`, so its node order matches the order
    /// assignments were written in and it can be handed straight to consumers like GerryChain's
    /// `Partition`. The raw JSON is still available through `read_json_asset("graph.json")`.
    #[pyo3(text_signature = "(self)")]
    fn read_graph<'py>(&mut self, py: Python<'py>) -> PyResult<Option<Py<PyAny>>> {
        let Some(data) = self.read_known_json(py, ASSET_TYPE_GRAPH, "graph.json")? else {
            return Ok(None);
        };
        Ok(Some(crate::common::networkx_graph_from_json(
            py,
            data.bind(py),
        )?))
    }

    /// Read the bundle's `metadata.json` asset as parsed JSON, or `None` if absent.
    #[pyo3(text_signature = "(self)")]
    fn read_metadata<'py>(&mut self, py: Python<'py>) -> PyResult<Option<Py<PyAny>>> {
        self.read_known_json(py, ASSET_TYPE_METADATA, "metadata.json")
    }

    /// Read the bundle's `node_permutation_map.json` asset as parsed JSON, or `None` if absent.
    #[pyo3(text_signature = "(self)")]
    fn read_node_permutation_map<'py>(&mut self, py: Python<'py>) -> PyResult<Option<Py<PyAny>>> {
        self.read_known_json(
            py,
            ASSET_TYPE_NODE_PERMUTATION_MAP,
            "node_permutation_map.json",
        )
    }

    /// Copy the embedded assignment stream region verbatim to `out_path`. The resulting file can be
    /// opened directly with `BenDecoder(out_path, mode=dec.assignment_format())`.
    #[pyo3(signature = (out_path, overwrite=false, allow_unfinalized=false))]
    #[pyo3(text_signature = "(self, out_path, overwrite=False, allow_unfinalized=False)")]
    fn extract_stream(
        &mut self,
        out_path: PathBuf,
        overwrite: bool,
        allow_unfinalized: bool,
    ) -> PyResult<()> {
        if out_path.exists() && !overwrite {
            return Err(PyIOError::new_err(format!(
                "Output file {} already exists (use overwrite=True to replace).",
                out_path.display()
            )));
        }
        let mut stream = if allow_unfinalized && !self.reader.is_finalized() {
            self.reader
                .assignment_stream_reader_unverified()
                .map_err(|e| PyException::new_err(format!("Failed to open stream region: {e}")))?
        } else {
            self.reader
                .assignment_stream_reader()
                .map_err(|e| PyException::new_err(format!("Failed to open stream region: {e}")))?
        };

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
        .map_err(|e| PyIOError::new_err(format!("Failed to create {}: {e}", out_path.display())))?;
        let mut out = BufWriter::new(out);

        io::copy(&mut stream, &mut out)
            .map_err(|e| PyIOError::new_err(format!("Failed to copy stream bytes: {e}")))?;
        out.flush()
            .map_err(|e| PyIOError::new_err(format!("Failed to flush output: {e}")))?;
        Ok(())
    }

    fn __repr__(&self) -> String {
        let h = self.reader.header();
        format!(
            "BendlDecoder(path={:?}, format={:?}, complete={}, assets={})",
            self.path,
            self.cursor.mode().as_str(),
            h.is_finalized(),
            self.reader.assets().len(),
        )
    }
}

impl PyBendlDecoder {
    /// Read a known singleton asset by type, returning `None` when it is absent.
    fn read_known_json<'py>(
        &mut self,
        py: Python<'py>,
        asset_type: u16,
        name: &str,
    ) -> PyResult<Option<Py<PyAny>>> {
        if self.reader.find_asset_by_type(asset_type).is_none() {
            return Ok(None);
        }
        Ok(Some(self.read_json_asset(py, name)?))
    }
}

/// Parse JSON bytes into a Python object, with errors naming the asset.
fn json_loads(py: Python<'_>, bytes: &[u8], name: &str) -> PyResult<Py<PyAny>> {
    let json_mod = py.import("json")?;
    let text = std::str::from_utf8(bytes)
        .map_err(|e| PyException::new_err(format!("asset {name:?} is not valid UTF-8: {e}")))?;
    let parsed = json_mod.call_method1("loads", (text,))?;
    Ok(parsed.into())
}
