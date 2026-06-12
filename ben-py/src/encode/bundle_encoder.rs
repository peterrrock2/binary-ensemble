//! `.bendl` file authoring bindings: [`PyBendlEncoder`] and its [`PyBendlStreamSession`].
//!
//! The encoder threads the bundle through the library's typestate machinery — `BendlWriter`
//! (assets) → `BendlStreamSession` (stream) → `BendlWriter::finish` (finalize) — for the create
//! path, and reopens a `BendlAppender` per asset for post-stream / append-mode adds. The state enum
//! below tracks which phase the encoder is in so a second `stream()` is refused and so `add_*`
//! routes through the writer pre-stream and the appender afterwards.

use crate::common::{
    graph_node_count, networkx_graph_from_bytes, open_output, parse_graph_input,
    parse_metadata_input, parse_variant,
};
use crate::graph::helpers::{reorder_graph_to_bytes, resolve_reorder};
use binary_ensemble::io::bundle::format::{AssignmentFormat, KnownAssetKind};
use binary_ensemble::io::bundle::writer::BendlAppender;
use binary_ensemble::io::bundle::{
    AddAssetOptions, BendlStreamSession, BendlWriteError, BendlWriter,
};
use binary_ensemble::io::writer::BenStreamWriter;
use pyo3::exceptions::{PyException, PyIOError, PyKeyError, PyValueError};
use pyo3::prelude::*;
use std::fs::{File, OpenOptions};
use std::io::{self, BufWriter};
use std::path::PathBuf;

fn map_bundle_err(err: BendlWriteError) -> PyErr {
    match err {
        BendlWriteError::Io(e) => PyIOError::new_err(format!("{e}")),
        // Matches the decoder's lookup errors (read_asset_bytes, asset_size).
        BendlWriteError::UnknownAssetName(name) => {
            PyKeyError::new_err(format!("no asset named {name:?} in bundle"))
        }
        other => PyException::new_err(format!("{other}")),
    }
}

fn map_io_err(err: io::Error) -> PyErr {
    PyIOError::new_err(format!("{err}"))
}

/// Map a `content_type` string to writer options. `"json"` sets the JSON flag so the decoder can
/// auto-parse; `"text"` stores without it.
fn opts_for(content_type: &str) -> PyResult<AddAssetOptions> {
    match content_type {
        "json" => Ok(AddAssetOptions::defaults().json()),
        // "text" and "binary" carry the same wire options (raw bytes, default compression
        // policy); the two names exist so call sites document their payloads honestly.
        "text" | "binary" => Ok(AddAssetOptions::defaults()),
        other => Err(PyValueError::new_err(format!(
            "content_type must be 'json', 'text', or 'binary', got {other:?}"
        ))),
    }
}

/// Phase of a [`PyBendlEncoder`].
enum BundleState {
    /// Create mode, before the stream: the writer owns the file and accepts asset writes.
    PreStream {
        writer: Box<BendlWriter<BufWriter<File>>>,
        /// Node count of a pre-stream graph, used to validate each written assignment.
        graph_node_count: Option<usize>,
    },
    /// A stream session is open; the writer has been moved into the session object. The session
    /// signals back via [`PyBendlEncoder::mark_finalized`] / [`PyBendlEncoder::mark_failed`].
    Streaming,
    /// The bundle is finalized on disk: post-stream create mode, or append mode. `add_*` reopen a
    /// `BendlAppender` on the path and commit immediately.
    Appendable,
    /// The stream session exited via an exception; the bundle is unfinalized on disk and `close()`
    /// must not finalize over the truncated stream.
    Failed,
    /// The encoder has been closed.
    Closed,
}

/// Writer for a single `.bendl` file.
#[pyclass(module = "binary_ensemble", name = "BendlEncoder", unsendable)]
pub struct PyBendlEncoder {
    path: PathBuf,
    append_mode: bool,
    state: BundleState,
}

