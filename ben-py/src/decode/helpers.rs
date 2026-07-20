use super::types::{DecoderMode, DynIter, StreamSource};
use crate::common::open_input;
use binary_ensemble::io::bundle::format::BENDL_MAGIC;
use binary_ensemble::io::bundle::ExactLen;
use binary_ensemble::io::reader::{
    build_frame_iter, build_frame_iter_from_reader, count_samples_from_file,
    count_samples_from_frame_iter, BenStreamReader, BenWireFormat, FrameIter,
};
use binary_ensemble::ops::extract::{extract_assignment_ben_seek, SampleError};
use pyo3::exceptions::{PyException, PyIOError, PyIndexError, PyUserWarning};
use pyo3::prelude::*;
use pyo3::types::PyDict;
use std::fs::File;
use std::io::{self, BufReader, Read, Seek, SeekFrom};
use std::path::Path;

/// On-disk identity of a bundle file, captured when a decoder opens it.
///
/// In-place transforms (`remove_asset`, `compress_stream`, `relabel_bundle`) swap a rewritten
/// file over the path, and appends rewrite the directory in place. A decoder built before
/// either would silently serve a mix of old bytes (through its held handle) and new bytes
/// (through path reopens for iteration), so every IO entry point compares the file's current
/// identity against this snapshot and refuses with a clear error when they diverge.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct FileIdentity {
    len: u64,
    modified: Option<std::time::SystemTime>,
    #[cfg(unix)]
    dev: u64,
    #[cfg(unix)]
    ino: u64,
}

impl FileIdentity {
    fn from_metadata(meta: &std::fs::Metadata) -> Self {
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;
        FileIdentity {
            len: meta.len(),
            modified: meta.modified().ok(),
            #[cfg(unix)]
            dev: meta.dev(),
            #[cfg(unix)]
            ino: meta.ino(),
        }
    }

    /// Capture the identity of an already-open handle (the exact inode the decoder reads).
    pub(crate) fn of_file(file: &File) -> io::Result<Self> {
        Ok(Self::from_metadata(&file.metadata()?))
    }

    /// Error unless the file currently at `path` is still the captured one, unmodified.
    pub(crate) fn ensure_unchanged(&self, path: &Path, op: &str) -> PyResult<()> {
        let meta = std::fs::metadata(path).map_err(|e| {
            PyIOError::new_err(format!(
                "cannot {op}: failed to stat {}: {e}",
                path.display()
            ))
        })?;
        if Self::from_metadata(&meta) != *self {
            return Err(PyException::new_err(format!(
                "cannot {op}: the bundle at {} changed on disk after this decoder opened it \
                 (in-place transforms swap in a rewritten file; appends rewrite the directory). \
                 Open a fresh BendlDecoder to read the current file.",
                path.display()
            )));
        }
        Ok(())
    }
}

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

/// A bundle stream region bounded by [`ExactLen`], with its short-range EOF remapped to a hard
/// decode error.
///
/// `ExactLen` reports a backing range shorter than the declared `stream_len` as `UnexpectedEof`,
/// but the BEN/XBEN frame decoders treat an `UnexpectedEof` at a frame boundary as a clean end of
/// stream. A `.bendl` whose `stream_len` overruns the file but ends on a frame boundary would
/// therefore iterate as a silent truncated prefix. Remapping the short range to `InvalidData`
/// forces the decoder to surface it. The Rust bundle API catches the same case through its CRC
/// check; Python iteration/subsample is non-verifying by design, so it needs this explicit guard.
struct StrictStreamRegion<R: Read> {
    inner: ExactLen<R>,
}

impl<R: Read> Read for StrictStreamRegion<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self.inner.read(buf) {
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "bundle assignment stream is shorter than its declared stream_len \
                 (truncated or corrupt bundle)",
            )),
            other => other,
        }
    }
}

/// Create a `Read`-only handle bounded to a bundle's assignment stream region.
///
/// The cached `stream_offset`/`stream_len` are only meaningful for the file the decoder
/// originally opened, so the reopen refuses when the file at `path` has changed identity. The range
/// is bounded with [`StrictStreamRegion`] so a `stream_len` that overruns the file is rejected
/// rather than silently truncating iteration.
fn open_bundle_stream_reader(
    path: &Path,
    identity: &FileIdentity,
    stream_offset: u64,
    stream_len: u64,
) -> PyResult<StrictStreamRegion<BufReader<File>>> {
    identity.ensure_unchanged(path, "iterate the bundle stream")?;
    let file = File::open(path)
        .map_err(|e| PyIOError::new_err(format!("Failed to open {}: {e}", path.display())))?;
    let mut buf = BufReader::new(file);
    buf.seek(SeekFrom::Start(stream_offset))
        .map_err(|e| PyIOError::new_err(format!("Failed to seek into bundle stream: {e}")))?;
    Ok(StrictStreamRegion {
        inner: ExactLen::bounded(buf, stream_len),
    })
}

/// A seekable view whose positions are relative to one bounded region of a larger file.
struct SeekableStreamRegion<R: Read + Seek> {
    inner: R,
    start: u64,
    len: u64,
    position: u64,
}

impl<R: Read + Seek> SeekableStreamRegion<R> {
    fn new(mut inner: R, start: u64, len: u64) -> io::Result<Self> {
        inner.seek(SeekFrom::Start(start))?;
        Ok(Self {
            inner,
            start,
            len,
            position: 0,
        })
    }
}

