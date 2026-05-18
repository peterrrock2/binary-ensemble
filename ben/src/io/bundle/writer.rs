//! Write-side API for `.bendl` files.
//!
//! [`BendlWriter`] produces finalized bundles with the on-disk layout
//!
//! ```text
//! [header] [asset payloads] [assignment stream] [directory]
//! ```
//!
//! The writer operates in three logical phases, expressed via owned typestate transitions:
//!
//! 1. **asset phase** — the caller invokes [`BendlWriter::add_asset`] zero or more times. Each call
//!    writes the (optionally xz-compressed) payload to the file and records its absolute offset and
//!    length in an in-memory entry list.
//! 2. **stream phase** — the caller invokes [`BendlWriter::into_stream_session`] to consume the
//!    writer and obtain a [`BendlStreamSession`] that owns the underlying writer and implements
//!    `Write`. When the stream is complete the caller calls
//!    [`BendlStreamSession::finish_into_writer`] to recover the [`BendlWriter`] in the
//!    `StreamWritten` state.
//! 3. **finalize phase** — [`BendlWriter::finish`] writes the trailing directory and patches the
//!    header.
//!
//! The writer requires `Write + Seek` because the header is patched twice: once with the stream
//! offset (implicitly, by having reserved its slot at construction) and once with the finalized
//! stream length, sample count, directory offset, directory length, and `complete` flag.

use std::collections::HashSet;
use std::io::{self, Read, Seek, SeekFrom, Write};

use thiserror::Error;
use xz2::write::XzEncoder;

use super::format::{
    default_compresses_by_type, encode_directory, read_directory, standardized_name_for,
    AssignmentFormat, BendlDirectoryEntry, BendlFormatError, BendlHeader, KnownAssetKind,
    ASSET_FLAG_CHECKSUM, ASSET_FLAG_JSON, ASSET_FLAG_XZ, ASSET_TYPE_CUSTOM, DEFAULT_XZ_PRESET,
    FINALIZED_YES, HEADER_SIZE,
};

/// Ability to truncate an underlying seekable target to a given length.
///
/// This is not part of `std::io`, so `BendlAppender` takes a trait bound that abstracts it and is
/// implemented below for `std::fs::File` and `std::io::Cursor<Vec<u8>>`.
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
///
/// There is no "checksum opt-in/opt-out" knob: every asset written through the library carries a
/// CRC32C of its on-disk payload bytes, computed automatically by the writer. A future
/// recovery/debug writer that needs to emit unchecked assets must be an explicitly named
/// `*_unverified` API and excluded from normal write paths.
#[derive(Debug, Clone, Default)]
pub struct AddAssetOptions {
    /// Compression override. `None` means "follow the default policy for this asset type";
    /// `Some(true)` forces xz compression; `Some(false)` forces a raw payload.
    pub compress: Option<bool>,
    /// Whether the decoded payload is UTF-8 JSON. Adds the [`ASSET_FLAG_JSON`] bit to the entry's
    /// flags.
    pub is_json: bool,
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

    /// Force the writer to store the payload raw even if the default policy would compress it.
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
    /// No assets have been written yet, but the provisional header is already in place and the
    /// writer is positioned just after it.
    Assets,
    /// A stream session has been finished and the writer is ready for [`BendlWriter::finish`]. The
    /// streaming phase itself is expressed in the type system via [`BendlStreamSession`] and is
    /// therefore not observable in this enum.
    StreamWritten { stream_len: u64, sample_count: i64 },
}