#[pymethods]
impl PyBendlEncoder {
    /// Open a new bundle writer in create mode.
    ///
    /// A create-mode encoder writes one `.bendl` file. Add graph and metadata assets, then
    /// open exactly one assignment stream with :meth:`stream`. The stream context finalizes the
    /// bundle on a clean close.
    ///
    /// Args:
    ///     file_path (StrPath): Output path (``str`` or ``os.PathLike``). Must not exist
    ///         unless ``overwrite=True``.
    ///     overwrite (bool, optional): Replace an existing file at ``file_path``. Default is
    ///         ``False``.
    ///
    /// Raises:
    ///     OSError: If ``file_path`` exists and ``overwrite`` is ``False``, or it cannot be
    ///         created.
    ///
    /// Example:
    ///     >>> from binary_ensemble import BendlEncoder
    ///     >>> encoder = BendlEncoder("ensemble.bendl", overwrite=True)
    ///     >>> with encoder.stream() as stream:
    ///     ...     stream.write([1, 1, 2, 2])
    #[new]
    #[pyo3(signature = (file_path, overwrite = false))]
    #[pyo3(text_signature = "(file_path, overwrite=False)")]
    fn new(file_path: PathBuf, overwrite: bool) -> PyResult<Self> {
        let buf = open_output(&file_path, overwrite)?;
        let writer = BendlWriter::new(buf, AssignmentFormat::Ben).map_err(map_io_err)?;
        Ok(Self {
            path: file_path,
            append_mode: false,
            state: BundleState::PreStream {
                writer: Box::new(writer),
                graph_node_count: None,
            },
        })
    }

