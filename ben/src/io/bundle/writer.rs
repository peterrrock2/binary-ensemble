//! Write-side API for `.bendl` files.
//!
//! [`BendlWriter`] produces finalized bundles with the on-disk layout
//!
//! ```text
//! [header] [asset payloads] [assignment stream] [directory]
//! ```
//!
//! The writer operates in three logical phases:
//!
//! 1. **asset phase** — the caller invokes [`BendlWriter::add_asset`] zero
//!    or more times. Each call writes the (optionally xz-compressed)
//!    payload to the file and records its absolute offset and length in
//!    an in-memory entry list.
//! 2. **stream phase** — the caller invokes [`BendlWriter::begin_stream`]
//!    to enter the stream region. The returned handle wraps the raw
//!    underlying writer so the caller can plumb it into
//!    [`crate::io::writer::AssignmentWriter`] or
//!    [`crate::io::writer::XZAssignmentWriter`]. When the stream is
//!    complete the caller records the sample count via
//!    [`BendlWriter::end_stream`].
//! 3. **finalize phase** — [`BendlWriter::finish`] writes the trailing
//!    directory and patches the header.
//!
//! The writer requires `Write + Seek` because the header is patched
//! twice: once with the stream offset (implicitly, by having reserved
//! its slot at construction) and once with the finalized stream length,
//! sample count, directory offset, directory length, and `complete` flag.

use std::collections::HashSet;
use std::io::{self, Read, Seek, SeekFrom, Write};

use thiserror::Error;
use xz2::write::XzEncoder;

use super::format::{
    standardized_name_for, default_compresses_by_type, encode_directory, read_directory,
    AssignmentFormat, BendlDirectoryEntry, BendlFormatError, BendlHeader, ASSET_FLAG_JSON,
    ASSET_FLAG_XZ, FINALIZED_YES, DEFAULT_XZ_PRESET, HEADER_SIZE,
};

/// Ability to truncate an underlying seekable target to a given length.
///
/// This is not part of `std::io`, so `BendlAppender` takes a trait bound
/// that abstracts it and is implemented below for `std::fs::File` and
/// `std::io::Cursor<Vec<u8>>`.
pub trait BendlTruncate {
    /// Truncate or extend the underlying target to exactly `len` bytes.
    fn truncate_at(&mut self, len: u64) -> io::Result<()>;
}

impl BendlTruncate for std::fs::File {
    fn truncate_at(&mut self, len: u64) -> io::Result<()> {
        self.set_len(len)
    }
}

impl BendlTruncate for std::io::Cursor<Vec<u8>> {
    fn truncate_at(&mut self, len: u64) -> io::Result<()> {
        let target = len as usize;
        let vec = self.get_mut();
        debug_assert!(vec.len() >= target, "truncate_at called past end of buffer");
        vec.truncate(target);
        Ok(())
    }
}

/// Options passed alongside each [`BendlWriter::add_asset`] call.
#[derive(Debug, Clone, Default)]
pub struct AddAssetOptions {
    /// Compression override. `None` means "follow the default policy for
    /// this asset type"; `Some(true)` forces xz compression; `Some(false)`
    /// forces a raw payload.
    pub compress: Option<bool>,
    /// Whether the decoded payload is UTF-8 JSON. Adds the
    /// [`ASSET_FLAG_JSON`] bit to the entry's flags.
    pub is_json: bool,
    /// Optional trailing checksum bytes to store in the directory entry.
    /// When set, [`crate::io::bundle::format::ASSET_FLAG_CHECKSUM`] is
    /// applied automatically.
    pub checksum: Option<Vec<u8>>,
}

impl AddAssetOptions {
    /// Sentinel "use the default policy with no extras" options.
    pub fn defaults() -> Self {
        Self::default()
    }

    /// Flag a payload as UTF-8 JSON.
    pub fn json(mut self) -> Self {
        self.is_json = true;
        self
    }

    /// Force xz compression regardless of the default policy.
    pub fn compress(mut self) -> Self {
        self.compress = Some(true);
        self
    }

    /// Force the writer to store the payload raw even if the default
    /// policy would compress it.
    pub fn raw(mut self) -> Self {
        self.compress = Some(false);
        self
    }
}

