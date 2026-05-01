use super::helpers::{parse_graph_input, xz_compress};
use super::types::{OutputMode, SharedFileSlot, SharedFileWriter};
use crate::common::{open_output, parse_variant};
use binary_ensemble::io::bundle::format::{
    encode_directory, AssignmentFormat, BendlDirectoryEntry, BendlHeader, ASSET_FLAG_JSON,
    ASSET_FLAG_XZ, ASSET_TYPE_GRAPH, CANONICAL_NAME_GRAPH, COMPLETE_YES, HEADER_SIZE,
};
use binary_ensemble::io::writer::AssignmentWriter;
use pyo3::exceptions::{PyException, PyIOError, PyValueError};
use pyo3::prelude::*;
use std::cell::RefCell;
use std::io::{Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::rc::Rc;

#[pyclass(unsendable)]
pub struct PyBenEncoder {
    file: Option<SharedFileSlot>,
    encoder: Option<AssignmentWriter<SharedFileWriter>>,
    mode: OutputMode,
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
        let file: SharedFileSlot = Rc::new(RefCell::new(buf));

        let mode = if ben_file_only {
            OutputMode::BenOnly
        } else {
            let graph_bytes = match graph {
                Some(obj) => Some(parse_graph_input(py, &obj)?),
                None => None,
            };

            // Write a provisional bundle header and any graph asset before
            // the assignment stream begins.
            let mut header = BendlHeader::provisional(AssignmentFormat::Ben, HEADER_SIZE as u64);
            let mut entries: Vec<BendlDirectoryEntry> = Vec::new();
            {
                let mut slot = file.borrow_mut();
                slot.seek(SeekFrom::Start(0))
                    .map_err(|e| PyIOError::new_err(format!("Failed to seek output: {e}")))?;
                header.write_to(&mut *slot).map_err(|e| {
                    PyIOError::new_err(format!("Failed to write bundle header: {e}"))
                })?;

                if let Some(bytes) = graph_bytes {
                    let compressed = xz_compress(&bytes).map_err(|e| {
                        PyIOError::new_err(format!("Failed to xz-compress graph asset: {e}"))
                    })?;
                    let payload_offset = slot.stream_position().map_err(|e| {
                        PyIOError::new_err(format!("Failed to query output position: {e}"))
                    })?;
                    slot.write_all(&compressed).map_err(|e| {
                        PyIOError::new_err(format!("Failed to write graph asset payload: {e}"))
                    })?;
                    entries.push(BendlDirectoryEntry {
                        asset_type: ASSET_TYPE_GRAPH,
                        asset_flags: ASSET_FLAG_JSON | ASSET_FLAG_XZ,
                        name: CANONICAL_NAME_GRAPH.to_string(),
                        payload_offset,
                        payload_len: compressed.len() as u64,
                        checksum: None,
                    });
                }
            }

            let stream_start = file
                .borrow_mut()
                .stream_position()
                .map_err(|e| PyIOError::new_err(format!("Failed to query output position: {e}")))?;
            header.stream_offset = stream_start;

            OutputMode::Bundle {
                header,
                entries,
                stream_start,
                sample_count: 0,
            }
        };

        // Construct the AssignmentWriter on a clone of the shared slot.
        // This writes the BEN banner as its first action, which in the
        // bundle case becomes the first byte of the stream region.
        let encoder = AssignmentWriter::new(SharedFileWriter(Rc::clone(&file)), ben_var)
            .map_err(|e| PyIOError::new_err(format!("Failed to create encoder: {e}")))?;

        Ok(PyBenEncoder {
            file: Some(file),
            encoder: Some(encoder),
            mode,
        })
    }

    /// Encode a single assignment and append it to the output stream.
    #[pyo3(signature = (assignment))]
    #[pyo3(text_signature = "(assignment)")]
    fn write(&mut self, assignment: Vec<u16>) -> PyResult<()> {
        let enc = self
            .encoder
            .as_mut()
            .ok_or_else(|| PyIOError::new_err("Encoder has already been closed."))?;
        enc.write_assignment(assignment)
            .map_err(|e| PyIOError::new_err(format!("Failed to encode assignment: {e}")))?;
        if let OutputMode::Bundle { sample_count, .. } = &mut self.mode {
            *sample_count += 1;
        }
        Ok(())
    }

    /// Flush the assignment stream and, for bundle output, patch the
    /// header and write the trailing directory. Idempotent.
    fn close(&mut self) -> PyResult<()> {
        // Finish the assignment stream and drop the inner encoder so its
        // Rc handle to the shared file slot is released.
        if let Some(mut enc) = self.encoder.take() {
            enc.finish().map_err(|e| {
                PyIOError::new_err(format!("Failed to flush encoder when closing: {e}"))
            })?;
            drop(enc);
        }

        let file = match self.file.take() {
            Some(f) => f,
            None => return Ok(()),
        };

        match &mut self.mode {
            OutputMode::BenOnly => {
                file.borrow_mut()
                    .flush()
                    .map_err(|e| PyIOError::new_err(format!("Failed to flush output: {e}")))?;
            }
            OutputMode::Bundle {
                header,
                entries,
                stream_start,
                sample_count,
            } => {
                let mut slot = file.borrow_mut();
                let stream_end = slot.stream_position().map_err(|e| {
                    PyIOError::new_err(format!("Failed to query output position: {e}"))
                })?;
                let stream_len = stream_end.saturating_sub(*stream_start);

                let directory_offset = stream_end;
                let directory_bytes = encode_directory(entries).map_err(|e| {
                    PyException::new_err(format!("Failed to encode bundle directory: {e}"))
                })?;
                slot.write_all(&directory_bytes).map_err(|e| {
                    PyIOError::new_err(format!("Failed to write bundle directory: {e}"))
                })?;
                let directory_len = directory_bytes.len() as u64;

                header.stream_offset = *stream_start;
                header.stream_len = stream_len;
                header.directory_offset = directory_offset;
                header.directory_len = directory_len;
                header.sample_count = *sample_count;
                header.complete = COMPLETE_YES;

                slot.seek(SeekFrom::Start(0))
                    .map_err(|e| PyIOError::new_err(format!("Failed to seek output: {e}")))?;
                header.write_to(&mut *slot).map_err(|e| {
                    PyIOError::new_err(format!("Failed to patch bundle header: {e}"))
                })?;
                slot.flush()
                    .map_err(|e| PyIOError::new_err(format!("Failed to flush output: {e}")))?;
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
