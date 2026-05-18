use super::types::{BundleState, DecoderBackend, DecoderMode, DynIter};
use crate::common::open_input;
use binary_ensemble::io::bundle::format::BENDL_MAGIC;
use binary_ensemble::io::reader::{
    build_frame_iter, build_frame_iter_from_reader, count_samples_from_frame_iter, BenStreamReader,
    BenWireFormat,
};
use pyo3::exceptions::{PyException, PyIOError, PyUserWarning};
use pyo3::prelude::*;
use pyo3::types::PyDict;
use std::fs::File;
use std::io::{self, BufReader, Read, Seek, SeekFrom};
use std::path::Path;

pub(super) fn warn_xben_startup(py: Python<'_>) -> PyResult<()> {
    let warnings = py.import("warnings")?;
    let kwargs = PyDict::new(py);

    warnings.call_method(
        "warn",
        (
            "XBEN may take a second to start decoding.",
            py.get_type::<PyUserWarning>(),
        ),
        Some(&kwargs),
    )?;

    Ok(())
}

/// Sniff the first 8 bytes of a file and decide whether it starts with the `BENDL` magic.
pub(super) fn detect_is_bundle(path: &Path) -> io::Result<bool> {
    let mut file = File::open(path)?;
    let mut magic = [0u8; 8];
    match file.read_exact(&mut magic) {
        Ok(()) => Ok(magic == BENDL_MAGIC),
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => Ok(false),
        Err(e) => Err(e),
    }
}

fn open_stream_reader<R>(reader: R, format: BenWireFormat) -> PyResult<DynIter>
where
    R: Read + Send + 'static,
{
    match format {
        BenWireFormat::Ben => {
            let ben = BenStreamReader::from_ben(reader)
                .map_err(|e| PyException::new_err(format!("Failed to create BenDecoder: {e}")))?;
            Ok(Box::new(ben))
        }
        BenWireFormat::XBen => {
            let xben = BenStreamReader::from_xben(reader)
                .map_err(|e| PyException::new_err(format!("Failed to create XBenDecoder: {e}")))?;
            Ok(Box::new(xben))
        }
    }
}

/// Build a plain-stream iterator from `path` using `mode`.
pub(super) fn build_plain_iter(path: &Path, mode: DecoderMode) -> PyResult<DynIter> {
    let reader = open_input(&path.to_path_buf())?;
    open_stream_reader(reader, mode.wire_format())
}

/// Open a second file handle on the bundle path, seek to the stream region, and wrap it in the
/// appropriate assignment reader so the decoder iterator only walks the embedded stream.
pub(super) fn build_bundle_iter(
    path: &Path,
    state: &BundleState,
    mode: DecoderMode,
) -> PyResult<DynIter> {
    let reader = open_bundle_stream_reader(path, state)?;
    open_stream_reader(reader, mode.wire_format())
}

/// Create a `Read`-only handle bounded to the bundle's assignment stream region.
pub(super) fn open_bundle_stream_reader(
    path: &Path,
    state: &BundleState,
) -> PyResult<io::Take<BufReader<File>>> {
    let file = File::open(path)
        .map_err(|e| PyIOError::new_err(format!("Failed to open {}: {e}", path.display())))?;
    let mut buf = BufReader::new(file);
    buf.seek(SeekFrom::Start(state.stream_offset))
        .map_err(|e| PyIOError::new_err(format!("Failed to seek into bundle stream: {e}")))?;
    Ok(buf.take(state.stream_len))
}

pub(super) fn build_frames_for_subsample(
    path: &Path,
    mode: DecoderMode,
    backend: &DecoderBackend,
) -> PyResult<binary_ensemble::io::reader::FrameIter> {
    let format = mode.wire_format();
    match backend {
        DecoderBackend::Plain => build_frame_iter(&path.to_path_buf(), format).map_err(|e| {
            PyException::new_err(format!(
                "Failed to create frame iterator from {}: {e}",
                path.display()
            ))
        }),
        DecoderBackend::Bundle(state) => {
            let reader = open_bundle_stream_reader(path, state)?;
            build_frame_iter_from_reader(reader, format).map_err(|e| {
                PyException::new_err(format!(
                    "Failed to create frame iterator from bundle {}: {e}",
                    path.display()
                ))
            })
        }
    }
}

pub(super) fn scan_bundle_samples(
    path: &Path,
    state: &BundleState,
    mode: DecoderMode,
) -> PyResult<usize> {
    let reader = open_bundle_stream_reader(path, state)?;
    let iter = build_frame_iter_from_reader(reader, mode.wire_format()).map_err(|e| {
        PyException::new_err(format!(
            "Failed to open bundle stream for sample count: {e}"
        ))
    })?;
    count_samples_from_frame_iter(iter)
        .map_err(|e| PyException::new_err(format!("Failed to count samples in bundle: {e}")))
}