/// Writer for a single `.bendl` file.
pub struct BendlWriter<W: Write + Seek> {
    inner: W,
    header: BendlHeader,
    entries: Vec<BendlDirectoryEntry>,
    names: HashSet<String>,
    singleton_types: HashSet<u16>,
    state: WriterState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WriterState {
    /// No assets have been written yet, but the provisional header is
    /// already in place and the writer is positioned just after it.
    Assets,
    /// `begin_stream` has been called; the caller is responsible for
    /// writing the embedded BEN/XBEN payload before calling `end_stream`.
    Streaming,
    /// `end_stream` has completed; the writer is ready for `finish`.
    StreamWritten { stream_len: u64, sample_count: i64 },
    /// `finish` has been called. No further operations are permitted.
    Finished,
}

impl<W: Write + Seek> BendlWriter<W> {
    /// Create a new writer by writing a provisional header at offset 0.
    ///
    /// The assignment stream will begin immediately after the asset
    /// payload region — [`BendlWriter::begin_stream`] computes the
    /// exact offset at the moment it is called, so asset writes that
    /// happen between `new` and `begin_stream` push the stream out as
    /// expected.
    pub fn new(mut inner: W, assignment_format: AssignmentFormat) -> io::Result<Self> {
        inner.seek(SeekFrom::Start(0))?;
        // stream_offset in the provisional header is patched at
        // begin_stream time; start it just after the header.
        let header = BendlHeader::provisional(assignment_format, HEADER_SIZE as u64);
        header.write_to(&mut inner)?;

        Ok(BendlWriter {
            inner,
            header,
            entries: Vec::new(),
            names: HashSet::new(),
            singleton_types: HashSet::new(),
            state: WriterState::Assets,
        })
    }

    /// Add an asset to the bundle.
    ///
    /// The payload is written to the file immediately at the current
    /// position (right after the previous asset, or right after the
    /// header if this is the first asset). Its absolute offset and
    /// length are recorded in the in-memory directory entry list.
    ///
    /// This method enforces the canonical-name and uniqueness rules
    /// **before** writing any bytes, so a rejected asset leaves the
    /// file untouched.
    pub fn add_asset(
        &mut self,
        asset_type: u16,
        name: &str,
        payload: &[u8],
        options: AddAssetOptions,
    ) -> Result<(), BendlWriteError> {
        if self.state != WriterState::Assets {
            return Err(BendlWriteError::AssetsAfterStream);
        }

        // Canonical-name rule for known singleton types.
        if let Some(canonical) = standardized_name_for(asset_type) {
            if name != canonical {
                return Err(BendlWriteError::WrongCanonicalName {
                    asset_type,
                    expected: canonical.to_string(),
                    found: name.to_string(),
                });
            }
            if !self.singleton_types.insert(asset_type) {
                return Err(BendlWriteError::DuplicateSingletonType(asset_type));
            }
        }

        // Unique name rule.
        if !self.names.insert(name.to_string()) {
            // Roll back the singleton insertion before returning, so
            // the writer remains in a consistent state. (Only known
            // singleton types would have been inserted above.)
            if standardized_name_for(asset_type).is_some() {
                self.singleton_types.remove(&asset_type);
            }
            return Err(BendlWriteError::DuplicateName(name.to_string()));
        }

        // Decide compression.
        let compress = options
            .compress
            .unwrap_or_else(|| default_compresses_by_type(asset_type));

        // Compute final payload bytes.
        let payload_bytes: Vec<u8> = if compress {
            let mut encoder = XzEncoder::new(Vec::new(), DEFAULT_XZ_PRESET);
            encoder.write_all(payload).map_err(BendlWriteError::Io)?;
            encoder.finish().map_err(BendlWriteError::Io)?
        } else {
            payload.to_vec()
        };

        // Flags.
        let mut asset_flags: u16 = 0;
        if options.is_json {
            asset_flags |= ASSET_FLAG_JSON;
        }
        if compress {
            asset_flags |= ASSET_FLAG_XZ;
        }
        if options.checksum.is_some() {
            asset_flags |= crate::io::bundle::format::ASSET_FLAG_CHECKSUM;
        }

        // Write at current file position.
        let payload_offset = self.inner.seek(SeekFrom::Current(0))?;
        self.inner
            .write_all(&payload_bytes)
            .map_err(BendlWriteError::Io)?;
        let payload_len = payload_bytes.len() as u64;

        self.entries.push(BendlDirectoryEntry {
            asset_type,
            asset_flags,
            name: name.to_string(),
            payload_offset,
            payload_len,
            checksum: options.checksum,
        });

        Ok(())
    }