    /// Open an existing finalized bundle for append.
    ///
    /// Append mode is for assets only. ``stream()`` is unavailable because a bundle has exactly
    /// one assignment stream. Each ``add_*`` operation commits immediately.
    ///
    /// Args:
    ///     file_path (StrPath): Existing finalized ``.bendl`` bundle (``str`` or
    ///         ``os.PathLike``).
    ///
    /// Returns:
    ///     BendlEncoder: An encoder in append mode.
    ///
    /// Raises:
    ///     OSError: If the bundle cannot be opened for append.
    ///     Exception: If the file is not a finalized bundle.
    ///
    /// Example:
    ///     >>> encoder = BendlEncoder.append("ensemble.bendl")
    ///     >>> encoder.add_asset("notes.txt", "reviewed", content_type="text")
    ///     >>> encoder.close()
    #[staticmethod]
    #[pyo3(signature = (file_path))]
    #[pyo3(text_signature = "(file_path)")]
    fn append(file_path: PathBuf) -> PyResult<Self> {
        // Validate the target is a finalized bundle up front by opening (and discarding) an
        // appender. The actual asset adds reopen their own appender per call.
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&file_path)
            .map_err(|e| {
                PyIOError::new_err(format!(
                    "Failed to open {} for append: {e}",
                    file_path.display()
                ))
            })?;
        let appender = BendlAppender::open(file).map_err(map_bundle_err)?;
        appender.abort();
        Ok(Self {
            path: file_path,
            append_mode: true,
            state: BundleState::Appendable,
        })
    }

    /// Add a custom asset (asset type ``CUSTOM``).
    ///
    /// Payloads are stored verbatim with a CRC32C integrity checksum, so any bytes round-trip —
    /// including binary blobs such as zipped shapefiles or GeoPackages. Payloads at or above 1
    /// KiB are xz-compressed on disk by default (transparent on read); already-compressed blobs
    /// gain little from this but are not harmed by it.
    ///
    /// Args:
    ///     name (str): Asset name stored in the bundle directory.
    ///     payload (bytes): The bytes to store. (The :class:`binary_ensemble.bundle.BendlEncoder`
    ///         facade accepts richer payload shapes and coerces them to bytes.)
    ///     content_type (str): ``"json"``, ``"text"``, or ``"binary"``. JSON assets are marked so
    ///         :meth:`binary_ensemble.bundle.BendlDecoder.read_json_asset` can parse them;
    ///         ``"text"`` and ``"binary"`` store the bytes unmarked.
    ///
    /// Raises:
    ///     ValueError: If ``content_type`` is not ``"json"``, ``"text"``, or ``"binary"``.
    ///     Exception: If the encoder is closed, failed, or currently streaming.
    ///
    /// Example:
    ///     >>> encoder.add_asset("scores.json", '{"cut_edges": [10]}', content_type="json")
    ///     >>> encoder.add_asset("tracts.gpkg", gpkg_bytes, content_type="binary")
    #[pyo3(signature = (name, payload, content_type))]
    #[pyo3(text_signature = "(self, name, payload, content_type)")]
    fn add_asset(&mut self, name: &str, payload: Vec<u8>, content_type: &str) -> PyResult<()> {
        let opts = opts_for(content_type)?;
        if let BundleState::PreStream { writer, .. } = &mut self.state {
            return writer
                .add_custom_asset(name, &payload, opts)
                .map_err(map_bundle_err);
        }
        if matches!(self.state, BundleState::Appendable) {
            return self.append_commit(|a| a.add_custom_asset(name, &payload, opts));
        }
        Err(state_error(&self.state, "add_asset"))
    }

    /// Remove a named asset from a finalized bundle's directory.
    ///
    /// Available wherever ``add_asset`` commits immediately: append mode, or create mode after
    /// the stream has closed. Only the directory entry is dropped — the payload bytes remain in
    /// the file as unreferenced dead space until the next whole-bundle rewrite (e.g.
    /// :func:`binary_ensemble.bundle.compress_stream` or
    /// :func:`binary_ensemble.bundle.relabel_bundle`) compacts them. The name (and any
    /// singleton-type claim, e.g. ``metadata.json``) becomes free again, so remove-then-add is
    /// the way to replace an asset's payload.
    ///
    /// Args:
    ///     name (str): The asset's name, as listed by
    ///         :meth:`binary_ensemble.bundle.BendlDecoder.asset_names`.
    ///
    /// Raises:
    ///     KeyError: If no asset with that name exists in the bundle.
    ///     Exception: If the encoder is in create mode before the stream (just don't add the
    ///         asset), is currently streaming, or is closed.
    ///
    /// Example:
    ///     >>> appender = BendlEncoder.append("ensemble.bendl")
    ///     >>> appender.remove_asset("notes.txt")
    #[pyo3(signature = (name))]
    #[pyo3(text_signature = "(self, name)")]
    fn remove_asset(&mut self, name: &str) -> PyResult<()> {
        if matches!(self.state, BundleState::Appendable) {
            return self.append_commit(|a| a.remove_asset(name));
        }
        Err(state_error(&self.state, "remove_asset"))
    }

    /// Add the canonical ``metadata.json`` known asset.
    ///
    /// ``metadata`` accepts a Python ``dict``/``list``, UTF-8 JSON bytes, a file-like object with
    /// ``.read()``, or a path to JSON. The decoder returns it with :meth:`read_metadata`.
    ///
    /// Args:
    ///     metadata (MetadataInput): The JSON payload: a ``dict``/``list``, UTF-8 JSON
    ///         ``bytes``, a file-like object with ``.read()``, or a ``str``/``os.PathLike``
    ///         path to a JSON file (a plain ``str`` is a *path* here).
    ///
    /// Raises:
    ///     Exception: If the metadata cannot be converted to JSON bytes, or if the encoder is in
    ///         an invalid state.
    ///
    /// Example:
    ///     >>> encoder.add_metadata({"sampler": "ReCom", "seed": 1234})
    #[pyo3(signature = (metadata))]
    #[pyo3(text_signature = "(self, metadata)")]
    fn add_metadata(&mut self, py: Python<'_>, metadata: Bound<'_, PyAny>) -> PyResult<()> {
        let bytes = parse_metadata_input(py, &metadata)?;
        let opts = AddAssetOptions::defaults().json();
        if let BundleState::PreStream { writer, .. } = &mut self.state {
            return writer
                .add_known_asset(KnownAssetKind::Metadata, &bytes, opts)
                .map_err(map_bundle_err);
        }
        if matches!(self.state, BundleState::Appendable) {
            return self
                .append_commit(|a| a.add_known_asset(KnownAssetKind::Metadata, &bytes, opts));
        }
        Err(state_error(&self.state, "add_metadata"))
    }

    /// Add the ``graph.json`` known asset and return the graph to use for assignments.
    ///
    /// `sort` defaults to `"mlc"`, so by default the graph is reordered for better compression.
    /// `sort` is `"mlc"` (multi-level clustering), `"rcm"` (reverse Cuthill-McKee), `"key"` to sort
    /// by a node attribute named via `key` (e.g. `key="GEOID"`), or `None` to store the graph
    /// as-is. When reordering, both `graph.json` and `node_permutation_map.json` are stored,
    /// and the reordered graph is returned (as a NetworkX graph, matching
    /// `BendlDecoder.read_graph`) so the chain runs on that ordering. Reordering is pre-stream
    /// only; a raw graph (`sort=None`) may also be attached post-stream / in append mode. The
    /// returned graph's node count is recorded for per-write validation.
    ///
    /// Args:
    ///     graph (GraphInput): The dual graph: a live ``networkx.Graph`` (subclasses such as
    ///         ``gerrychain.Graph`` count), or NetworkX adjacency JSON as a ``dict``/``list``,
    ///         raw JSON ``bytes``, a file-like object with ``.read()``, or a
    ///         ``str``/``os.PathLike`` path to a JSON file (a plain ``str`` is a *path* here).
    ///     sort (SortMethod | None, optional): ``"mlc"``, ``"rcm"``, ``"key"``, or ``None``
    ///         to store the graph as-is. Default is ``"mlc"``.
    ///     key (str | None, optional): Node attribute used when ``sort="key"``. Use ``"id"``
    ///         for node id ordering. Default is ``None``.
    ///
    /// Returns:
    ///     networkx.Graph: The stored graph, after any reordering.
    ///
    /// Raises:
    ///     ValueError: If ``sort``/``key`` is invalid.
    ///     Exception: If a reordering graph is added after the stream has started.
    ///
    /// Example:
    ///     >>> stored_graph = encoder.add_graph("graph.json", sort="mlc")
    ///     >>> write_order = list(stored_graph.nodes)
    #[pyo3(signature = (graph, sort = Some("mlc".to_string()), key = None))]
    #[pyo3(text_signature = "(self, graph, sort='mlc', key=None)")]
    fn add_graph(
        &mut self,
        py: Python<'_>,
        graph: Bound<'_, PyAny>,
        sort: Option<String>,
        key: Option<String>,
    ) -> PyResult<Py<PyAny>> {
        let plan = resolve_reorder(sort.as_deref(), key.as_deref())?;
        let graph_bytes = parse_graph_input(py, &graph)?;
        let opts = AddAssetOptions::defaults().json();

        if let Some(plan) = plan {
            // Reordering rewrites the node ordering the chain must write in, so it is pre-stream
            // only.
            if !matches!(self.state, BundleState::PreStream { .. }) {
                return Err(PyException::new_err(
                    "a reordering add_graph (sort != None) is only allowed before stream(); \
                     post-stream or append-mode graphs must use sort=None",
                ));
            }
            let (reordered, map) = reorder_graph_to_bytes(&graph_bytes, &plan)?;
            let count = graph_node_count(&reordered)?;
            if let BundleState::PreStream {
                writer,
                graph_node_count: gnc,
            } = &mut self.state
            {
                writer
                    .add_known_asset(KnownAssetKind::Graph, &reordered, opts.clone())
                    .map_err(map_bundle_err)?;
                writer
                    .add_known_asset(KnownAssetKind::NodePermutationMap, &map, opts)
                    .map_err(map_bundle_err)?;
                *gnc = Some(count);
            }
            return networkx_graph_from_bytes(py, &reordered);
        }

        // Raw graph: stored as-is, no permutation map.
        let count = graph_node_count(&graph_bytes)?;
        if let BundleState::PreStream {
            writer,
            graph_node_count: gnc,
        } = &mut self.state
        {
            writer
                .add_known_asset(KnownAssetKind::Graph, &graph_bytes, opts)
                .map_err(map_bundle_err)?;
            *gnc = Some(count);
            return networkx_graph_from_bytes(py, &graph_bytes);
        }
        if matches!(self.state, BundleState::Appendable) {
            self.append_commit(|a| a.add_known_asset(KnownAssetKind::Graph, &graph_bytes, opts))?;
            return networkx_graph_from_bytes(py, &graph_bytes);
        }
        Err(state_error(&self.state, "add_graph"))
    }

    /// Open the single-use assignment stream.
    ///
    /// The embedded stream is always written in the BEN wire format; XBEN bundles are produced
    /// by :func:`binary_ensemble.bundle.compress_stream` after writing (XBEN is a whole-stream
    /// LZMA2 wrap, so it cannot be written live sample-by-sample without forfeiting its
    /// compression).
    ///
    /// Args:
    ///     variant (Variant, optional): BEN encoding variant — ``"standard"``,
    ///         ``"mkv_chain"``, or ``"twodelta"``. Default is ``"twodelta"``.
    ///
    /// Returns:
    ///     BendlStreamSession: Context manager whose :meth:`write` method accepts assignments.
    ///
    /// Raises:
    ///     ValueError: If ``variant`` is invalid.
    ///     Exception: If a stream has already been written, append mode is active, or the encoder
    ///         is closed/failed.
    ///
    /// Example:
    ///     >>> with encoder.stream(variant="standard") as stream:
    ///     ...     stream.write([1, 1, 2, 2])
    #[pyo3(signature = (*, variant = "twodelta"))]
    #[pyo3(text_signature = "(self, *, variant='twodelta')")]
    fn stream(slf: Bound<'_, Self>, variant: &str) -> PyResult<PyBendlStreamSession> {
        let ben_var = parse_variant(Some(variant))?;

        let encoder_handle: Py<PyBendlEncoder> = slf.clone().unbind();
        let mut me = slf.borrow_mut();

        if me.append_mode {
            return Err(PyException::new_err(
                "stream() is unavailable in append mode; open a fresh BendlEncoder to write a \
                 new stream",
            ));
        }
        match &me.state {
            BundleState::PreStream { .. } => {}
            BundleState::Streaming => return Err(PyException::new_err("a stream is already open")),
            BundleState::Appendable => {
                return Err(PyException::new_err(
                    "a stream has already been written to this bundle",
                ))
            }
            BundleState::Failed => {
                return Err(PyException::new_err(
                    "the previous stream failed; this bundle is unfinalized",
                ))
            }
            BundleState::Closed => return Err(PyException::new_err("encoder is closed")),
        }

        let prev = std::mem::replace(&mut me.state, BundleState::Streaming);
        let BundleState::PreStream {
            writer,
            graph_node_count,
        } = prev
        else {
            unreachable!("validated PreStream above")
        };

        let build = (|| {
            let session = writer.into_stream_session().map_err(map_bundle_err)?;
            let ben_writer = BenStreamWriter::for_ben(session, ben_var).map_err(map_io_err)?;
            Ok::<_, PyErr>(ben_writer)
        })();

        match build {
            Ok(ben_writer) => Ok(PyBendlStreamSession {
                writer: Some(Box::new(ben_writer)),
                sample_count: 0,
                graph_node_count,
                encoder: encoder_handle,
            }),
            Err(e) => {
                me.state = BundleState::Failed;
                Err(e)
            }
        }
    }

    /// Finalize or close the bundle. Idempotent.
    ///
    /// In create mode, closing before any stream creates a finalized assets-only bundle. The
    /// stream context normally finalizes the bundle for you. After a failed stream, ``close()``
    /// does not stamp the partial bundle as complete. In append mode, asset writes have already
    /// committed and ``close()`` is a no-op.
    fn close(&mut self) -> PyResult<()> {
        match &self.state {
            // The session owns the writer and finalizes on its own close.
            BundleState::Streaming => Ok(()),
            BundleState::Appendable | BundleState::Failed | BundleState::Closed => {
                self.state = BundleState::Closed;
                Ok(())
            }
            BundleState::PreStream { .. } => {
                let prev = std::mem::replace(&mut self.state, BundleState::Closed);
                if let BundleState::PreStream { writer, .. } = prev {
                    writer.finish().map_err(map_bundle_err)?;
                }
                Ok(())
            }
        }
    }

    fn __enter__(slf: PyRefMut<Self>) -> PyRefMut<Self> {
        slf
    }

    fn __exit__(
        &mut self,
        _exc_type: Option<&Bound<'_, PyAny>>,
        _exc_value: Option<&Bound<'_, PyAny>>,
        _traceback: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<bool> {
        self.close()?;
        Ok(false)
    }
}

