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

/// Reader and iterator for a ``.bendl`` bundle.
///
/// Iterate the decoder to yield the embedded assignment stream one plan at a time (each a
/// ``list[int]`` of district ids), and use ``len()`` for the sample count. Alongside the
/// stream, a bundle carries assets — the dual graph, metadata, a node permutation map, and any
/// custom blobs — exposed through the canonical getters (:meth:`read_graph`,
/// :meth:`read_metadata`, :meth:`read_node_permutation_map`) and the generic
/// :meth:`read_asset_bytes` / :meth:`read_json_asset`. Inspect the directory with
/// :meth:`asset_names`, :meth:`list_assets`, :meth:`version`, and :meth:`is_complete`.
///
/// This decoder is bundle-only: opening it on a plain ``.ben``/``.xben`` stream raises and
/// points the caller at :class:`~binary_ensemble.stream.BenDecoder`. A finalized assets-only
/// bundle (one written with no assignment stream) iterates to nothing with ``len() == 0``.
///
/// Example:
///     >>> from binary_ensemble import BendlDecoder
///     >>> dec = BendlDecoder("ensemble.bendl")
///     >>> graph = dec.read_graph()
///     >>> for assignment in dec:
///     ...     ...
#[pyclass(module = "binary_ensemble", name = "BendlDecoder", unsendable)]
pub struct PyBendlDecoder {
    path: PathBuf,
    reader: BendlReader<BufReader<File>>,
    cursor: SampleCursor,
}

#[pymethods]
impl PyBendlDecoder {
    /// Open a decoder on a ``.bendl`` bundle.
    ///
    /// The file's leading bytes are sniffed and a plain ``.ben``/``.xben`` stream is rejected.
    /// The bundle header decides whether the embedded stream is BEN or XBEN; an XBEN stream
    /// pays a one-time decompression startup cost.
    ///
    /// Args:
    ///     file_path: Path to the input ``.bendl`` file.
    ///
    /// Raises:
    ///     Exception: If ``file_path`` is not a bundle (use
    ///         :class:`~binary_ensemble.stream.BenDecoder` for plain streams), or its header
    ///         cannot be parsed.
    ///     OSError: If the file cannot be opened.
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

    /// Count the samples in the embedded stream.
    ///
    /// The result is the *expanded* sample count (a frame repeating five identical samples
    /// contributes five). It is computed lazily and cached, so repeated calls and ``len()``
    /// are cheap.
    ///
    /// Returns:
    ///     int: The number of samples in the bundle's stream.
    #[pyo3(text_signature = "(self)")]
    fn count_samples(&mut self, py: Python<'_>) -> PyResult<usize> {
        self.cursor.count_samples(py)
    }

    /// Restrict iteration to the samples at the given 1-indexed positions.
    ///
    /// Selected samples are reached by skipping frames rather than decoding the whole stream.
    ///
    /// Args:
    ///     indices: The 1-indexed sample numbers to keep.
    ///
    /// Returns:
    ///     BendlDecoder: ``self``, so the call can be chained into a ``for`` loop.
    #[pyo3(text_signature = "(self, indices, /)")]
    fn subsample_indices<'py>(
        mut slf: PyRefMut<'py, Self>,
        indices: Vec<usize>,
        py: Python<'_>,
    ) -> PyResult<Py<Self>> {
        slf.cursor.subsample_indices(indices, py)?;
        Ok(slf.into())
    }

    /// Restrict iteration to a contiguous, 1-indexed inclusive range of samples.
    ///
    /// Args:
    ///     start: First sample number to keep (1-indexed, inclusive).
    ///     end: Last sample number to keep (1-indexed, inclusive).
    ///
    /// Returns:
    ///     BendlDecoder: ``self``, for chaining into a ``for`` loop.
    ///
    /// Example:
    ///     >>> list(BendlDecoder("ensemble.bendl").subsample_range(10, 15))
    ///     # samples 10, 11, 12, 13, 14, and 15
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

    /// Restrict iteration to every ``step``-th sample.
    ///
    /// Args:
    ///     step: Stride between kept samples (e.g. ``10`` keeps every tenth sample).
    ///     offset: 1-indexed position of the first kept sample. Defaults to ``1``.
    ///
    /// Returns:
    ///     BendlDecoder: ``self``, for chaining into a ``for`` loop.
    #[pyo3(signature = (step, offset=1))]
    #[pyo3(text_signature = "(self, step, offset=1)")]
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

    /// Return the container format of the embedded assignment stream.
    ///
    /// Returns:
    ///     str: ``"ben"`` or ``"xben"``.
    #[pyo3(text_signature = "(self)")]
    fn assignment_format(&self) -> &'static str {
        self.cursor.mode().as_str()
    }

    /// Return the bundle's format version as a ``(major, minor)`` tuple.
    ///
    /// Returns:
    ///     tuple[int, int]: Bundle format version.
    #[pyo3(text_signature = "(self)")]
    fn version(&self) -> (u16, u16) {
        let h = self.reader.header();
        (h.major_version, h.minor_version)
    }

    /// Whether the bundle was successfully finalized.
    ///
    /// Returns:
    ///     bool: ``True`` for a complete bundle, ``False`` for a recoverable partial bundle.
    #[pyo3(text_signature = "(self)")]
    fn is_complete(&self) -> bool {
        self.reader.is_finalized()
    }

    /// Names of every entry in the bundle's directory, in directory order.
    ///
    /// Returns:
    ///     list[str]: Asset names such as ``"graph.json"`` and ``"metadata.json"``.
    #[pyo3(text_signature = "(self)")]
    fn asset_names(&self) -> Vec<String> {
        self.reader
            .assets()
            .iter()
            .map(|e| e.name.clone())
            .collect()
    }

    /// Return the full bundle directory.
    ///
    /// Returns:
    ///     list[dict]: Each dict has ``name``, ``type``, ``offset``, ``len``, and ``flags``.
    ///     ``flags`` is a list of string tags such as ``"json"``, ``"xz"``, and
    ///     ``"checksum"``.
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

    /// Read the (decoded) bytes of a named asset as a Python ``bytes`` object.
    ///
    /// Args:
    ///     name: The asset's name, as listed by :meth:`asset_names`.
    ///
    /// Returns:
    ///     bytes: The asset's decoded payload.
    ///
    /// Raises:
    ///     KeyError: If no asset with that name exists in the bundle.
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

    /// Parse a JSON asset into a Python object (``dict``, ``list``, …).
    ///
    /// Args:
    ///     name: The asset's name, as listed by :meth:`asset_names`.
    ///
    /// Returns:
    ///     The parsed JSON value.
    ///
    /// Raises:
    ///     KeyError: If no asset with that name exists in the bundle.
    ///     Exception: If the asset is not valid UTF-8 JSON.
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

    /// Copy the embedded assignment stream out to a standalone ``.ben``/``.xben`` file.
    ///
    /// The bytes are copied verbatim, so the result can be opened directly with
    /// ``BenDecoder(out_path, mode=dec.assignment_format())``.
    ///
    /// Args:
    ///     out_path: Path to write the extracted stream to.
    ///     overwrite: Replace ``out_path`` if it already exists. Defaults to ``False``.
    ///     allow_unfinalized: Permit extraction from a bundle that was never finalized
    ///         (recovering a partial stream). Defaults to ``False``.
    ///
    /// Raises:
    ///     OSError: If ``out_path`` exists and ``overwrite`` is ``False``, or the copy fails.
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