    /// Convenience wrapper around [`add_asset`] for JSON-encoded assets.
    pub fn add_json_asset(
        &mut self,
        asset_type: u16,
        name: &str,
        payload: &[u8],
    ) -> Result<(), BendlWriteError> {
        self.add_asset(
            asset_type,
            name,
            payload,
            AddAssetOptions::defaults().json(),
        )
    }

    /// Transition from the asset phase into the stream phase and return
    /// a mutable reference to the inner writer so the caller can
    /// directly write the embedded BEN/XBEN payload.
    ///
    /// Once this method has been called, no further assets may be added.
    /// The caller is responsible for calling [`BendlWriter::end_stream`]
    /// when the payload is complete.
    pub fn begin_stream(&mut self) -> Result<BendlStreamHandle<'_, W>, BendlWriteError> {
        if self.state != WriterState::Assets {
            return Err(BendlWriteError::WrongState {
                expected: "Assets",
                found: if matches!(self.state, WriterState::Streaming) {
                    "Streaming"
                } else {
                    "StreamWritten"
                },
            });
        }

        let stream_offset = self.inner.seek(SeekFrom::Current(0))?;
        self.header.stream_offset = stream_offset;
        self.state = WriterState::Streaming;

        Ok(BendlStreamHandle {
            parent: self,
            start_offset: stream_offset,
        })
    }

    /// Directly write the whole stream region from an in-memory byte
    /// slice. This is a convenience for tests and for tools that already
    /// have the encoded stream bytes on hand.
    pub fn write_stream_bytes(
        &mut self,
        bytes: &[u8],
        sample_count: i64,
    ) -> Result<(), BendlWriteError> {
        let mut handle = self.begin_stream()?;
        handle.write_all(bytes).map_err(BendlWriteError::Io)?;
        handle.finish(sample_count)
    }