impl PyBendlEncoder {
    /// Mark the bundle finalized after a successful stream session close, so subsequent `add_*` go
    /// through the appender.
    pub(crate) fn mark_finalized(&mut self) {
        self.state = BundleState::Appendable;
    }

    /// Mark the bundle failed after the stream session exited via an exception.
    pub(crate) fn mark_failed(&mut self) {
        self.state = BundleState::Failed;
    }

    /// Open a fresh appender on the bundle path, run one asset operation, and commit it.
    fn append_commit<F>(&self, op: F) -> PyResult<()>
    where
        F: FnOnce(&mut BendlAppender<File>) -> Result<(), BendlWriteError>,
    {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&self.path)
            .map_err(|e| {
                PyIOError::new_err(format!(
                    "Failed to open {} for append: {e}",
                    self.path.display()
                ))
            })?;
        let mut appender = BendlAppender::open(file).map_err(map_bundle_err)?;
        op(&mut appender).map_err(map_bundle_err)?;
        appender.commit().map_err(map_bundle_err)?;
        Ok(())
    }
}

/// Build a clear error for an operation attempted in an invalid encoder state.
fn state_error(state: &BundleState, op: &str) -> PyErr {
    let reason = match state {
        BundleState::Streaming => "the assignment stream is open; close it before adding assets",
        BundleState::Failed => "the previous stream failed; this bundle is unfinalized",
        BundleState::Closed => "the encoder is closed",
        BundleState::PreStream { .. } => "the bundle is not finalized yet",
        BundleState::Appendable => "invalid state",
    };
    PyException::new_err(format!("cannot {op}: {reason}"))
}

