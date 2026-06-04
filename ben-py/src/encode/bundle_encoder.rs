//! `.bendl` bundle authoring bindings: [`PyBendlEncoder`] and its [`PyBendlStreamSession`].
//!
//! The encoder threads the bundle through the library's typestate machinery — `BendlWriter`
//! (assets) → `BendlStreamSession` (stream) → `BendlWriter::finish` (finalize) — for the create
//! path, and reopens a `BendlAppender` per asset for post-stream / append-mode adds. The state enum
//! below tracks which phase the encoder is in so a second `stream()` is refused and so `add_*`
//! routes through the writer pre-stream and the appender afterwards.

use crate::common::{
    graph_node_count, networkx_graph_from_bytes, open_output, parse_graph_input, parse_variant,
};
use crate::graph::helpers::{reorder_graph_to_bytes, resolve_reorder};
use binary_ensemble::io::bundle::format::{AssignmentFormat, KnownAssetKind};
use binary_ensemble::io::bundle::writer::BendlAppender;
use binary_ensemble::io::bundle::{
    AddAssetOptions, BendlStreamSession, BendlWriteError, BendlWriter,
};
use binary_ensemble::io::writer::BenStreamWriter;
use pyo3::exceptions::{PyException, PyIOError, PyValueError};
use pyo3::prelude::*;
use std::fs::{File, OpenOptions};
use std::io::{self, BufWriter};
use std::path::PathBuf;

fn map_bundle_err(err: BendlWriteError) -> PyErr {
    match err {
        BendlWriteError::Io(e) => PyIOError::new_err(format!("{e}")),
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
        "text" => Ok(AddAssetOptions::defaults()),
        other => Err(PyValueError::new_err(format!(
            "content_type must be 'json' or 'text', got {other:?}"
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

/// Writer for a single `.bendl` bundle.
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
    /// # Arguments
    ///
    /// * `file_path` - Output path. Must not exist unless `overwrite=True`.
    /// * `overwrite` - Replace an existing file at `file_path`.
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

    /// Open an existing finalized bundle for append. `stream()` is unavailable; `add_*` commit
    /// immediately.
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

    /// Add a custom asset (asset type `CUSTOM`). `content_type` is `"json"` or `"text"`.
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

    /// Add the canonical `metadata.json` known asset. `metadata` accepts the same inputs as a graph
    /// (dict/list, bytes, a file-like with `.read()`, or a path).
    #[pyo3(signature = (metadata))]
    #[pyo3(text_signature = "(self, metadata)")]
    fn add_metadata(&mut self, py: Python<'_>, metadata: Bound<'_, PyAny>) -> PyResult<()> {
        let bytes = parse_graph_input(py, &metadata)?;
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

    /// Add the `graph.json` known asset.
    ///
    /// `sort` defaults to `"mlc"`, so by default the graph is reordered for better compression.
    /// `sort` is `"mlc"` (multi-level clustering), `"rcm"` (reverse Cuthill-McKee), `"key"` to sort
    /// by a node attribute named via `key` (e.g. `key="GEOID"`), or `None` to store the graph
    /// as-is. When reordering, both `graph.json` and `node_permutation_map.json` are stored,
    /// and the reordered graph is returned (as a NetworkX graph, matching
    /// `BendlDecoder.read_graph`) so the chain runs on that ordering. Reordering is pre-stream
    /// only; a raw graph (`sort=None`) may also be attached post-stream / in append mode. The
    /// returned graph's node count is recorded for per-write validation.
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

    /// Open the single-use assignment stream. Only `"ben"` is accepted today; XBEN comes from
    /// `bundle.compress_stream`. `variant` selects the BEN variant (default `"twodelta"`).
    #[pyo3(signature = (format = "ben", variant = None))]
    #[pyo3(text_signature = "(self, format='ben', variant=None)")]
    fn stream(
        slf: Bound<'_, Self>,
        format: &str,
        variant: Option<String>,
    ) -> PyResult<PyBendlStreamSession> {
        if format != "ben" {
            return Err(PyValueError::new_err(format!(
                "stream format must be 'ben' (got {format:?}); produce XBEN via \
                 binary_ensemble.bundle.compress_stream"
            )));
        }
        let ben_var = parse_variant(variant.as_deref())?;

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

    /// Finalize the bundle. Idempotent. In create mode a normal close (including before any
    /// `stream()`) finalizes the bundle; after a failed stream it does not finalize. In append mode
    /// it is a no-op after the already-committed appends.
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
        BundleState::PreStream { .. } | BundleState::Appendable => "invalid state",
    };
    PyException::new_err(format!("cannot {op}: {reason}"))
}

/// Single-use context manager over the bundle's assignment stream.
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
    /// Encode a single assignment. When the bundle carries a pre-stream graph, the assignment
    /// length must equal the graph's node count.
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
