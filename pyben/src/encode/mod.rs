use crate::common::{open_input, open_output, parse_variant, validate_input_output_paths};
use binary_ensemble::codec::encode::{
    encode_ben_to_xben, encode_jsonl_to_ben, encode_jsonl_to_xben,
};
use binary_ensemble::io::bundle::format::{
    encode_directory, AssignmentFormat, BendlDirectoryEntry, BendlHeader, ASSET_FLAG_JSON,
    ASSET_FLAG_XZ, ASSET_TYPE_GRAPH, CANONICAL_NAME_GRAPH, COMPLETE_YES, DEFAULT_XZ_PRESET,
    HEADER_SIZE,
};
use binary_ensemble::io::writer::AssignmentWriter;
use pyo3::exceptions::{PyException, PyIOError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict, PyList};
use std::cell::RefCell;
use std::fs::File;
use std::io::{self, BufWriter, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::rc::Rc;
use xz2::write::XzEncoder;

/// Handle to the underlying output file shared between the live
/// `AssignmentWriter` and the `PyBenEncoder` that owns it. Needed so the
/// encoder can reach the buffered file after the inner assignment writer
/// has finished, in order to patch the bundle header and write the
/// trailing directory.
type SharedFileSlot = Rc<RefCell<BufWriter<File>>>;

/// Wrapper around a shared buffered file that implements `Write`. The
/// `AssignmentWriter` holds one of these and delegates every write into
/// the shared slot.
struct SharedFileWriter(SharedFileSlot);

impl Write for SharedFileWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.borrow_mut().write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.borrow_mut().flush()
    }
}

/// Output container produced by `PyBenEncoder`.
enum OutputMode {
    /// Plain `.ben` file: just the assignment stream, no header or directory.
    BenOnly,
    /// `.bendl` bundle: provisional header up front, optional graph asset,
    /// then the assignment stream, then a directory written at close time.
    Bundle {
        header: BendlHeader,
        entries: Vec<BendlDirectoryEntry>,
        stream_start: u64,
        sample_count: i64,
    },
}

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
    #[pyo3(text_signature = "(file_path, overwrite=False, variant=None, graph=None, ben_file_only=False)")]
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
                header
                    .write_to(&mut *slot)
                    .map_err(|e| PyIOError::new_err(format!("Failed to write bundle header: {e}")))?;

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

            let stream_start = file.borrow_mut().stream_position().map_err(|e| {
                PyIOError::new_err(format!("Failed to query output position: {e}"))
            })?;
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
        let enc = self.encoder.as_mut().ok_or_else(|| {
            PyIOError::new_err("Encoder has already been closed.")
        })?;
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

/// xz-compress a byte slice with the bundle's default preset.
fn xz_compress(bytes: &[u8]) -> io::Result<Vec<u8>> {
    let mut encoder = XzEncoder::new(Vec::new(), DEFAULT_XZ_PRESET);
    encoder.write_all(bytes)?;
    encoder.finish()
}

/// Normalize a user-supplied graph argument into raw UTF-8 JSON bytes.
///
/// Accepted forms:
///
/// - `dict` / `list`: serialized via `json.dumps`.
/// - `bytes` / `bytearray`: used verbatim.
/// - any object with a `.read()` method (e.g. `io.BytesIO`, open files):
///   `.read()` is called and the result is coerced to bytes.
/// - `pathlib.Path` or `str`: treated as a filesystem path to read.
fn parse_graph_input(py: Python<'_>, obj: &Bound<'_, PyAny>) -> PyResult<Vec<u8>> {
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

#[pyfunction]
#[pyo3(signature = (in_file, out_file, overwrite=false, n_threads = None, compression_level = None))]
#[pyo3(
    text_signature = "(in_file, out_file, overwrite=false, n_threads=None, compression_level=None)"
)]
pub fn compress_ben_to_xben(
    in_file: PathBuf,
    out_file: PathBuf,
    overwrite: bool,
    n_threads: Option<u32>,
    compression_level: Option<u32>,
) -> PyResult<()> {
    validate_input_output_paths(&in_file, &out_file)?;
    let reader = open_input(&in_file)?;
    let writer = open_output(&out_file, overwrite)?;

    encode_ben_to_xben(reader, writer, n_threads, compression_level, None).map_err(|e| {
        PyIOError::new_err(format!(
            "Failed to convert BEN to XBEN from {} to {}: {e}",
            in_file.display(),
            out_file.display()
        ))
    })?;

    Ok(())
}

#[pyfunction]
#[pyo3(signature = (in_file, out_file, overwrite=false, variant="mkv_chain"))]
#[pyo3(text_signature = "(in_file, out_file, overwrite=false, variant='mkv_chain')")]
pub fn compress_jsonl_to_ben(
    in_file: PathBuf,
    out_file: PathBuf,
    overwrite: bool,
    variant: &str,
) -> PyResult<()> {
    let ben_var = parse_variant(Some(variant))?;
    validate_input_output_paths(&in_file, &out_file)?;
    let reader = open_input(&in_file)?;
    let writer = open_output(&out_file, overwrite)?;

    encode_jsonl_to_ben(reader, writer, ben_var).map_err(|e| {
        PyIOError::new_err(format!(
            "Failed to convert JSONL to BEN from {} to {}: {e}",
            in_file.display(),
            out_file.display()
        ))
    })?;
    Ok(())
}

#[pyfunction]
#[pyo3(signature = (in_file, out_file, overwrite=false, variant="mkv_chain", n_threads=None, compression_level=None))]
#[pyo3(
    text_signature = "(in_file, out_file, overwrite=false, variant='mkv_chain', n_threads=None, compression_level=None)"
)]
pub fn compress_jsonl_to_xben(
    in_file: PathBuf,
    out_file: PathBuf,
    overwrite: bool,
    variant: &str,
    n_threads: Option<u32>,
    compression_level: Option<u32>,
) -> PyResult<()> {
    let ben_var = parse_variant(Some(variant))?;
    validate_input_output_paths(&in_file, &out_file)?;
    let reader = open_input(&in_file)?;
    let writer = open_output(&out_file, overwrite)?;

    encode_jsonl_to_xben(reader, writer, ben_var, n_threads, compression_level, None).map_err(
        |e| {
            PyIOError::new_err(format!(
                "Failed to convert JSONL to XBEN from {} to {}: {e}",
                in_file.display(),
                out_file.display()
            ))
        },
    )?;
    Ok(())
}