/// Single-use context manager over a bundle's assignment stream.
///
/// Obtained from :meth:`binary_ensemble.bundle.BendlEncoder.stream`; you don't construct it
/// directly. Write assignments with :meth:`write` inside a ``with`` block. Closing the context
/// cleanly **finalizes** the bundle; if the block exits via an exception the bundle is left
/// unfinalized (recoverable, rather than stamped complete over a truncated stream).
#[pyclass(module = "binary_ensemble", name = "BendlStreamSession", unsendable)]
pub struct PyBendlStreamSession {
    writer: Option<Box<BenStreamWriter<BendlStreamSession<BufWriter<File>>>>>,
    sample_count: i64,
    /// Node count of a pre-stream graph, used to validate each written assignment.
    graph_node_count: Option<usize>,
    encoder: Py<PyBendlEncoder>,
}

#[pymethods]
impl PyBendlStreamSession {
    /// Encode a single assignment into the bundle's stream.
    ///
    /// Args:
    ///     assignment (Sequence[int]): The plan as a sequence of district ids (e.g. a
    ///         ``list[int]``), one per node in dual-graph node order.
    ///
    /// Returns:
    ///     None.
    ///
    /// Raises:
    ///     ValueError: If the bundle carries a pre-stream graph and the assignment length does
    ///         not equal the graph's node count.
    ///     OSError: If the session is already closed, or the write fails.
    ///
    /// Example:
    ///     >>> stream.write([1, 1, 2, 2])
    #[pyo3(signature = (assignment))]
    #[pyo3(text_signature = "(self, assignment)")]
    fn write(&mut self, assignment: Vec<u16>) -> PyResult<()> {
        if let Some(n) = self.graph_node_count {
            if assignment.len() != n {
                return Err(PyValueError::new_err(format!(
                    "assignment length {} does not match graph node count {n}",
                    assignment.len()
                )));
            }
        }
        let writer = self
            .writer
            .as_mut()
            .ok_or_else(|| PyIOError::new_err("stream session is already closed"))?;
        writer.write_assignment(assignment).map_err(map_io_err)?;
        self.sample_count += 1;
        Ok(())
    }

