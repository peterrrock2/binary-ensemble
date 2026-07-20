use super::cursor::SampleCursor;
use super::helpers::{detect_is_bundle, warn_xben_startup, FileIdentity};
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
/// stream, a bundle carries assets (the dual graph, metadata, a node permutation map, and any
/// custom blobs) exposed through the canonical getters (:meth:`read_graph`,
/// :meth:`read_metadata`, :meth:`read_node_permutation_map`) and the generic
/// :meth:`read_asset_bytes` / :meth:`read_json_asset`. Inspect the directory with
/// :meth:`asset_names`, :meth:`list_assets`, :meth:`version`, and :meth:`is_complete`.
///
/// A decoder is a snapshot of the file it opened: if the bundle changes on disk afterwards
/// (an in-place transform swaps in a rewritten file, or an append rewrites the directory),
/// every data-reading call refuses with a clear error rather than mixing old and new bytes;
/// open a fresh decoder to read the current file.
///
/// This decoder is bundle-only: opening it on a plain ``.ben``/``.xben`` stream raises and
/// points the caller at :class:`~binary_ensemble.stream.BenDecoder`. A finalized assets-only
/// bundle (one written with no assignment stream) iterates to nothing with ``len() == 0``.
///
/// Args:
///     file_path (StrPath): Path to the input ``.bendl`` file (``str`` or ``os.PathLike``).
///         Whether the embedded stream is BEN or XBEN is read from the bundle header; an XBEN
///         stream warns about a one-time decompression startup cost.
///
/// Raises:
///     Exception: If ``file_path`` is not a bundle (use
///         :class:`~binary_ensemble.stream.BenDecoder` for plain streams), or its header
///         cannot be parsed.
///     OSError: If the file cannot be opened.
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
    /// Identity of the file at open time; every IO entry point refuses if it changed.
    identity: FileIdentity,
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
    ///     file_path (StrPath): Path to the input ``.bendl`` file (``str`` or ``os.PathLike``).
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
                "{} is not a .bendl file (missing BENDL magic). Open plain BEN/XBEN \
                 streams with binary_ensemble.stream.BenDecoder instead.",
                file_path.display()
            )));
        }

        let file = File::open(&file_path).map_err(|e| {
            PyIOError::new_err(format!("Failed to open {}: {e}", file_path.display()))
        })?;
        let identity = FileIdentity::of_file(&file).map_err(|e| {
            PyIOError::new_err(format!("Failed to stat {}: {e}", file_path.display()))
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
            identity,
            stream_offset,
            stream_len,
            header_sample_count,
            empty,
        };

        Ok(Self {
            path: file_path,
            identity,
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
    /// contributes five). On a finalized bundle the count is read from the bundle header,
    /// so it never requires scanning the stream; it is cached either way, so repeated
    /// calls and ``len()`` are cheap.
    ///
    /// Returns:
    ///     int: The number of samples in the bundle's stream.
    #[pyo3(text_signature = "(self)")]
    fn count_samples(&mut self, py: Python<'_>) -> PyResult<usize> {
        self.cursor.count_samples(py)
    }

    /// Return the assignment at a zero-based sample index.
    ///
    /// Lookup uses a separate reader, so it does not change the decoder's current iteration
    /// position or any selection installed with a ``subsample_*`` method.
    ///
    /// Args:
    ///     index (int): Zero-based sample index to return.
    ///
    /// Returns:
    ///     list[int]: The assignment at ``index``.
    ///
    /// Raises:
    ///     IndexError: If ``index`` is outside the stream.
    #[pyo3(text_signature = "(self, index)")]
    fn lookup(&self, index: isize, py: Python<'_>) -> PyResult<Vec<u16>> {
        self.cursor.lookup(index, py)
    }

    /// Restrict iteration to the samples at the given zero-based indices.
    ///
    /// Skipped samples are never materialized as Python lists, and where the encoding
    /// variant allows it (``standard``, ``mkv_chain``) whole frames are skipped without
    /// being unpacked.
    ///
    /// Args:
    ///     indices (Sequence[int]): The zero-based indices to keep. Duplicates are
    ///         dropped; an unsorted list is sorted, with a ``UserWarning``.
    ///
    /// Returns:
    ///     BendlDecoder: ``self``, so the call can be chained into a ``for`` loop.
    ///
    /// Raises:
    ///     Exception: If ``indices`` is empty or contains an index greater than or equal to the
    ///         number of samples in the stream.
    #[pyo3(text_signature = "(self, indices)")]
    fn subsample_indices<'py>(
        mut slf: PyRefMut<'py, Self>,
        indices: Vec<usize>,
        py: Python<'_>,
    ) -> PyResult<Py<Self>> {
        slf.cursor.subsample_indices(indices, py)?;
        Ok(slf.into())
    }

    /// Restrict iteration to a contiguous, zero-based half-open range of samples.
    ///
    /// Args:
    ///     start (int): First index to keep (inclusive).
    ///     end (int): Index at which to stop (exclusive).
    ///
    /// Returns:
    ///     BendlDecoder: ``self``, for chaining into a ``for`` loop.
    ///
    /// Raises:
    ///     Exception: If ``end`` is less than ``start`` or greater than the number of samples in
    ///         the stream.
    ///
    /// Example:
    ///     >>> list(BendlDecoder("ensemble.bendl").subsample_range(10, 15))
    ///     # indices 10, 11, 12, 13, and 14
    #[pyo3(text_signature = "(self, start, end)")]
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
    ///     step (int): Stride between kept samples (e.g. ``10`` keeps every tenth sample).
    ///     offset (int, optional): Zero-based index of the first kept sample. Default is ``0``.
    ///
    /// Returns:
    ///     BendlDecoder: ``self``, for chaining into a ``for`` loop.
    ///
    /// Raises:
    ///     Exception: If ``step`` is ``0`` or ``offset`` is outside the stream.
    #[pyo3(signature = (step, offset=0))]
    #[pyo3(text_signature = "(self, step, offset=0)")]
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

    /// Return the on-disk byte length of the embedded assignment stream.
    ///
    /// Read straight from the bundle header's ``stream_len`` field; no decoding or copying.
    /// This is the size of the stream region as stored (BEN bytes, or compressed XBEN bytes),
    /// the same bytes ``extract_stream`` would copy out. For an unfinalized bundle the stream
    /// is taken to extend to the directory (or EOF), matching recovery extraction.
    ///
    /// Returns:
    ///     int: Byte length of the embedded stream region; ``0`` for an assets-only bundle.
    ///
    /// Example:
    ///     >>> BendlDecoder("ensemble.bendl").stream_size()
    ///     40110
    #[pyo3(text_signature = "(self)")]
    fn stream_size(&mut self) -> PyResult<u64> {
        let (_offset, len) = self
            .reader
            .assignment_stream_range()
            .map_err(|e| PyIOError::new_err(format!("Failed to read stream range: {e}")))?;
        Ok(len)
    }

    /// Return the on-disk byte length of a named asset's stored payload.
    ///
    /// Read straight from the bundle directory; no decoding or copying. For assets stored
    /// xz-compressed (the ``"xz"`` flag in :meth:`list_assets`), this is the compressed size;
    /// the decoded payload can be larger, so use ``len(read_asset_bytes(name))`` for that.
    ///
    /// Args:
    ///     name (str): The asset's name, as listed by :meth:`asset_names`.
    ///
    /// Returns:
    ///     int: Stored byte length of the asset's payload region.
    ///
    /// Raises:
    ///     KeyError: If no asset with that name exists in the bundle.
    #[pyo3(text_signature = "(self, name)")]
    fn asset_size(&self, name: &str) -> PyResult<u64> {
        let entry = self
            .reader
            .find_asset_by_name(name)
            .ok_or_else(|| PyKeyError::new_err(format!("no asset named {name:?} in bundle")))?;
        Ok(entry.payload_len)
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

    /// Verify the bundle's integrity: asset and stream checksums, plus the header sample count.
    ///
    /// Scans the raw on-disk bytes of every asset and of the assignment stream and compares them
    /// against the CRC32C checksums recorded when the bundle was written, then walks the stream's
    /// frame boundaries to confirm the decoded sample count matches the (unchecksummed) header
    /// `sample_count`. Iterating or subsampling a decoder reads the stream *without* checking the
    /// checksums (partial reads cannot prove a whole-stream checksum) and trusts the header count
    /// for finalized bundles, so call this when integrity matters, e.g. after downloading a bundle
    /// or before an important run.
    ///
    /// Raises:
    ///     Exception: If any asset checksum or the stream checksum does not match the on-disk
    ///         bytes, if the header sample_count disagrees with the decoded stream, or if the
    ///         bundle is unfinalized (an unfinalized bundle's stream checksum and sample count are
    ///         not authoritative).
    ///
    /// Example:
    ///     >>> dec = BendlDecoder("ensemble.bendl")
    ///     >>> dec.verify()  # raises on any corruption
    #[pyo3(text_signature = "(self)")]
    fn verify(&mut self, py: Python<'_>) -> PyResult<()> {
        self.identity
            .ensure_unchanged(&self.path, "verify the bundle")?;
        // The CRC scan is Rust-only IO; run it detached so a whole-file verify doesn't block
        // other Python threads (or KeyboardInterrupt delivery) for its duration.
        let reader = &mut self.reader;
        py.detach(move || {
            reader.verify_all_asset_checksums().map_err(|e| {
                PyException::new_err(format!("Bundle asset verification failed: {e}"))
            })?;
            reader.verify_stream_checksum().map_err(|e| {
                PyException::new_err(format!("Bundle stream verification failed: {e}"))
            })?;
            // The header CRC proves the header bytes are intact, but not that sample_count matches
            // the actual stream content. So cross-check sample_count against the decoded stream,
            // which count_samples()/len()/subsample bounds trust for finalized bundles.
            reader.verify_sample_count().map_err(|e| {
                PyException::new_err(format!("Bundle sample-count verification failed: {e}"))
            })?;
            Ok(())
        })
    }

    /// Read the (decoded) bytes of a named asset as a Python ``bytes`` object.
    ///
    /// Args:
    ///     name (str): The asset's name, as listed by :meth:`asset_names`.
    ///
    /// Returns:
    ///     bytes: The asset's decoded payload.
    ///
    /// Raises:
    ///     KeyError: If no asset with that name exists in the bundle.
    #[pyo3(text_signature = "(self, name)")]
    fn read_asset_bytes(&mut self, name: &str) -> PyResult<Vec<u8>> {
        self.identity
            .ensure_unchanged(&self.path, "read an asset")?;
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
    ///     name (str): The asset's name, as listed by :meth:`asset_names`.
    ///
    /// Returns:
    ///     The parsed JSON value.
    ///
    /// Raises:
    ///     KeyError: If no asset with that name exists in the bundle.
    ///     Exception: If the asset is not valid UTF-8 JSON.
    #[pyo3(text_signature = "(self, name)")]
    fn read_json_asset<'py>(&mut self, py: Python<'py>, name: &str) -> PyResult<Py<PyAny>> {
        let bytes = self.read_asset_bytes(name)?;
        json_loads(py, &bytes, name)
    }

    /// Read the bundle's `graph.json` asset as a NetworkX graph, or `None` if absent.
    ///
    /// The stored adjacency-format JSON is rebuilt into a live graph via
    /// `networkx.readwrite.json_graph.adjacency_graph`, so its node order matches the order
    /// assignments were written in and it can be handed straight to consumers like GerryChain's
    /// `Partition`. The result is a :class:`networkx.Graph`, or a
    /// :class:`networkx.MultiGraph` if the stored adjacency declares itself a multigraph.
    /// The raw JSON is still available through `read_json_asset("graph.json")`.
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
    ///     out_path (StrPath): Path to write the extracted stream to (``str`` or
    ///         ``os.PathLike``).
    ///     overwrite (bool, optional): Replace ``out_path`` if it already exists. Default is
    ///         ``False``.
    ///     allow_unfinalized (bool, optional): Permit extraction from a bundle that was never
    ///         finalized (recovering a partial stream). Default is ``False``.
    ///
    /// Raises:
    ///     OSError: If ``out_path`` exists and ``overwrite`` is ``False``, or the copy fails.
    #[pyo3(signature = (out_path, overwrite=false, allow_unfinalized=false))]
    #[pyo3(text_signature = "(self, out_path, overwrite=False, allow_unfinalized=False)")]
    fn extract_stream(
        &mut self,
        py: Python<'_>,
        out_path: PathBuf,
        overwrite: bool,
        allow_unfinalized: bool,
    ) -> PyResult<()> {
        self.identity
            .ensure_unchanged(&self.path, "extract the stream")?;
        if out_path.exists() && !overwrite {
            return Err(PyIOError::new_err(format!(
                "Output file {} already exists (use overwrite=True to replace).",
                out_path.display()
            )));
        }
        // The copy is Rust-only IO; run it detached. The boxed stream reader is created inside
        // the closure (locals need not be Send, only captures do).
        let reader = &mut self.reader;
        py.detach(move || {
            let mut stream = if allow_unfinalized && !reader.is_finalized() {
                reader.assignment_stream_reader_unverified().map_err(|e| {
                    PyException::new_err(format!("Failed to open stream region: {e}"))
                })?
            } else {
                reader.assignment_stream_reader().map_err(|e| {
                    PyException::new_err(format!("Failed to open stream region: {e}"))
                })?
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
            .map_err(|e| {
                PyIOError::new_err(format!("Failed to create {}: {e}", out_path.display()))
            })?;
            let mut out = BufWriter::new(out);

            io::copy(&mut stream, &mut out)
                .map_err(|e| PyIOError::new_err(format!("Failed to copy stream bytes: {e}")))?;
            out.flush()
                .map_err(|e| PyIOError::new_err(format!("Failed to flush output: {e}")))?;
            Ok(())
        })
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
