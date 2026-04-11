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
    canonical_name_for, default_compresses_by_type, encode_directory, read_directory,
    AssignmentFormat, BendlDirectoryEntry, BendlFormatError, BendlHeader, ASSET_FLAG_JSON,
    ASSET_FLAG_XZ, COMPLETE_YES, DEFAULT_XZ_PRESET, HEADER_SIZE,
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
        if vec.len() > target {
            vec.truncate(target);
        } else if vec.len() < target {
            vec.resize(target, 0);
        }
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
        if let Some(canonical) = canonical_name_for(asset_type) {
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
            if canonical_name_for(asset_type).is_some() {
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
            encoder
                .write_all(payload)
                .map_err(BendlWriteError::Io)?;
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
            asset_flags |=
                crate::io::bundle::format::ASSET_FLAG_CHECKSUM;
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
        self.add_asset(asset_type, name, payload, AddAssetOptions::defaults().json())
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
                found: self.state_name(),
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
            let mut ben =
                crate::io::writer::AssignmentWriter::new(&mut handle, variant)?;
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
            let mut xben =
                crate::io::writer::XZAssignmentWriter::new(encoder, variant)?;
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
        let (stream_len, sample_count) = match self.state {
            WriterState::StreamWritten {
                stream_len,
                sample_count,
            } => (stream_len, sample_count),
            // Allow finalizing a bundle that has no stream at all (useful
            // for asset-only bundles), treating the stream as empty.
            WriterState::Assets => {
                let stream_offset = self.inner.seek(SeekFrom::Current(0))?;
                self.header.stream_offset = stream_offset;
                (0, 0)
            }
            WriterState::Streaming => {
                return Err(BendlWriteError::WrongState {
                    expected: "StreamWritten",
                    found: "Streaming",
                });
            }
            WriterState::Finished => {
                return Err(BendlWriteError::WrongState {
                    expected: "StreamWritten",
                    found: "Finished",
                });
            }
        };

        // Position at end of stream (== start of directory).
        let directory_offset = self.header.stream_offset + stream_len;
        self.inner.seek(SeekFrom::Start(directory_offset))?;

        let directory_bytes = encode_directory(&self.entries)
            .map_err(BendlWriteError::Format)?;
        self.inner
            .write_all(&directory_bytes)
            .map_err(BendlWriteError::Io)?;

        let directory_len = directory_bytes.len() as u64;

        // Patch the header.
        self.header.directory_offset = directory_offset;
        self.header.directory_len = directory_len;
        self.header.stream_len = stream_len;
        self.header.sample_count = sample_count;
        self.header.complete = COMPLETE_YES;
        self.inner.seek(SeekFrom::Start(0))?;
        self.header.write_to(&mut self.inner)?;

        // Flush explicitly; some writers (files) are not flushed on drop.
        self.inner.flush()?;

        self.state = WriterState::Finished;
        Ok(self.inner)
    }

    fn state_name(&self) -> &'static str {
        match self.state {
            WriterState::Assets => "Assets",
            WriterState::Streaming => "Streaming",
            WriterState::StreamWritten { .. } => "StreamWritten",
            WriterState::Finished => "Finished",
        }
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

    /// A singleton asset was added under the wrong canonical name.
    #[error(
        "asset type {asset_type} must use canonical name {expected:?}, got {found:?}"
    )]
    WrongCanonicalName {
        /// The asset type whose canonical name was violated.
        asset_type: u16,
        /// The canonical name the caller should have used.
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
        if !header.is_complete() {
            return Err(BendlWriteError::BundleIncomplete);
        }
        if header.directory_offset == 0 || header.directory_len == 0 {
            return Err(BendlWriteError::BundleIncomplete);
        }

        inner.seek(SeekFrom::Start(header.directory_offset))?;
        let existing_entries =
            read_directory(&mut inner).map_err(BendlWriteError::Format)?;

        let mut existing_names = HashSet::new();
        let mut existing_singleton_types = HashSet::new();
        for entry in &existing_entries {
            existing_names.insert(entry.name.clone());
            if canonical_name_for(entry.asset_type).is_some() {
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

    /// The currently loaded (pre-append) directory entries.
    pub fn existing_assets(&self) -> &[BendlDirectoryEntry] {
        &self.existing_entries
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
        if let Some(canonical) = canonical_name_for(asset_type) {
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
        if canonical_name_for(asset_type).is_some() {
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
        self.add_asset(asset_type, name, payload, AddAssetOptions::defaults().json())
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
                asset_flags |=
                    crate::io::bundle::format::ASSET_FLAG_CHECKSUM;
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
        let directory_bytes =
            encode_directory(&new_entries).map_err(BendlWriteError::Format)?;
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Read};

    use super::*;
    use crate::io::bundle::format::{
        ASSET_FLAG_XZ, ASSET_TYPE_CUSTOM, ASSET_TYPE_GRAPH, ASSET_TYPE_METADATA,
    };
    use crate::io::bundle::reader::BendlReader;

    fn make_buffer() -> Cursor<Vec<u8>> {
        Cursor::new(Vec::new())
    }

    #[test]
    fn minimal_bundle_round_trip_through_reader() {
        let mut writer = BendlWriter::new(make_buffer(), AssignmentFormat::Ben).unwrap();
        writer
            .add_json_asset(
                ASSET_TYPE_METADATA,
                "metadata.json",
                br#"{"note":"hello"}"#,
            )
            .unwrap();
        let stream_bytes = b"STANDARD BEN FILE\x00\x01fake".to_vec();
        writer.write_stream_bytes(&stream_bytes, 7).unwrap();
        let buf = writer.finish().unwrap().into_inner();

        let mut reader = BendlReader::open(Cursor::new(buf)).unwrap();
        assert!(reader.is_complete());
        assert_eq!(reader.sample_count(), Some(7));
        assert_eq!(reader.assignment_format(), Some(AssignmentFormat::Ben));
        assert_eq!(reader.assets().len(), 1);

        let entry = reader
            .find_asset_by_type(ASSET_TYPE_METADATA)
            .cloned()
            .expect("metadata entry present");
        assert_eq!(entry.name, "metadata.json");
        assert_eq!(entry.asset_flags & ASSET_FLAG_XZ, 0);
        let meta_bytes = reader.asset_bytes(&entry).unwrap();
        assert_eq!(meta_bytes, br#"{"note":"hello"}"#);

        let mut stream_buf = Vec::new();
        reader
            .assignment_stream_reader()
            .unwrap()
            .read_to_end(&mut stream_buf)
            .unwrap();
        assert_eq!(stream_buf, stream_bytes);
    }

    #[test]
    fn graph_asset_is_compressed_by_default() {
        let graph = br#"{"nodes":[0,1,2,3,4,5,6,7,8,9],"edges":[[0,1],[1,2],[2,3],[3,4]]}"#;
        let mut writer = BendlWriter::new(make_buffer(), AssignmentFormat::Ben).unwrap();
        writer
            .add_json_asset(ASSET_TYPE_GRAPH, "graph.json", graph)
            .unwrap();
        writer.write_stream_bytes(b"STANDARD BEN FILE\x00fake", 1).unwrap();
        let buf = writer.finish().unwrap().into_inner();

        let mut reader = BendlReader::open(Cursor::new(buf)).unwrap();
        let entry = reader
            .find_asset_by_type(ASSET_TYPE_GRAPH)
            .cloned()
            .expect("graph entry present");
        assert_ne!(entry.asset_flags & ASSET_FLAG_XZ, 0);
        // Compressed size should differ from the raw size for a non-trivial
        // JSON payload. For very short payloads xz actually inflates the
        // bytes, so this just checks the size is non-zero and different.
        assert_ne!(entry.payload_len, graph.len() as u64);

        // Decoded bytes round-trip.
        let decoded = reader.asset_bytes(&entry).unwrap();
        assert_eq!(decoded, graph);
    }

    #[test]
    fn graph_asset_can_be_forced_raw() {
        let graph = br#"{"nodes":[0,1,2]}"#;
        let mut writer = BendlWriter::new(make_buffer(), AssignmentFormat::Ben).unwrap();
        writer
            .add_asset(
                ASSET_TYPE_GRAPH,
                "graph.json",
                graph,
                AddAssetOptions::defaults().json().raw(),
            )
            .unwrap();
        writer.write_stream_bytes(b"STANDARD BEN FILE\x00fake", 1).unwrap();
        let buf = writer.finish().unwrap().into_inner();

        let reader = BendlReader::open(Cursor::new(buf)).unwrap();
        let entry = reader
            .find_asset_by_type(ASSET_TYPE_GRAPH)
            .expect("graph entry present");
        assert_eq!(entry.asset_flags & ASSET_FLAG_XZ, 0);
        assert_eq!(entry.payload_len, graph.len() as u64);
    }

    #[test]
    fn writer_rejects_second_graph() {
        let mut writer = BendlWriter::new(make_buffer(), AssignmentFormat::Ben).unwrap();
        writer
            .add_json_asset(ASSET_TYPE_GRAPH, "graph.json", b"{}")
            .unwrap();
        let err = writer
            .add_json_asset(ASSET_TYPE_GRAPH, "graph.json", b"{}")
            .unwrap_err();
        assert!(matches!(err, BendlWriteError::DuplicateSingletonType(t) if t == ASSET_TYPE_GRAPH));
    }

    #[test]
    fn writer_rejects_wrong_canonical_name_for_singleton() {
        let mut writer = BendlWriter::new(make_buffer(), AssignmentFormat::Ben).unwrap();
        let err = writer
            .add_json_asset(ASSET_TYPE_GRAPH, "graph_but_wrong_name.json", b"{}")
            .unwrap_err();
        assert!(matches!(
            err,
            BendlWriteError::WrongCanonicalName { asset_type: ASSET_TYPE_GRAPH, .. }
        ));
    }

    #[test]
    fn writer_rejects_duplicate_custom_name() {
        let mut writer = BendlWriter::new(make_buffer(), AssignmentFormat::Ben).unwrap();
        writer
            .add_asset(
                ASSET_TYPE_CUSTOM,
                "blob",
                b"first",
                AddAssetOptions::defaults(),
            )
            .unwrap();
        let err = writer
            .add_asset(
                ASSET_TYPE_CUSTOM,
                "blob",
                b"second",
                AddAssetOptions::defaults(),
            )
            .unwrap_err();
        assert!(matches!(err, BendlWriteError::DuplicateName(ref n) if n == "blob"));
    }

    #[test]
    fn writer_rejects_asset_added_after_stream_begins() {
        let mut writer = BendlWriter::new(make_buffer(), AssignmentFormat::Ben).unwrap();
        {
            let mut handle = writer.begin_stream().unwrap();
            handle.write_all(b"STANDARD BEN FILE\x00fake").unwrap();
            handle.finish(1).unwrap();
        }
        let err = writer
            .add_json_asset(ASSET_TYPE_METADATA, "metadata.json", b"{}")
            .unwrap_err();
        assert!(matches!(err, BendlWriteError::AssetsAfterStream));
    }

    #[test]
    fn asset_only_bundle_finalizes_with_empty_stream() {
        let mut writer = BendlWriter::new(make_buffer(), AssignmentFormat::Ben).unwrap();
        writer
            .add_json_asset(ASSET_TYPE_METADATA, "metadata.json", b"{}")
            .unwrap();
        let buf = writer.finish().unwrap().into_inner();

        let reader = BendlReader::open(Cursor::new(buf)).unwrap();
        assert!(reader.is_complete());
        assert_eq!(reader.sample_count(), Some(0));
        assert_eq!(reader.header().stream_len, 0);
    }

    #[test]
    fn finalized_directory_lives_at_eof() {
        let mut writer = BendlWriter::new(make_buffer(), AssignmentFormat::Ben).unwrap();
        writer
            .add_json_asset(ASSET_TYPE_METADATA, "metadata.json", b"{}")
            .unwrap();
        writer.write_stream_bytes(b"STANDARD BEN FILE\x00fake", 1).unwrap();
        let buf = writer.finish().unwrap().into_inner();

        let reader = BendlReader::open(Cursor::new(buf.clone())).unwrap();
        let header = reader.header();
        let file_len = buf.len() as u64;
        assert_eq!(header.directory_offset + header.directory_len, file_len);
        // Stream ends where directory begins.
        assert_eq!(header.stream_offset + header.stream_len, header.directory_offset);
    }

    // -----------------------------------------------------------------------
    // Append-path tests
    // -----------------------------------------------------------------------

    /// Build a finalized bundle with a single `metadata.json` asset and
    /// a short fake stream, then return both the bytes and the byte
    /// range (offset, len) occupied by the stream region.
    fn build_base_bundle() -> (Vec<u8>, (u64, u64)) {
        let mut writer = BendlWriter::new(make_buffer(), AssignmentFormat::Ben).unwrap();
        writer
            .add_json_asset(ASSET_TYPE_METADATA, "metadata.json", b"{\"version\":1}")
            .unwrap();
        let stream = b"STANDARD BEN FILE\x00\x01\x02\x03\x04\x05stream bytes";
        writer.write_stream_bytes(stream, 3).unwrap();
        let buf = writer.finish().unwrap().into_inner();

        let reader = BendlReader::open(Cursor::new(buf.clone())).unwrap();
        let range = (reader.header().stream_offset, reader.header().stream_len);
        (buf, range)
    }

    #[test]
    fn append_adds_new_asset_and_preserves_old_entries() {
        let (bundle, _) = build_base_bundle();

        let mut appender = BendlAppender::open(Cursor::new(bundle)).unwrap();
        appender
            .add_json_asset(ASSET_TYPE_GRAPH, "graph.json", b"{\"nodes\":[]}")
            .unwrap();
        let buf = appender.commit().unwrap().into_inner();

        let reader = BendlReader::open(Cursor::new(buf)).unwrap();
        assert_eq!(reader.assets().len(), 2);
        assert!(reader.find_asset_by_name("metadata.json").is_some());
        assert!(reader.find_asset_by_name("graph.json").is_some());
        // Finalized bundle invariants still hold.
        assert!(reader.is_complete());
        assert_eq!(reader.sample_count(), Some(3));
    }

    #[test]
    fn append_leaves_stream_bytes_byte_for_byte_unchanged() {
        let (bundle, (stream_offset, stream_len)) = build_base_bundle();
        let original_stream_bytes = bundle
            [stream_offset as usize..(stream_offset + stream_len) as usize]
            .to_vec();

        let mut appender = BendlAppender::open(Cursor::new(bundle)).unwrap();
        appender
            .add_asset(
                ASSET_TYPE_CUSTOM,
                "blob",
                b"appended custom bytes",
                AddAssetOptions::defaults(),
            )
            .unwrap();
        let buf = appender.commit().unwrap().into_inner();

        // Read back the new header to locate the stream region, then
        // confirm the stream bytes are byte-identical to the original.
        let reader = BendlReader::open(Cursor::new(buf.clone())).unwrap();
        let (off, len) = (reader.header().stream_offset, reader.header().stream_len);
        let appended_stream_bytes = buf[off as usize..(off + len) as usize].to_vec();
        assert_eq!(appended_stream_bytes, original_stream_bytes);
        // Stream offset should not have moved either.
        assert_eq!(off, stream_offset);
        assert_eq!(len, stream_len);
    }

    #[test]
    fn append_preserves_existing_entries_payload_offsets() {
        let (bundle, _) = build_base_bundle();

        // Snapshot the metadata entry's payload_offset before append.
        let reader = BendlReader::open(Cursor::new(bundle.clone())).unwrap();
        let old_offset = reader
            .find_asset_by_name("metadata.json")
            .unwrap()
            .payload_offset;
        drop(reader);

        let mut appender = BendlAppender::open(Cursor::new(bundle)).unwrap();
        appender
            .add_json_asset(ASSET_TYPE_GRAPH, "graph.json", b"{\"nodes\":[0,1,2,3,4,5]}")
            .unwrap();
        let buf = appender.commit().unwrap().into_inner();

        let reader = BendlReader::open(Cursor::new(buf)).unwrap();
        let new_offset = reader
            .find_asset_by_name("metadata.json")
            .unwrap()
            .payload_offset;
        assert_eq!(old_offset, new_offset, "existing asset offset must not move");
    }

    #[test]
    fn append_rejects_duplicate_singleton_without_touching_file() {
        let (bundle, _) = build_base_bundle();
        let bundle_before = bundle.clone();

        let mut appender = BendlAppender::open(Cursor::new(bundle)).unwrap();
        let err = appender
            .add_json_asset(ASSET_TYPE_METADATA, "metadata.json", b"{\"new\":true}")
            .unwrap_err();
        assert!(matches!(err, BendlWriteError::DuplicateSingletonType(_)));

        // Abort and confirm the file is byte-for-byte unchanged.
        let buf = appender.abort().into_inner();
        assert_eq!(buf, bundle_before);
    }

    #[test]
    fn append_rejects_duplicate_custom_name_without_touching_file() {
        // Start from a bundle containing a custom asset named "blob", then
        // try to append another "blob".
        let mut writer = BendlWriter::new(make_buffer(), AssignmentFormat::Ben).unwrap();
        writer
            .add_asset(
                ASSET_TYPE_CUSTOM,
                "blob",
                b"original",
                AddAssetOptions::defaults(),
            )
            .unwrap();
        writer
            .write_stream_bytes(b"STANDARD BEN FILE\x00fake", 1)
            .unwrap();
        let bundle = writer.finish().unwrap().into_inner();
        let bundle_before = bundle.clone();

        let mut appender = BendlAppender::open(Cursor::new(bundle)).unwrap();
        let err = appender
            .add_asset(
                ASSET_TYPE_CUSTOM,
                "blob",
                b"dup",
                AddAssetOptions::defaults(),
            )
            .unwrap_err();
        assert!(matches!(err, BendlWriteError::DuplicateName(ref n) if n == "blob"));

        let buf = appender.abort().into_inner();
        assert_eq!(buf, bundle_before);
    }

    #[test]
    fn append_rejects_wrong_canonical_name_without_touching_file() {
        let (bundle, _) = build_base_bundle();
        let bundle_before = bundle.clone();

        let mut appender = BendlAppender::open(Cursor::new(bundle)).unwrap();
        let err = appender
            .add_json_asset(ASSET_TYPE_GRAPH, "not_graph.json", b"{}")
            .unwrap_err();
        assert!(matches!(
            err,
            BendlWriteError::WrongCanonicalName { asset_type: ASSET_TYPE_GRAPH, .. }
        ));

        let buf = appender.abort().into_inner();
        assert_eq!(buf, bundle_before);
    }

    #[test]
    fn append_rejects_incomplete_bundle() {
        // Construct a minimal incomplete bundle: just the provisional
        // header and some stream bytes, no directory.
        use crate::io::bundle::format::{BENDL_MAGIC, BENDL_MAJOR_VERSION, BENDL_MINOR_VERSION,
            COMPLETE_NO};
        let header = BendlHeader {
            magic: BENDL_MAGIC,
            major_version: BENDL_MAJOR_VERSION,
            minor_version: BENDL_MINOR_VERSION,
            complete: COMPLETE_NO,
            assignment_format: AssignmentFormat::Ben.to_u8(),
            reserved_0: 0,
            flags: 0,
            directory_offset: 0,
            directory_len: 0,
            stream_offset: HEADER_SIZE as u64,
            stream_len: 0,
            sample_count: -1,
        };
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&header.to_bytes());
        bytes.extend_from_slice(b"STANDARD BEN FILE\x00fake");

        match BendlAppender::open(Cursor::new(bytes)) {
            Err(BendlWriteError::BundleIncomplete) => {}
            Err(other) => panic!("expected BundleIncomplete, got {other:?}"),
            Ok(_) => panic!("expected error, got Ok"),
        }
    }

    #[test]
    fn append_multiple_assets_in_one_commit() {
        let (bundle, _) = build_base_bundle();
        let mut appender = BendlAppender::open(Cursor::new(bundle)).unwrap();
        appender
            .add_json_asset(ASSET_TYPE_GRAPH, "graph.json", b"{\"n\":[0,1,2]}")
            .unwrap();
        appender
            .add_asset(
                ASSET_TYPE_CUSTOM,
                "blob1",
                b"blob one",
                AddAssetOptions::defaults(),
            )
            .unwrap();
        appender
            .add_asset(
                ASSET_TYPE_CUSTOM,
                "blob2",
                b"blob two",
                AddAssetOptions::defaults(),
            )
            .unwrap();
        let buf = appender.commit().unwrap().into_inner();

        let mut reader = BendlReader::open(Cursor::new(buf)).unwrap();
        assert_eq!(reader.assets().len(), 4);
        // Round-trip the appended graph through the reader to confirm
        // compression happened and decodes cleanly.
        let graph_entry = reader
            .find_asset_by_name("graph.json")
            .cloned()
            .expect("graph entry present");
        assert_ne!(graph_entry.asset_flags & ASSET_FLAG_XZ, 0);
        let graph_bytes = reader.asset_bytes(&graph_entry).unwrap();
        assert_eq!(graph_bytes, b"{\"n\":[0,1,2]}");
    }

    #[test]
    fn append_rejects_conflicting_pending_additions() {
        let (bundle, _) = build_base_bundle();
        let bundle_before = bundle.clone();

        let mut appender = BendlAppender::open(Cursor::new(bundle)).unwrap();
        appender
            .add_asset(
                ASSET_TYPE_CUSTOM,
                "new_blob",
                b"a",
                AddAssetOptions::defaults(),
            )
            .unwrap();
        let err = appender
            .add_asset(
                ASSET_TYPE_CUSTOM,
                "new_blob",
                b"b",
                AddAssetOptions::defaults(),
            )
            .unwrap_err();
        assert!(matches!(err, BendlWriteError::DuplicateName(_)));

        let buf = appender.abort().into_inner();
        assert_eq!(buf, bundle_before);
    }

    // -------- Phase 4: assignment-stream integration tests --------

    #[test]
    fn write_ben_stream_round_trips_through_assignment_reader() {
        use crate::io::bundle::reader::BundleAssignmentReader;
        use crate::BenVariant;

        let samples: Vec<Vec<u16>> = vec![
            vec![0, 0, 1, 1, 2, 2],
            vec![0, 1, 1, 1, 2, 2],
            vec![0, 1, 1, 1, 2, 2], // repeat
            vec![1, 1, 1, 1, 2, 2],
        ];

        let mut writer = BendlWriter::new(make_buffer(), AssignmentFormat::Ben).unwrap();
        writer
            .write_ben_stream(BenVariant::MkvChain, |ctx| {
                for s in &samples {
                    ctx.write_assignment(s.clone())?;
                }
                Ok(())
            })
            .unwrap();
        let buf = writer.finish().unwrap().into_inner();

        let mut reader = BendlReader::open(Cursor::new(buf)).unwrap();
        assert!(reader.is_complete());
        // Four write_assignment calls → sample_count == 4.
        assert_eq!(reader.sample_count(), Some(samples.len() as i64));
        assert_eq!(reader.assignment_format(), Some(AssignmentFormat::Ben));

        let decoder = reader.open_assignment_reader().unwrap();
        let inner = match decoder {
            BundleAssignmentReader::Ben(r) => r,
            BundleAssignmentReader::Xben(_) => panic!("expected Ben reader"),
        };
        let decoded: Vec<Vec<u16>> = inner
            .silent(true)
            .flat_map(|r| {
                let (assign, count) = r.unwrap();
                std::iter::repeat(assign).take(count as usize)
            })
            .collect();
        assert_eq!(decoded, samples);
    }

    #[test]
    fn write_xben_stream_round_trips_through_assignment_reader() {
        use crate::io::bundle::reader::BundleAssignmentReader;
        use crate::BenVariant;

        let samples: Vec<Vec<u16>> = vec![
            vec![0, 1, 2, 3, 4, 5],
            vec![0, 1, 2, 3, 4, 5], // repeat
            vec![1, 1, 2, 3, 4, 5],
            vec![1, 1, 2, 3, 4, 4],
        ];

        let mut writer = BendlWriter::new(make_buffer(), AssignmentFormat::Xben).unwrap();
        writer
            .write_xben_stream(BenVariant::MkvChain, |ctx| {
                for s in &samples {
                    ctx.write_assignment(s.clone())?;
                }
                Ok(())
            })
            .unwrap();
        let buf = writer.finish().unwrap().into_inner();

        let mut reader = BendlReader::open(Cursor::new(buf)).unwrap();
        assert!(reader.is_complete());
        assert_eq!(reader.sample_count(), Some(samples.len() as i64));
        assert_eq!(reader.assignment_format(), Some(AssignmentFormat::Xben));

        let decoder = reader.open_assignment_reader().unwrap();
        let inner = match decoder {
            BundleAssignmentReader::Xben(r) => r,
            BundleAssignmentReader::Ben(_) => panic!("expected Xben reader"),
        };
        let decoded: Vec<Vec<u16>> = inner
            .silent(true)
            .flat_map(|r| {
                let (assign, count) = r.unwrap();
                std::iter::repeat(assign).take(count as usize)
            })
            .collect();
        assert_eq!(decoded, samples);
    }

    #[test]
    fn write_ben_stream_alongside_front_loaded_asset() {
        use crate::io::bundle::reader::BundleAssignmentReader;
        use crate::BenVariant;

        let graph = br#"{"nodes":[0,1,2],"edges":[[0,1],[1,2]]}"#;
        let samples: Vec<Vec<u16>> = vec![vec![0, 1, 1, 2], vec![0, 1, 2, 2]];

        let mut writer = BendlWriter::new(make_buffer(), AssignmentFormat::Ben).unwrap();
        writer
            .add_json_asset(ASSET_TYPE_GRAPH, "graph.json", graph)
            .unwrap();
        writer
            .write_ben_stream(BenVariant::Standard, |ctx| {
                for s in &samples {
                    ctx.write_assignment(s.clone())?;
                }
                Ok(())
            })
            .unwrap();
        let buf = writer.finish().unwrap().into_inner();

        let mut reader = BendlReader::open(Cursor::new(buf)).unwrap();
        assert_eq!(reader.sample_count(), Some(samples.len() as i64));

        // Front-loaded graph asset survives round trip through xz.
        let entry = reader
            .find_asset_by_type(ASSET_TYPE_GRAPH)
            .cloned()
            .expect("graph asset present");
        assert_ne!(entry.asset_flags & ASSET_FLAG_XZ, 0);
        let decoded_graph = reader.asset_bytes(&entry).unwrap();
        assert_eq!(decoded_graph, graph);

        // Assignment stream is still intact after pulling asset bytes.
        let decoder = reader.open_assignment_reader().unwrap();
        let inner = match decoder {
            BundleAssignmentReader::Ben(r) => r,
            BundleAssignmentReader::Xben(_) => panic!("expected Ben reader"),
        };
        let decoded: Vec<Vec<u16>> = inner
            .silent(true)
            .flat_map(|r| {
                let (assign, count) = r.unwrap();
                std::iter::repeat(assign).take(count as usize)
            })
            .collect();
        assert_eq!(decoded, samples);
    }

    #[test]
    fn open_assignment_reader_rejects_mismatched_format() {
        // Build a BEN bundle and open a reader, and verify the is_ben/is_xben
        // discriminators reflect the header.
        use crate::io::bundle::reader::BundleAssignmentReader;
        use crate::BenVariant;

        let mut writer = BendlWriter::new(make_buffer(), AssignmentFormat::Ben).unwrap();
        writer
            .write_ben_stream(BenVariant::Standard, |ctx| {
                ctx.write_assignment(vec![0, 1])?;
                Ok(())
            })
            .unwrap();
        let buf = writer.finish().unwrap().into_inner();

        let mut reader = BendlReader::open(Cursor::new(buf)).unwrap();
        let decoder: BundleAssignmentReader<_> =
            reader.open_assignment_reader().unwrap();
        assert!(decoder.is_ben());
        assert!(!decoder.is_xben());
    }

    // -----------------------------------------------------------------------
    // Robustness tests
    // -----------------------------------------------------------------------

    #[test]
    fn fully_empty_bundle_finalizes_and_round_trips() {
        // No assets, no stream bytes, no stream phase at all.
        let writer = BendlWriter::new(make_buffer(), AssignmentFormat::Ben).unwrap();
        let buf = writer.finish().unwrap().into_inner();
        let reader = BendlReader::open(Cursor::new(buf)).unwrap();
        assert!(reader.is_complete());
        assert_eq!(reader.sample_count(), Some(0));
        assert_eq!(reader.header().stream_len, 0);
        assert_eq!(reader.assets().len(), 0);
        // Even with zero assets the directory is present and empty.
        assert_ne!(reader.header().directory_offset, 0);
        // directory_len should equal the 4-byte empty entry-count header.
        assert_eq!(reader.header().directory_len, 4);
    }

    #[test]
    fn begin_stream_twice_returns_wrong_state_error() {
        let mut writer = BendlWriter::new(make_buffer(), AssignmentFormat::Ben).unwrap();
        {
            let handle = match writer.begin_stream() {
                Ok(h) => h,
                Err(_) => panic!("first begin_stream must succeed"),
            };
            // Drop the handle without calling finish() — the writer is
            // now stuck in the Streaming state.
            drop(handle);
        }
        let err = writer.begin_stream().err().expect("second begin_stream must fail");
        assert!(matches!(err, BendlWriteError::WrongState { .. }));
    }

    #[test]
    fn finish_from_streaming_state_errors() {
        let mut writer = BendlWriter::new(make_buffer(), AssignmentFormat::Ben).unwrap();
        match writer.begin_stream() {
            Ok(handle) => drop(handle),
            Err(_) => panic!("begin_stream must succeed"),
        }
        // Intentionally leave the writer in the Streaming state.
        let err = writer.finish().unwrap_err();
        assert!(matches!(
            err,
            BendlWriteError::WrongState { found: "Streaming", .. }
        ));
    }

    #[test]
    fn stress_many_custom_assets_round_trip() {
        let mut writer = BendlWriter::new(make_buffer(), AssignmentFormat::Ben).unwrap();
        let count = 500usize;
        for i in 0..count {
            let name = format!("blob_{i:05}");
            let payload = vec![(i & 0xFF) as u8; (i % 17) + 1];
            writer
                .add_asset(ASSET_TYPE_CUSTOM, &name, &payload, AddAssetOptions::defaults())
                .unwrap();
        }
        writer
            .write_stream_bytes(b"STANDARD BEN FILE\x00fake", 1)
            .unwrap();
        let buf = writer.finish().unwrap().into_inner();

        let mut reader = BendlReader::open(Cursor::new(buf)).unwrap();
        assert_eq!(reader.assets().len(), count);
        // Spot-check a handful of entries by reading their payload bytes back.
        for i in [0usize, 1, 42, 199, 499] {
            let name = format!("blob_{i:05}");
            let entry = reader.find_asset_by_name(&name).cloned().unwrap();
            let got = reader.asset_bytes(&entry).unwrap();
            assert_eq!(got, vec![(i & 0xFF) as u8; (i % 17) + 1]);
        }
    }

    #[test]
    fn append_empty_commit_is_noop() {
        let (bundle, _) = build_base_bundle();
        let bundle_before = bundle.clone();
        let appender = BendlAppender::open(Cursor::new(bundle)).unwrap();
        // No add_asset calls. Commit should return the file unchanged.
        let buf = appender.commit().unwrap().into_inner();
        assert_eq!(buf, bundle_before);
    }

    #[test]
    fn append_then_reopen_and_append_again() {
        let (bundle, _) = build_base_bundle();

        // First commit: add a graph.
        let mut appender = BendlAppender::open(Cursor::new(bundle)).unwrap();
        appender
            .add_json_asset(ASSET_TYPE_GRAPH, "graph.json", b"{\"n\":[0,1,2]}")
            .unwrap();
        let buf = appender.commit().unwrap().into_inner();

        // Second commit: reopen the same bytes and add a custom blob.
        let mut appender = BendlAppender::open(Cursor::new(buf)).unwrap();
        appender
            .add_asset(
                ASSET_TYPE_CUSTOM,
                "extra.bin",
                b"later",
                AddAssetOptions::defaults(),
            )
            .unwrap();
        let buf = appender.commit().unwrap().into_inner();

        // Final read: all three assets should be present.
        let reader = BendlReader::open(Cursor::new(buf)).unwrap();
        let names: Vec<&str> = reader.assets().iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"metadata.json"));
        assert!(names.contains(&"graph.json"));
        assert!(names.contains(&"extra.bin"));
        // Sample count from the original stream is preserved across both
        // appends.
        assert_eq!(reader.sample_count(), Some(3));
    }

    #[test]
    fn append_does_not_disturb_front_loaded_asset_bytes() {
        // Base bundle has a graph.json asset with known bytes; after
        // append of a custom blob, reading graph.json must still return
        // exactly the same decoded bytes as before.
        let graph = br#"{"nodes":[0,1,2,3,4,5,6,7,8,9,10]}"#;
        let mut writer = BendlWriter::new(make_buffer(), AssignmentFormat::Ben).unwrap();
        writer
            .add_json_asset(ASSET_TYPE_GRAPH, "graph.json", graph)
            .unwrap();
        writer
            .write_stream_bytes(b"STANDARD BEN FILE\x00fake", 1)
            .unwrap();
        let bundle = writer.finish().unwrap().into_inner();

        let mut reader = BendlReader::open(Cursor::new(bundle.clone())).unwrap();
        let entry = reader
            .find_asset_by_type(ASSET_TYPE_GRAPH)
            .cloned()
            .unwrap();
        let graph_before = reader.asset_bytes(&entry).unwrap();
        drop(reader);

        let mut appender = BendlAppender::open(Cursor::new(bundle)).unwrap();
        appender
            .add_asset(
                ASSET_TYPE_CUSTOM,
                "extra.bin",
                b"0123456789",
                AddAssetOptions::defaults(),
            )
            .unwrap();
        let buf = appender.commit().unwrap().into_inner();

        let mut reader = BendlReader::open(Cursor::new(buf)).unwrap();
        let entry = reader
            .find_asset_by_type(ASSET_TYPE_GRAPH)
            .cloned()
            .unwrap();
        let graph_after = reader.asset_bytes(&entry).unwrap();
        assert_eq!(graph_before, graph_after);
    }

    #[test]
    fn writer_accepts_custom_asset_with_canonical_name_but_non_canonical_type() {
        // A custom asset named "graph.json" is not a singleton because the
        // singleton uniqueness check keys off asset_type, not name. Adding
        // a real GRAPH singleton after it must then fail on DuplicateName.
        let mut writer = BendlWriter::new(make_buffer(), AssignmentFormat::Ben).unwrap();
        writer
            .add_asset(
                ASSET_TYPE_CUSTOM,
                "graph.json",
                b"custom graph-ish bytes",
                AddAssetOptions::defaults(),
            )
            .unwrap();
        let err = writer
            .add_json_asset(ASSET_TYPE_GRAPH, "graph.json", b"{}")
            .unwrap_err();
        assert!(matches!(err, BendlWriteError::DuplicateName(ref n) if n == "graph.json"));
    }

    #[test]
    fn writer_asset_with_checksum_round_trips_through_reader() {
        let mut writer = BendlWriter::new(make_buffer(), AssignmentFormat::Ben).unwrap();
        let checksum = vec![0x01, 0x02, 0x03, 0x04];
        writer
            .add_asset(
                ASSET_TYPE_CUSTOM,
                "with_checksum",
                b"hello",
                AddAssetOptions {
                    checksum: Some(checksum.clone()),
                    ..AddAssetOptions::defaults()
                },
            )
            .unwrap();
        writer
            .write_stream_bytes(b"STANDARD BEN FILE\x00fake", 1)
            .unwrap();
        let buf = writer.finish().unwrap().into_inner();

        let reader = BendlReader::open(Cursor::new(buf)).unwrap();
        let entry = reader
            .find_asset_by_name("with_checksum")
            .cloned()
            .unwrap();
        assert_eq!(entry.checksum, Some(checksum));
        assert_ne!(entry.asset_flags & crate::io::bundle::format::ASSET_FLAG_CHECKSUM, 0);
    }

    #[test]
    fn finished_writer_rejects_further_operations() {
        let mut writer = BendlWriter::new(make_buffer(), AssignmentFormat::Ben).unwrap();
        writer.write_stream_bytes(b"STANDARD BEN FILE\x00fake", 1).unwrap();
        // Take a handle to the writer by going through begin_stream first.
        // Actually finish() consumes self, so instead assert the state
        // machine barfs when we manually poke it in the Finished state.
        //
        // We simulate by calling finish() and then checking there is no
        // way to call add_asset/begin_stream afterwards — `finish` consumes
        // `self`, which is itself the protection.
        let buf = writer.finish().unwrap().into_inner();
        // The resulting buffer is a valid finalized bundle.
        let reader = BendlReader::open(Cursor::new(buf)).unwrap();
        assert!(reader.is_complete());
    }

    #[test]
    fn appender_commit_after_abort_is_not_possible_but_abort_leaves_bytes_unchanged() {
        let (bundle, _) = build_base_bundle();
        let before = bundle.clone();
        let mut appender = BendlAppender::open(Cursor::new(bundle)).unwrap();
        appender
            .add_asset(
                ASSET_TYPE_CUSTOM,
                "wont_land",
                b"orphan",
                AddAssetOptions::defaults(),
            )
            .unwrap();
        let buf = appender.abort().into_inner();
        assert_eq!(buf, before, "abort must leave file bytes unchanged");
    }

    #[test]
    fn writer_rejects_add_json_asset_with_wrong_canonical_metadata_name() {
        let mut writer = BendlWriter::new(make_buffer(), AssignmentFormat::Ben).unwrap();
        let err = writer
            .add_json_asset(ASSET_TYPE_METADATA, "meta.json", b"{}")
            .unwrap_err();
        assert!(matches!(
            err,
            BendlWriteError::WrongCanonicalName { asset_type: ASSET_TYPE_METADATA, .. }
        ));
        // After a rejected add, no entries have been recorded — a
        // subsequent valid add proceeds normally.
        writer
            .add_json_asset(ASSET_TYPE_METADATA, "metadata.json", b"{}")
            .unwrap();
        writer.write_stream_bytes(b"STANDARD BEN FILE\x00fake", 1).unwrap();
        let buf = writer.finish().unwrap().into_inner();
        let reader = BendlReader::open(Cursor::new(buf)).unwrap();
        assert_eq!(reader.assets().len(), 1);
    }

    #[test]
    fn writer_rejected_add_leaves_singleton_slot_usable() {
        // A rejected singleton add must not consume the singleton slot —
        // otherwise a future valid add with the correct canonical name
        // would spuriously fail with DuplicateSingletonType.
        let mut writer = BendlWriter::new(make_buffer(), AssignmentFormat::Ben).unwrap();
        // First try with wrong canonical name — rejected.
        let _ = writer
            .add_json_asset(ASSET_TYPE_GRAPH, "not_graph.json", b"{}")
            .unwrap_err();
        // Now retry with correct name; should succeed.
        writer
            .add_json_asset(ASSET_TYPE_GRAPH, "graph.json", b"{}")
            .unwrap();
    }

    #[test]
    fn append_rejects_duplicate_name_across_existing_and_pending() {
        let (bundle, _) = build_base_bundle();
        let mut appender = BendlAppender::open(Cursor::new(bundle)).unwrap();
        // First pending add: "blob".
        appender
            .add_asset(
                ASSET_TYPE_CUSTOM,
                "blob",
                b"1",
                AddAssetOptions::defaults(),
            )
            .unwrap();
        // Second pending add with same name must be rejected.
        let err = appender
            .add_asset(
                ASSET_TYPE_CUSTOM,
                "blob",
                b"2",
                AddAssetOptions::defaults(),
            )
            .unwrap_err();
        assert!(matches!(err, BendlWriteError::DuplicateName(_)));
        // Committing the still-valid first pending add should still work.
        let buf = appender.commit().unwrap().into_inner();
        let reader = BendlReader::open(Cursor::new(buf)).unwrap();
        assert!(reader.find_asset_by_name("blob").is_some());
    }

    #[test]
    fn write_ben_stream_closure_error_short_circuits_finalize() {
        use crate::BenVariant;

        let mut writer = BendlWriter::new(make_buffer(), AssignmentFormat::Ben).unwrap();
        let err = writer
            .write_ben_stream(BenVariant::Standard, |_ctx| {
                Err(io::Error::new(io::ErrorKind::Other, "boom"))
            })
            .unwrap_err();
        match err {
            BendlWriteError::Io(e) => assert_eq!(e.kind(), io::ErrorKind::Other),
            other => panic!("expected Io(Other), got {other:?}"),
        }
    }

    // -----------------------------------------------------------------------
    // Randomized / stress tests
    // -----------------------------------------------------------------------

    /// Build a bundle from a random set of custom assets (plus an optional
    /// metadata asset) and fully round-trip it through the reader. Repeated
    /// with a seeded ChaCha PRNG so the sequence is deterministic but
    /// covers a wide surface.
    #[test]
    fn randomized_round_trip_many_custom_assets() {
        use rand::{Rng, SeedableRng};
        use rand_chacha::ChaCha8Rng;

        for seed in 0u64..12 {
            let mut rng = ChaCha8Rng::seed_from_u64(seed ^ 0xA110_CADE_F00D);
            let n_assets: usize = rng.random_range(0..=25);
            let include_metadata = rng.random_bool(0.5);

            let mut writer = BendlWriter::new(make_buffer(), AssignmentFormat::Ben).unwrap();

            let mut expected: Vec<(String, Vec<u8>, bool)> = Vec::new();
            if include_metadata {
                let payload = format!(r#"{{"seed":{seed}}}"#).into_bytes();
                writer
                    .add_json_asset(ASSET_TYPE_METADATA, "metadata.json", &payload)
                    .unwrap();
                expected.push(("metadata.json".to_string(), payload, false));
            }

            for i in 0..n_assets {
                let size: usize = rng.random_range(0..=512);
                let payload: Vec<u8> = (0..size).map(|_| rng.random::<u8>()).collect();
                let compress = rng.random_bool(0.4);
                let is_json = rng.random_bool(0.15) && size > 0;
                let payload = if is_json {
                    // Override with a synthetic JSON blob so the json flag
                    // actually matches the content.
                    format!(r#"{{"i":{i},"seed":{seed}}}"#).into_bytes()
                } else {
                    payload
                };

                let mut opts = AddAssetOptions::defaults();
                if compress {
                    opts = opts.compress();
                } else {
                    opts = opts.raw();
                }
                if is_json {
                    opts = opts.json();
                }
                let name = format!("seed{seed}-asset{i}.bin");
                writer
                    .add_asset(ASSET_TYPE_CUSTOM, &name, &payload, opts)
                    .unwrap();
                expected.push((name, payload, is_json));
            }

            // Write a small deterministic stream so the bundle is
            // assignment-complete.
            let sample_count: i64 = rng.random_range(0..=20);
            let fake_stream = b"STANDARD BEN FILE\x00\x01\x02payload".to_vec();
            writer
                .write_stream_bytes(&fake_stream, sample_count)
                .unwrap();
            let buf = writer.finish().unwrap().into_inner();

            let mut reader = BendlReader::open(Cursor::new(buf)).unwrap();
            assert!(reader.is_complete(), "seed {seed}: not finalized");
            assert_eq!(reader.sample_count(), Some(sample_count));
            reader
                .validate_directory()
                .unwrap_or_else(|e| panic!("seed {seed}: validation failed: {e:?}"));
            assert_eq!(reader.assets().len(), expected.len(), "seed {seed}");

            for (name, want, _is_json) in &expected {
                let entry = reader
                    .find_asset_by_name(name)
                    .cloned()
                    .unwrap_or_else(|| panic!("seed {seed}: asset {name:?} missing"));
                let got = reader.asset_bytes(&entry).unwrap();
                assert_eq!(&got, want, "seed {seed}: payload mismatch for {name}");
            }

            // Stream must also read back exactly.
            let mut stream_buf = Vec::new();
            reader
                .assignment_stream_reader()
                .unwrap()
                .read_to_end(&mut stream_buf)
                .unwrap();
            assert_eq!(stream_buf, fake_stream, "seed {seed}");
        }
    }

    #[test]
    fn five_successive_appends_preserve_everything() {
        // Start from a finalized bundle with only a metadata asset and a
        // short stream. Then open it five times via BendlAppender and add
        // one asset per round. After every round, the previous assets must
        // still be readable and sample_count must remain authoritative.
        let (mut buf, _) = build_base_bundle();

        // Sanity-check the baseline.
        let baseline_reader = BendlReader::open(Cursor::new(buf.clone())).unwrap();
        let baseline_samples = baseline_reader.sample_count();
        assert!(baseline_samples.is_some());
        drop(baseline_reader);

        let mut accumulated: Vec<(String, Vec<u8>)> = vec![(
            "metadata.json".to_string(),
            br#"{"version":1}"#.to_vec(),
        )];

        for round in 0..5 {
            let cursor = Cursor::new(buf);
            let mut appender = BendlAppender::open(cursor).unwrap();
            let name = format!("round-{round}.bin");
            let payload: Vec<u8> = (0u8..=(round as u8 * 7 + 3)).collect();
            appender
                .add_asset(
                    ASSET_TYPE_CUSTOM,
                    &name,
                    &payload,
                    AddAssetOptions::defaults(),
                )
                .unwrap();
            let commit = appender.commit().unwrap();
            buf = commit.into_inner();
            accumulated.push((name, payload));

            // Re-open and verify the full set is intact and sample_count
            // still matches the baseline (append must not touch it).
            let mut reader = BendlReader::open(Cursor::new(buf.clone())).unwrap();
            assert!(reader.is_complete(), "round {round}");
            assert_eq!(
                reader.sample_count(),
                baseline_samples,
                "sample count drifted at round {round}"
            );
            assert_eq!(
                reader.assets().len(),
                accumulated.len(),
                "asset count wrong at round {round}"
            );
            reader.validate_directory().unwrap();

            for (n, want) in &accumulated {
                let entry = reader
                    .find_asset_by_name(n)
                    .cloned()
                    .unwrap_or_else(|| panic!("round {round}: {n:?} missing"));
                let got = reader.asset_bytes(&entry).unwrap();
                assert_eq!(&got, want, "round {round}: payload mismatch for {n}");
            }
        }
    }

    #[test]
    fn randomized_append_sequence_preserves_all_prior_entries() {
        // Independent coverage for append: random number of rounds, random
        // payload sizes. Catches any bookkeeping drift in the appender's
        // directory-rewrite path.
        use rand::{Rng, SeedableRng};
        use rand_chacha::ChaCha8Rng;

        let (mut buf, _) = build_base_bundle();
        let mut accumulated: Vec<(String, Vec<u8>)> = vec![(
            "metadata.json".to_string(),
            br#"{"version":1}"#.to_vec(),
        )];

        let mut rng = ChaCha8Rng::seed_from_u64(0xDEAD_BEEF_CAFE_F00D);
        let rounds: usize = rng.random_range(3..=8);
        for round in 0..rounds {
            let adds: usize = rng.random_range(1..=4);
            let cursor = Cursor::new(buf);
            let mut appender = BendlAppender::open(cursor).unwrap();
            for k in 0..adds {
                let size: usize = rng.random_range(0..=256);
                let payload: Vec<u8> =
                    (0..size).map(|_| rng.random::<u8>()).collect();
                let name = format!("r{round}-a{k}.bin");
                appender
                    .add_asset(
                        ASSET_TYPE_CUSTOM,
                        &name,
                        &payload,
                        AddAssetOptions::defaults(),
                    )
                    .unwrap();
                accumulated.push((name, payload));
            }
            let commit = appender.commit().unwrap();
            buf = commit.into_inner();

            let mut reader = BendlReader::open(Cursor::new(buf.clone())).unwrap();
            reader.validate_directory().unwrap();
            assert_eq!(reader.assets().len(), accumulated.len());
            for (n, want) in &accumulated {
                let entry = reader.find_asset_by_name(n).cloned().unwrap();
                let got = reader.asset_bytes(&entry).unwrap();
                assert_eq!(&got, want, "append round {round}: {n}");
            }
        }
    }
}