impl<R: Read + Seek> Read for SeekableStreamRegion<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() || self.position == self.len {
            return Ok(0);
        }
        let available = self.len - self.position;
        let max = available.min(buf.len() as u64) as usize;
        let n = self.inner.read(&mut buf[..max])?;
        if n == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "bundle assignment stream is shorter than its declared stream_len",
            ));
        }
        self.position += n as u64;
        Ok(n)
    }
}

impl<R: Read + Seek> Seek for SeekableStreamRegion<R> {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        let target = match position {
            SeekFrom::Start(offset) => i128::from(offset),
            SeekFrom::Current(offset) => i128::from(self.position) + i128::from(offset),
            SeekFrom::End(offset) => i128::from(self.len) + i128::from(offset),
        };
        if target < 0 || target > i128::from(self.len) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "seek outside bundle assignment stream",
            ));
        }

        let relative = target as u64;
        let absolute = self.start.checked_add(relative).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "bundle stream offset overflow")
        })?;
        self.inner.seek(SeekFrom::Start(absolute))?;
        self.position = relative;
        Ok(relative)
    }
}

/// Look up one sample in an uncompressed BEN source using the core seek-and-replay path.
pub(super) fn lookup_ben(source: &StreamSource, index: usize) -> PyResult<Vec<u16>> {
    let result = match source {
        StreamSource::Plain { path } => {
            let file = File::open(path).map_err(|e| {
                PyIOError::new_err(format!("Failed to open {}: {e}", path.display()))
            })?;
            extract_assignment_ben_seek(BufReader::new(file), index)
        }
        StreamSource::Bundle {
            path,
            identity,
            stream_offset,
            stream_len,
            ..
        } => {
            identity.ensure_unchanged(path, "look up a bundle sample")?;
            let file = File::open(path).map_err(|e| {
                PyIOError::new_err(format!("Failed to open {}: {e}", path.display()))
            })?;
            let reader =
                SeekableStreamRegion::new(BufReader::new(file), *stream_offset, *stream_len)
                    .map_err(|e| {
                        PyIOError::new_err(format!("Failed to seek into bundle stream: {e}"))
                    })?;
            extract_assignment_ben_seek(reader, index)
        }
    };

    match result {
        Ok(assignment) => Ok(assignment),
        Err(e @ SampleError::SampleNotFound { .. }) => Err(PyIndexError::new_err(e.to_string())),
        Err(e) => Err(PyException::new_err(format!(
            "Error looking up sample {index}: {e}"
        ))),
    }
}

/// Build a fresh assignment iterator for the given source.
///
/// A finalized assets-only bundle (`StreamSource::Bundle { empty: true, .. }`) has no BEN banner to
/// parse, so it yields an empty iterator rather than failing on the missing banner.
pub(super) fn build_iter(source: &StreamSource, mode: DecoderMode) -> PyResult<DynIter> {
    match source {
        StreamSource::Plain { path } => {
            let reader = open_input(&path.to_path_buf())?;
            open_stream_reader(reader, mode.wire_format())
        }
        StreamSource::Bundle { empty: true, .. } => Ok(Box::new(std::iter::empty())),
        StreamSource::Bundle {
            path,
            identity,
            stream_offset,
            stream_len,
            ..
        } => {
            let reader = open_bundle_stream_reader(path, identity, *stream_offset, *stream_len)?;
            open_stream_reader(reader, mode.wire_format())
        }
    }
}

/// Build a frame iterator for subsample selection over the given source.
pub(super) fn build_frames_for_subsample(
    source: &StreamSource,
    mode: DecoderMode,
) -> PyResult<FrameIter> {
    let format = mode.wire_format();
    match source {
        StreamSource::Plain { path } => {
            build_frame_iter(&path.to_path_buf(), format).map_err(|e| {
                PyException::new_err(format!(
                    "Failed to create frame iterator from {}: {e}",
                    path.display()
                ))
            })
        }
        StreamSource::Bundle {
            path,
            identity,
            stream_offset,
            stream_len,
            ..
        } => {
            let reader = open_bundle_stream_reader(path, identity, *stream_offset, *stream_len)?;
            build_frame_iter_from_reader(reader, format).map_err(|e| {
                PyException::new_err(format!(
                    "Failed to create frame iterator from bundle {}: {e}",
                    path.display()
                ))
            })
        }
    }
}

/// Count the samples in a source by reading it from the start.
pub(super) fn scan_samples(
    source: &StreamSource,
    mode: DecoderMode,
    py: Python<'_>,
) -> PyResult<usize> {
    match source {
        StreamSource::Plain { path } => {
            let path = path.clone();
            let format = mode.wire_format();
            py.detach(|| count_samples_from_file(&path, format))
                .map_err(|e| {
                    PyException::new_err(format!(
                        "Failed to count samples in {}: {e}",
                        path.display()
                    ))
                })
        }
        StreamSource::Bundle {
            path,
            identity,
            stream_offset,
            stream_len,
            ..
        } => {
            let reader = open_bundle_stream_reader(path, identity, *stream_offset, *stream_len)?;
            let format = mode.wire_format();
            // Match the plain-file branch: the scan is Rust-only IO, so run it detached.
            py.detach(move || {
                let iter = build_frame_iter_from_reader(reader, format).map_err(|e| {
                    PyException::new_err(format!(
                        "Failed to open bundle stream for sample count: {e}"
                    ))
                })?;
                count_samples_from_frame_iter(iter).map_err(|e| {
                    PyException::new_err(format!("Failed to count samples in bundle: {e}"))
                })
            })
        }
    }
}