    /// Open a BEN assignment stream backed by an
    /// [`crate::io::writer::AssignmentWriter`] and invoke `f` with a
    /// context that can encode assignments into it.
    ///
    /// The context tracks how many `write_assignment` / `write_json_value`
    /// calls the closure makes and records that count as the bundle's
    /// authoritative `sample_count` when the stream is finalized. The
    /// closure is free to short-circuit by returning an error, in which
    /// case the stream phase is abandoned and the error is propagated.
    pub fn write_ben_stream<F>(
        &mut self,
        variant: crate::BenVariant,
        f: F,
    ) -> Result<(), BendlWriteError>
    where
        F: FnOnce(&mut BundleAssignmentStreamCtx<'_>) -> io::Result<()>,
    {
        let mut handle = self.begin_stream()?;
        let mut sample_count: i64 = 0;
        {
            let mut ben = crate::io::writer::AssignmentWriter::new(&mut handle, variant)?;
            {
                let mut ctx = BundleAssignmentStreamCtx {
                    writer: &mut ben,
                    sample_count: &mut sample_count,
                };
                f(&mut ctx)?;
            }
            ben.finish()?;
            // `ben` is dropped here, releasing its borrow on `handle`.
        }
        handle.finish(sample_count)
    }

    /// Open an XBEN assignment stream backed by an
    /// [`crate::io::writer::XZAssignmentWriter`] and invoke `f` with a
    /// context that can encode assignments into it.
    ///
    /// The closure sees the same counting [`BundleAssignmentStreamCtx`]
    /// type used by [`BendlWriter::write_ben_stream`], so callers can be
    /// written to be generic over the assignment container.
    pub fn write_xben_stream<F>(
        &mut self,
        variant: crate::BenVariant,
        f: F,
    ) -> Result<(), BendlWriteError>
    where
        F: FnOnce(&mut BundleAssignmentStreamCtx<'_>) -> io::Result<()>,
    {
        let mut handle = self.begin_stream()?;
        let mut sample_count: i64 = 0;
        {
            let encoder = xz2::write::XzEncoder::new(&mut handle, DEFAULT_XZ_PRESET);
            let mut xben = crate::io::writer::XZAssignmentWriter::new(encoder, variant)?;
            {
                let mut ctx = BundleAssignmentStreamCtx {
                    writer: &mut xben,
                    sample_count: &mut sample_count,
                };
                f(&mut ctx)?;
            }
            xben.finish()?;
            // `xben` is dropped here, which drops its inner `XzEncoder`,
            // which in turn finalizes the xz stream and flushes the last
            // bytes out to `handle`.
        }
        handle.finish(sample_count)
    }

    /// Write the trailing directory, patch the header, and return the
    /// underlying writer.
    pub fn finish(mut self) -> Result<W, BendlWriteError> {
        if matches!(self.state, WriterState::Streaming) {
            return Err(BendlWriteError::WrongState {
                expected: "StreamWritten",
                found: "Streaming",
            });
        }
        let (stream_len, sample_count) =
            if let WriterState::StreamWritten { stream_len, sample_count } = self.state {
                (stream_len, sample_count)
            } else {
                // Assets state: no stream written; treat as empty stream.
                let stream_offset = self.inner.seek(SeekFrom::Current(0))?;
                self.header.stream_offset = stream_offset;
                (0, 0)
            };

        // Position at end of stream (== start of directory).
        let directory_offset = self.header.stream_offset + stream_len;
        self.inner.seek(SeekFrom::Start(directory_offset))?;

        let directory_bytes = encode_directory(&self.entries).map_err(BendlWriteError::Format)?;
        self.inner
            .write_all(&directory_bytes)
            .map_err(BendlWriteError::Io)?;

        let directory_len = directory_bytes.len() as u64;

        // Patch the header.
        self.header.directory_offset = directory_offset;
        self.header.directory_len = directory_len;
        self.header.stream_len = stream_len;
        self.header.sample_count = sample_count;
        self.header.finalized = FINALIZED_YES;
        self.inner.seek(SeekFrom::Start(0))?;
        self.header.write_to(&mut self.inner)?;

        // Flush explicitly; some writers (files) are not flushed on drop.
        self.inner.flush()?;

        self.state = WriterState::Finished;
        Ok(self.inner)
    }

}

/// Mutable handle to the stream region held by a [`BendlWriter`].
///
/// The handle implements `Write` so it can be wrapped in
/// `AssignmentWriter::new(handle, variant)` or
/// `XZAssignmentWriter::new(handle, variant)` directly.
pub struct BendlStreamHandle<'a, W: Write + Seek> {
    parent: &'a mut BendlWriter<W>,
    start_offset: u64,
}

impl<'a, W: Write + Seek> BendlStreamHandle<'a, W> {
    /// Record the sample count and transition the writer out of the
    /// stream phase. Call this after the embedded BEN/XBEN payload has
    /// been written.
    pub fn finish(self, sample_count: i64) -> Result<(), BendlWriteError> {
        let end = self.parent.inner.seek(SeekFrom::Current(0))?;
        let stream_len = end.saturating_sub(self.start_offset);
        self.parent.state = WriterState::StreamWritten {
            stream_len,
            sample_count,
        };
        Ok(())
    }
}

impl<'a, W: Write + Seek> Write for BendlStreamHandle<'a, W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.parent.inner.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.parent.inner.flush()
    }
}

/// Minimal trait that hides the concrete assignment-writer type behind a
/// pair of methods that both [`crate::io::writer::AssignmentWriter`] and
/// [`crate::io::writer::XZAssignmentWriter`] implement.
///
/// The bundle layer uses this to let a single
/// [`BundleAssignmentStreamCtx`] wrap either container.
pub trait BundleAssignmentSink {
    /// Encode one assignment vector.
    fn write_assignment(&mut self, assign_vec: Vec<u16>) -> io::Result<()>;
    /// Encode one JSON assignment record.
    fn write_json_value(&mut self, data: serde_json::Value) -> io::Result<()>;
}

impl<W: Write> BundleAssignmentSink for crate::io::writer::AssignmentWriter<W> {
    fn write_assignment(&mut self, assign_vec: Vec<u16>) -> io::Result<()> {
        crate::io::writer::AssignmentWriter::write_assignment(self, assign_vec)
    }