    /// Finalize the bundle and close the stream. Idempotent.
    ///
    /// You usually do not call this directly; leaving the stream ``with`` block cleanly calls it.
    fn close(&mut self, py: Python<'_>) -> PyResult<()> {
        let Some(writer) = self.writer.take() else {
            return Ok(());
        };
        let session = writer.finish_into_inner().map_err(map_io_err)?;
        let bundle = session.finish_into_writer(self.sample_count);
        bundle.finish().map_err(map_bundle_err)?;
        self.encoder.borrow_mut(py).mark_finalized();
        Ok(())
    }

    fn __enter__(slf: PyRefMut<Self>) -> PyRefMut<Self> {
        slf
    }

    fn __exit__(
        &mut self,
        py: Python<'_>,
        exc_type: Option<&Bound<'_, PyAny>>,
        _exc_value: Option<&Bound<'_, PyAny>>,
        _traceback: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<bool> {
        if exc_type.is_some() {
            // Leave the bundle unfinalized: dropping the writer abandons the session without
            // patching `finalized`, so the partial write is recoverable via allow_unfinalized
            // rather than being stamped complete over a truncated stream.
            self.writer = None;
            self.encoder.borrow_mut(py).mark_failed();
            Ok(false)
        } else {
            self.close(py)?;
            Ok(false)
        }
    }
}
