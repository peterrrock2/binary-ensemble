use super::helpers::parse_graph_input;
use crate::common::{open_output, parse_variant};
use binary_ensemble::io::bundle::format::{AssignmentFormat, KnownAssetKind};
use binary_ensemble::io::bundle::{
    AddAssetOptions, BendlStreamSession, BendlWriteError, BendlWriter,
};
use binary_ensemble::io::writer::BenStreamWriter;
use pyo3::exceptions::{PyException, PyIOError, PyValueError};
use pyo3::prelude::*;
use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::PathBuf;

/// Per-call encoder state. The bundle path threads ownership of the
/// underlying file through `BendlWriter` → `BendlStreamSession` →
/// `BenStreamWriter`, so when `close()` runs we walk the chain back
/// from `BenStreamWriter::finish_into_inner` (returning the session)
/// to `BendlStreamSession::finish_into_writer` (returning the bundle
/// writer) to `BendlWriter::finish` (returning the buffered file).
enum EncoderState {
    /// Plain `.ben` file path: writes directly to a buffered file with
    /// no bundle framing.
    BenOnly(BenStreamWriter<BufWriter<File>>),
    /// `.bendl` bundle path: the session owns the buffered file and the
    /// `BenStreamWriter` writes through it. `sample_count` is tracked
    /// alongside so it can be plumbed into `finish_into_writer` at
    /// `close()` time.
    BundleStreaming {
        writer: BenStreamWriter<BendlStreamSession<BufWriter<File>>>,
        sample_count: i64,
    },
}

#[pyclass(name = "BenEncoder", unsendable)]
pub struct PyBenEncoder {
    state: Option<EncoderState>,
}

impl PyBenEncoder {
    fn map_bundle_err(err: BendlWriteError) -> PyErr {
        match err {
            BendlWriteError::Io(e) => PyIOError::new_err(format!("{e}")),
            other => PyException::new_err(format!("{other}")),
        }
    }

    fn map_io_err(err: io::Error) -> PyErr {
        PyIOError::new_err(format!("{err}"))
    }
}

#[pymethods]
impl PyBenEncoder {
    /// Open a new encoder. The default output is a `.bendl` bundle with
    /// an embedded assignment stream and an optional embedded graph; set
    /// `ben_file_only=True` to emit a plain `.ben` file instead.
    ///
    /// # Arguments
    ///
    /// * `file_path` - Output path. Must not exist unless `overwrite=True`.
    /// * `overwrite` - Replace an existing file at `file_path`.
    /// * `variant` - BEN variant for the assignment stream (`"standard"`,
    ///   `"mkv_chain"`, or `"twodelta"`).
    /// * `graph` - Optional graph to embed as the `graph.json` asset when
    ///   writing a bundle. Accepts a `pathlib.Path` / `str` path, a
    ///   `bytes` object containing UTF-8 JSON, a Python `dict` / `list`
    ///   that will be serialized with `json.dumps`, or a file-like object
    ///   with a `.read()` method. Passing a graph alongside
    ///   `ben_file_only=True` is an error.
    /// * `ben_file_only` - If `True`, emit a plain `.ben` file with no
    ///   bundle framing. Defaults to `False`.
    #[new]
    #[pyo3(signature = (
        file_path,
        overwrite = false,
        variant = None,
        graph = None,
        ben_file_only = false,
    ))]
    #[pyo3(
        text_signature = "(file_path, overwrite=False, variant=None, graph=None, ben_file_only=False)"
    )]
    fn new(
        py: Python<'_>,
        file_path: PathBuf,
        overwrite: bool,
        variant: Option<String>,
        graph: Option<Bound<'_, PyAny>>,
        ben_file_only: bool,
    ) -> PyResult<Self> {
        let ben_var = parse_variant(variant.as_deref())?;

        if ben_file_only && graph.is_some() {
            return Err(PyValueError::new_err(
                "graph= cannot be combined with ben_file_only=True (the graph \
                 would have nowhere to live in a plain .ben file).",
            ));
        }

        let buf = open_output(&file_path, overwrite)?;

        let state = if ben_file_only {
            EncoderState::BenOnly(
                BenStreamWriter::for_ben(buf, ben_var).map_err(Self::map_io_err)?,
            )
        } else {
            // Bundle path. Add the optional graph asset before opening
            // the stream session — the bundle writer auto-compresses
            // graphs (default_compresses_by_type), so we hand it raw
            // JSON bytes and let it apply the XZ flag.
            let mut writer =
                BendlWriter::new(buf, AssignmentFormat::Ben).map_err(Self::map_io_err)?;
            if let Some(graph_obj) = graph {
                let raw = parse_graph_input(py, &graph_obj)?;
                writer
                    .add_known_asset(
                        KnownAssetKind::Graph,
                        &raw,
                        AddAssetOptions::defaults().json(),
                    )
                    .map_err(Self::map_bundle_err)?;
            }
            let session = writer
                .into_stream_session()
                .map_err(Self::map_bundle_err)?;
            let writer = BenStreamWriter::for_ben(session, ben_var).map_err(Self::map_io_err)?;
            EncoderState::BundleStreaming {
                writer,
                sample_count: 0,
            }
        };

        Ok(Self { state: Some(state) })
    }

    /// Encode a single assignment and append it to the output stream.
    #[pyo3(signature = (assignment))]
    #[pyo3(text_signature = "(assignment)")]
    fn write(&mut self, assignment: Vec<u16>) -> PyResult<()> {
        let state = self
            .state
            .as_mut()
            .ok_or_else(|| PyIOError::new_err("Encoder has already been closed."))?;
        match state {
            EncoderState::BenOnly(w) => {
                w.write_assignment(assignment).map_err(Self::map_io_err)?;
            }
            EncoderState::BundleStreaming {
                writer,
                sample_count,
            } => {
                writer
                    .write_assignment(assignment)
                    .map_err(Self::map_io_err)?;
                *sample_count += 1;
            }
        }
        Ok(())
    }

    /// Flush the assignment stream and, for bundle output, patch the
    /// header and write the trailing directory. Idempotent.
    fn close(&mut self) -> PyResult<()> {
        let Some(state) = self.state.take() else {
            return Ok(());
        };
        match state {
            EncoderState::BenOnly(writer) => {
                let mut buf = writer.finish_into_inner().map_err(Self::map_io_err)?;
                buf.flush().map_err(Self::map_io_err)?;
            }
            EncoderState::BundleStreaming {
                writer,
                sample_count,
            } => {
                let session = writer.finish_into_inner().map_err(Self::map_io_err)?;
                let bundle = session.finish_into_writer(sample_count);
                bundle.finish().map_err(Self::map_bundle_err)?;
            }
        }
        Ok(())
    }

    fn __enter__(slf: pyo3::PyRefMut<Self>) -> pyo3::PyRefMut<Self> {
        slf
    }

    fn __exit__(
        &mut self,
        _exc_type: Option<&pyo3::Bound<'_, pyo3::types::PyAny>>,
        _exc_value: Option<&pyo3::Bound<'_, pyo3::types::PyAny>>,
        _traceback: Option<&pyo3::Bound<'_, pyo3::types::PyAny>>,
    ) -> PyResult<bool> {
        self.close()?;
        Ok(false)
    }
}