    fn write_json_value(&mut self, data: serde_json::Value) -> io::Result<()> {
        crate::io::writer::AssignmentWriter::write_json_value(self, data)
    }
}

impl<W: Write> BundleAssignmentSink for crate::io::writer::XZAssignmentWriter<W> {
    fn write_assignment(&mut self, assign_vec: Vec<u16>) -> io::Result<()> {
        crate::io::writer::XZAssignmentWriter::write_assignment(self, assign_vec)
    }

    fn write_json_value(&mut self, data: serde_json::Value) -> io::Result<()> {
        crate::io::writer::XZAssignmentWriter::write_json_value(self, data)
    }
}

/// Closure-side handle passed to [`BendlWriter::write_ben_stream`] and
/// [`BendlWriter::write_xben_stream`].
///
/// Exposes the usual assignment-writing methods while also counting
/// samples so the bundle's header can be patched with an authoritative
/// `sample_count` at stream finalization.
pub struct BundleAssignmentStreamCtx<'a> {
    writer: &'a mut dyn BundleAssignmentSink,
    sample_count: &'a mut i64,
}

impl<'a> BundleAssignmentStreamCtx<'a> {
    /// Encode one assignment vector and bump the sample counter.
    pub fn write_assignment(&mut self, assign_vec: Vec<u16>) -> io::Result<()> {
        self.writer.write_assignment(assign_vec)?;
        *self.sample_count += 1;
        Ok(())
    }

    /// Encode one JSON assignment record and bump the sample counter.
    pub fn write_json_value(&mut self, data: serde_json::Value) -> io::Result<()> {
        self.writer.write_json_value(data)?;
        *self.sample_count += 1;
        Ok(())
    }

    /// Number of samples the closure has written so far.
    pub fn sample_count(&self) -> i64 {
        *self.sample_count
    }
}

/// Errors produced by the bundle writer.
#[derive(Debug, Error)]
pub enum BendlWriteError {
    /// A new asset's name collides with an existing one.
    #[error("duplicate asset name: {0:?}")]
    DuplicateName(String),

    /// A second singleton asset of this type was requested.
    #[error("duplicate singleton asset type: {0}")]
    DuplicateSingletonType(u16),

    /// A singleton asset was added under the wrong standardized name.
    #[error("asset type {asset_type} must use standardized name {expected:?}, got {found:?}")]
    WrongCanonicalName {
        /// The asset type whose standardized name was violated.
        asset_type: u16,
        /// The standardized name the caller should have used.
        expected: String,
        /// The name the caller actually provided.
        found: String,
    },

    /// An asset was added after the stream phase began.
    #[error("cannot add assets after the stream region has been opened")]
    AssetsAfterStream,

    /// Tried to append to a bundle that is not finalized.
    #[error("cannot append to a bundle whose header does not have complete == 1")]
    BundleIncomplete,

    /// The writer was asked to perform an operation in the wrong state.
    #[error("writer is in state {found}, expected {expected}")]
    WrongState {
        /// The state the operation expected.
        expected: &'static str,
        /// The state the writer was actually in.
        found: &'static str,
    },