impl<W: Write + Seek> BendlWriter<W> {
    /// Create a new writer by writing a provisional header at offset 0.
    ///
    /// The assignment stream will begin immediately after the asset payload region —
    /// [`BendlWriter::into_stream_session`] computes the exact offset at the moment it is called,
    /// so asset writes that happen between `new` and `into_stream_session` push the stream out as
    /// expected.
    pub fn new(mut inner: W, assignment_format: AssignmentFormat) -> io::Result<Self> {
        inner.seek(SeekFrom::Start(0))?;
        // stream_offset in the provisional header is patched at into_stream_session time; start it
        // just after the header.
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
    /// The payload is written to the file immediately at the current position (right after the
    /// previous asset, or right after the header if this is the first asset). Its absolute offset
    /// and length are recorded in the in-memory directory entry list.
    ///
    /// This method enforces the canonical-name and uniqueness rules **before** writing any bytes,
    /// so a rejected asset leaves the file untouched.
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
            // Roll back the singleton insertion before returning, so the writer remains in a
            // consistent state. (Only known singleton types would have been inserted above.)
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

        // CRC32C over the on-disk payload bytes. For compressed assets this is the compressed bytes
        // (verification happens before decompression). See ASSET_FLAG_CHECKSUM for the wire-format
        // pin.
        let crc = crc32c::crc32c(&payload_bytes);
        let checksum_bytes = crc.to_le_bytes().to_vec();

        let mut asset_flags: u16 = ASSET_FLAG_CHECKSUM;
        if options.is_json {
            asset_flags |= ASSET_FLAG_JSON;
        }
        if compress {
            asset_flags |= ASSET_FLAG_XZ;
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
            checksum: Some(checksum_bytes),
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

    /// Add one of the known singleton assets, using its reserved asset-type integer and
    /// standardized name automatically.
    pub fn add_known_asset(
        &mut self,
        kind: KnownAssetKind,
        payload: &[u8],
        options: AddAssetOptions,
    ) -> Result<(), BendlWriteError> {
        self.add_asset(
            kind.asset_type(),
            kind.standardized_name(),
            payload,
            options,
        )
    }

    /// Add a custom (writer-named) asset. The asset-type is set to [`ASSET_TYPE_CUSTOM`]
    /// automatically.
    pub fn add_custom_asset(
        &mut self,
        name: &str,
        payload: &[u8],
        options: AddAssetOptions,
    ) -> Result<(), BendlWriteError> {
        self.add_asset(ASSET_TYPE_CUSTOM, name, payload, options)
    }

    /// Consume the writer and transition into the stream phase.
    ///
    /// The returned [`BendlStreamSession`] owns the underlying writer and implements `Write`, so it
    /// can be plumbed into a [`crate::io::writer::BenStreamWriter`] (or written to directly). When
    /// the stream is complete the caller calls [`BendlStreamSession::finish_into_writer`] to
    /// recover ownership of a [`BendlWriter`] in the `StreamWritten` state, ready for
    /// [`BendlWriter::finish`].
    ///
    /// Returns [`BendlWriteError::WrongState`] when called on a writer that has already produced a
    /// stream (e.g. via a prior `finish_into_writer`); this guard prevents a second
    /// `into_stream_session` from silently overwriting `header.stream_offset` and corrupting the
    /// bundle.
    pub fn into_stream_session(mut self) -> Result<BendlStreamSession<W>, BendlWriteError> {
        match self.state {
            WriterState::Assets => {}
            WriterState::StreamWritten { .. } => {
                return Err(BendlWriteError::WrongState {
                    expected: "Assets",
                    found: "StreamWritten",
                });
            }
        }

        let stream_offset = self.inner.seek(SeekFrom::Current(0))?;
        self.header.stream_offset = stream_offset;

        Ok(BendlStreamSession {
            inner: Some(self.inner),
            parent: Some(ParentState {
                header: self.header,
                entries: self.entries,
                names: self.names,
                singleton_types: self.singleton_types,
            }),
            start_offset: stream_offset,
            bytes_written: 0,
        })
    }

    /// Write the trailing directory, patch the header, and return the underlying writer.
    pub fn finish(mut self) -> Result<W, BendlWriteError> {
        let (stream_len, sample_count) = match self.state {
            WriterState::StreamWritten {
                stream_len,
                sample_count,
            } => (stream_len, sample_count),
            WriterState::Assets => {
                // No stream written; treat as empty stream located just after the asset region.
                let stream_offset = self.inner.seek(SeekFrom::Current(0))?;
                self.header.stream_offset = stream_offset;
                (0, 0)
            }
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

        Ok(self.inner)
    }
}

/// Internal state of a [`BendlWriter`] that has been temporarily moved into a
/// [`BendlStreamSession`]. Stored as a single struct so `finish_into_writer` can rebuild the writer
/// with one move.
struct ParentState {
    header: BendlHeader,
    entries: Vec<BendlDirectoryEntry>,
    names: HashSet<String>,
    singleton_types: HashSet<u16>,
}

/// Owned stream-phase session. Holds the underlying writer and the parent [`BendlWriter`]'s
/// in-memory state across the streaming phase, implements `Write` so it can be plumbed into a
/// [`crate::io::writer::BenStreamWriter`], and exposes [`Self::finish_into_writer`] to hand
/// ownership back as a [`BendlWriter`] in the `StreamWritten` state.
///
/// `inner` and `parent` are wrapped in `Option` so `finish_into_writer` can `take()` them without
/// partial-moving out of a `Drop` type. The [`Drop`] impl emits a `tracing::warn!` if the session
/// is dropped without `finish_into_writer`, since that leaves the bundle on disk unfinalized.
pub struct BendlStreamSession<W: Write + Seek> {
    inner: Option<W>,
    parent: Option<ParentState>,
    start_offset: u64,
    bytes_written: u64,
}

impl<W: Write + Seek> BendlStreamSession<W> {
    /// Number of bytes written into the stream region so far. Pure counter — no I/O, no `&mut`
    /// required.
    pub fn bytes_written(&self) -> u64 {
        self.bytes_written
    }

    /// Offset (in the underlying writer) at which the stream region began, recorded at
    /// session-construction time.
    pub fn start_offset(&self) -> u64 {
        self.start_offset
    }

    /// End the stream phase and return ownership of a [`BendlWriter`] in the `StreamWritten` state,
    /// ready for [`BendlWriter::finish`].
    ///
    /// Infallible: the body is `take()` + arithmetic + struct construction with no I/O. Once this
    /// method returns, the session's [`Drop`] impl observes `inner.is_none()` and skips the warn.
    pub fn finish_into_writer(mut self, sample_count: i64) -> BendlWriter<W> {
        let inner = self.inner.take().expect("session has not been finished");
        let parent = self.parent.take().expect("session has not been finished");
        BendlWriter {
            inner,
            header: parent.header,
            entries: parent.entries,
            names: parent.names,
            singleton_types: parent.singleton_types,
            state: WriterState::StreamWritten {
                stream_len: self.bytes_written,
                sample_count,
            },
        }
    }
}

impl<W: Write + Seek> Write for BendlStreamSession<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let inner = self.inner.as_mut().expect("session has not been finished");
        let n = inner.write(buf)?;
        self.bytes_written += n as u64;
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner
            .as_mut()
            .expect("session has not been finished")
            .flush()
    }
}

impl<W: Write + Seek> Drop for BendlStreamSession<W> {
    fn drop(&mut self) {
        if self.inner.is_some() {
            tracing::warn!(
                "BendlStreamSession dropped without finish_into_writer; \
                 bundle on disk is unfinalized"
            );
        }
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

/// Post-finalize appender that grows an existing `.bendl` file with new assets without rewriting
/// the assignment stream.
///
/// The workflow is:
///
/// 1. [`BendlAppender::open`] opens a finalized bundle and loads its directory into memory.
/// 2. [`BendlAppender::add_asset`] (or [`BendlAppender::add_json_asset`]) validates and buffers
///    each new asset. Validation happens up front, so duplicate singletons or names are rejected
///    **before** any file mutation, and a rejected add_asset leaves the file unchanged.
/// 3. [`BendlAppender::commit`] compresses the buffered assets (if any), truncates the file at the
///    old directory offset, writes the new asset payloads, writes a new directory at the new EOF,
///    and patches the header.
///
/// A [`BendlAppender`] that is dropped without calling `commit` leaves the underlying file
/// unchanged.
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
}

impl<W: Read + Write + Seek + BendlTruncate> BendlAppender<W> {
    /// Open a finalized bundle for append.
    ///
    /// Returns [`BendlWriteError::BundleIncomplete`] if the header's `complete` flag is not set —
    /// append is unsafe on unfinalized bundles because the stream region has no authoritative end.
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
    /// This validates the new asset against both the loaded directory and any previously-enqueued
    /// pending assets. If validation fails, the pending list is unchanged and no bytes have been
    /// written to the file.
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