    /// A format-layer error escaped while encoding the directory table.
    #[error(transparent)]
    Format(#[from] BendlFormatError),

    /// An underlying I/O error.
    #[error(transparent)]
    Io(#[from] io::Error),
}

// ---------------------------------------------------------------------------
// Append path
// ---------------------------------------------------------------------------

/// Post-finalize appender that grows an existing `.bendl` file with new
/// assets without rewriting the assignment stream.
///
/// The workflow is:
///
/// 1. [`BendlAppender::open`] opens a finalized bundle and loads its
///    directory into memory.
/// 2. [`BendlAppender::add_asset`] (or [`BendlAppender::add_json_asset`])
///    validates and buffers each new asset. Validation happens up front,
///    so duplicate singletons or names are rejected **before** any file
///    mutation, and a rejected add_asset leaves the file unchanged.
/// 3. [`BendlAppender::commit`] compresses the buffered assets (if any),
///    truncates the file at the old directory offset, writes the new
///    asset payloads, writes a new directory at the new EOF, and patches
///    the header.
///
/// A [`BendlAppender`] that is dropped without calling `commit` leaves
/// the underlying file unchanged.
pub struct BendlAppender<W: Read + Write + Seek + BendlTruncate> {
    inner: W,
    header: BendlHeader,
    existing_entries: Vec<BendlDirectoryEntry>,
    existing_names: HashSet<String>,
    existing_singleton_types: HashSet<u16>,
    pending: Vec<PendingAsset>,
    pending_names: HashSet<String>,
    pending_singleton_types: HashSet<u16>,
}

/// An asset queued for append but not yet written to disk.
struct PendingAsset {
    asset_type: u16,
    name: String,
    /// Raw payload bytes as provided by the caller.
    raw_payload: Vec<u8>,
    /// Resolved compression decision: `true` means compress, `false` means raw.
    compress: bool,
    is_json: bool,
    checksum: Option<Vec<u8>>,
}

impl<W: Read + Write + Seek + BendlTruncate> BendlAppender<W> {
    /// Open a finalized bundle for append.
    ///
    /// Returns [`BendlWriteError::BundleIncomplete`] if the header's
    /// `complete` flag is not set — append is unsafe on unfinalized
    /// bundles because the stream region has no authoritative end.
    pub fn open(mut inner: W) -> Result<Self, BendlWriteError> {
        inner.seek(SeekFrom::Start(0))?;
        let header = BendlHeader::read_from(&mut inner).map_err(BendlWriteError::Format)?;
        if !header.is_finalized() {
            return Err(BendlWriteError::BundleIncomplete);
        }
        if header.directory_offset == 0 || header.directory_len == 0 {
            return Err(BendlWriteError::BundleIncomplete);
        }

        inner.seek(SeekFrom::Start(header.directory_offset))?;
        let mut bounded = (&mut inner).take(header.directory_len);
        let existing_entries = read_directory(&mut bounded).map_err(BendlWriteError::Format)?;
        let remaining = bounded.limit();
        if remaining != 0 {
            return Err(BendlWriteError::Format(
                BendlFormatError::TrailingDirectoryBytes { remaining },
            ));
        }
        super::reader::validate_directory_entries(&existing_entries).map_err(|e| {
            BendlWriteError::Format(BendlFormatError::MalformedDirectory(e.to_string()))
        })?;

        let mut existing_names = HashSet::new();
        let mut existing_singleton_types = HashSet::new();
        for entry in &existing_entries {
            existing_names.insert(entry.name.clone());
            if standardized_name_for(entry.asset_type).is_some() {
                existing_singleton_types.insert(entry.asset_type);
            }
        }

        Ok(BendlAppender {
            inner,
            header,
            existing_entries,
            existing_names,
            existing_singleton_types,
            pending: Vec::new(),
            pending_names: HashSet::new(),
            pending_singleton_types: HashSet::new(),
        })
    }

    /// Enqueue a new asset for append.
    ///
    /// This validates the new asset against both the loaded directory
    /// and any previously-enqueued pending assets. If validation fails,
    /// the pending list is unchanged and no bytes have been written to
    /// the file.
    pub fn add_asset(
        &mut self,
        asset_type: u16,
        name: &str,
        payload: &[u8],
        options: AddAssetOptions,
    ) -> Result<(), BendlWriteError> {
        // Canonical-name rule.
        if let Some(canonical) = standardized_name_for(asset_type) {
            if name != canonical {
                return Err(BendlWriteError::WrongCanonicalName {
                    asset_type,
                    expected: canonical.to_string(),
                    found: name.to_string(),
                });
            }
            if self.existing_singleton_types.contains(&asset_type)
                || self.pending_singleton_types.contains(&asset_type)
            {
                return Err(BendlWriteError::DuplicateSingletonType(asset_type));
            }
        }

        // Uniqueness rule against both existing and pending assets.
        if self.existing_names.contains(name) || self.pending_names.contains(name) {
            return Err(BendlWriteError::DuplicateName(name.to_string()));
        }

        let compress = options
            .compress
            .unwrap_or_else(|| default_compresses_by_type(asset_type));

        self.pending_names.insert(name.to_string());
        if standardized_name_for(asset_type).is_some() {
            self.pending_singleton_types.insert(asset_type);
        }
        self.pending.push(PendingAsset {
            asset_type,
            name: name.to_string(),
            raw_payload: payload.to_vec(),
            compress,
            is_json: options.is_json,
            checksum: options.checksum,
        });
        Ok(())
    }

    /// Convenience wrapper around [`add_asset`] for JSON-encoded assets.
    pub fn add_json_asset(
        &mut self,
        asset_type: u16,
        name: &str,
        payload: &[u8],
    ) -> Result<(), BendlWriteError> {
        self.add_asset(
            asset_type,
            name,
            payload,
            AddAssetOptions::defaults().json(),
        )
    }

    /// Commit all pending appends.
    ///
    /// This compresses any buffered payloads that need it (entirely in
    /// memory), then performs the file mutation in a single burst:
    /// truncate at the old directory offset, write new payloads, write
    /// a new directory, and patch the header.
    ///
    /// If compression fails, the file is left unchanged.
    pub fn commit(mut self) -> Result<W, BendlWriteError> {
        // If nothing was enqueued, commit is a no-op — return the file untouched.
        if self.pending.is_empty() {
            return Ok(self.inner);
        }

        // Phase 1: compress any pending payloads and build new entries with
        // placeholder offsets. Do this entirely in memory so failures here
        // leave the file untouched.
        struct EncodedPending {
            asset_type: u16,
            name: String,
            bytes: Vec<u8>,
            asset_flags: u16,
            checksum: Option<Vec<u8>>,
        }

        let mut encoded: Vec<EncodedPending> = Vec::with_capacity(self.pending.len());
        for asset in self.pending.drain(..) {
            let bytes = if asset.compress {
                let mut encoder = XzEncoder::new(Vec::new(), DEFAULT_XZ_PRESET);
                encoder.write_all(&asset.raw_payload)?;
                encoder.finish()?
            } else {
                asset.raw_payload
            };

            let mut asset_flags: u16 = 0;
            if asset.is_json {
                asset_flags |= ASSET_FLAG_JSON;
            }
            if asset.compress {
                asset_flags |= ASSET_FLAG_XZ;
            }
            if asset.checksum.is_some() {
                asset_flags |= crate::io::bundle::format::ASSET_FLAG_CHECKSUM;
            }

            encoded.push(EncodedPending {
                asset_type: asset.asset_type,
                name: asset.name,
                bytes,
                asset_flags,
                checksum: asset.checksum,
            });
        }

        // Phase 2: file mutation. From this point forward, a failure
        // leaves the bundle in a damaged state. We do everything in the
        // order (truncate, write payloads, write directory, patch header)
        // so that even if we crash mid-way, the header still points at
        // the old directory until the very last write.
        let old_directory_offset = self.header.directory_offset;

        // Truncate at the old directory offset.
        self.inner.truncate_at(old_directory_offset)?;
        self.inner.seek(SeekFrom::Start(old_directory_offset))?;

        // Compute new entries with real offsets as we write.
        let mut new_entries: Vec<BendlDirectoryEntry> =
            Vec::with_capacity(self.existing_entries.len() + encoded.len());
        new_entries.extend(self.existing_entries.iter().cloned());

        for enc in encoded {
            let payload_offset = self.inner.seek(SeekFrom::Current(0))?;
            self.inner.write_all(&enc.bytes)?;
            new_entries.push(BendlDirectoryEntry {
                asset_type: enc.asset_type,
                asset_flags: enc.asset_flags,
                name: enc.name,
                payload_offset,
                payload_len: enc.bytes.len() as u64,
                checksum: enc.checksum,
            });
        }

        // Write the new directory at the new EOF.
        let new_directory_offset = self.inner.seek(SeekFrom::Current(0))?;
        let directory_bytes = encode_directory(&new_entries).map_err(BendlWriteError::Format)?;
        self.inner.write_all(&directory_bytes)?;
        let new_directory_len = directory_bytes.len() as u64;

        // Patch the header.
        self.header.directory_offset = new_directory_offset;
        self.header.directory_len = new_directory_len;
        self.inner.seek(SeekFrom::Start(0))?;
        self.header.write_to(&mut self.inner)?;
        self.inner.flush()?;

        Ok(self.inner)
    }

    /// Release the underlying reader without committing any pending
    /// appends. The file is unchanged.
    pub fn abort(self) -> W {
        self.inner
    }
}