    /// Append one of the known singleton assets, using its reserved asset-type integer and
    /// standardized name automatically.
    pub fn add_known_asset(
        &mut self,
        kind: KnownAssetKind,
        payload: &[u8],
        options: AddAssetOptions,
    ) -> Result<(), BendlWriteError> {
        self.add_asset(
            kind.asset_type(),
            kind.standardized_name(),
            payload,
            options,
        )
    }

    /// Append a custom (writer-named) asset. The asset-type is set to [`ASSET_TYPE_CUSTOM`]
    /// automatically.
    pub fn add_custom_asset(
        &mut self,
        name: &str,
        payload: &[u8],
        options: AddAssetOptions,
    ) -> Result<(), BendlWriteError> {
        self.add_asset(ASSET_TYPE_CUSTOM, name, payload, options)
    }

    /// Commit all pending appends.
    ///
    /// This compresses any buffered payloads that need it (entirely in memory), then performs the
    /// file mutation in a single burst: truncate at the old directory offset, write new payloads,
    /// write a new directory, and patch the header.
    ///
    /// If compression fails, the file is left unchanged.
    pub fn commit(mut self) -> Result<W, BendlWriteError> {
        // If nothing was enqueued, commit is a no-op — return the file untouched.
        if self.pending.is_empty() {
            return Ok(self.inner);
        }

        // Phase 1: compress any pending payloads and build new entries with placeholder offsets. Do
        // this entirely in memory so failures here leave the file untouched.
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

            // CRC32C over on-disk payload bytes (compressed if XZ).
            let crc = crc32c::crc32c(&bytes);
            let checksum_bytes = crc.to_le_bytes().to_vec();

            let mut asset_flags: u16 = ASSET_FLAG_CHECKSUM;
            if asset.is_json {
                asset_flags |= ASSET_FLAG_JSON;
            }
            if asset.compress {
                asset_flags |= ASSET_FLAG_XZ;
            }

            encoded.push(EncodedPending {
                asset_type: asset.asset_type,
                name: asset.name,
                bytes,
                asset_flags,
                checksum: Some(checksum_bytes),
            });
        }

        // Phase 2: file mutation. From this point forward, a failure leaves the bundle in a damaged
        // state. We do everything in the order (truncate, write payloads, write directory, patch
        // header) so that even if we crash mid-way, the header still points at the old directory
        // until the very last write.
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

    /// Release the underlying reader without committing any pending appends. The file is unchanged.
    pub fn abort(self) -> W {
        self.inner
    }
}
