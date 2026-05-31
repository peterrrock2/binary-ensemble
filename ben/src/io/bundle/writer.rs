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
//! The writer requires `Write + Seek` because the header is written provisionally at construction
//! and patched on finalization with the stream checksum, stream length, sample count, directory
//! offset, directory length, and `finalized` flag.

use std::collections::HashSet;
use std::io::{self, Read, Seek, SeekFrom, Write};

use thiserror::Error;
use xz2::write::XzEncoder;

use super::format::{
    default_compresses_by_type, encode_directory, read_directory, standardized_name_for,
    AssignmentFormat, BendlDirectoryEntry, BendlFormatError, BendlHeader, KnownAssetKind,
    ASSET_FLAG_CHECKSUM, ASSET_FLAG_JSON, ASSET_FLAG_XZ, ASSET_TYPE_CUSTOM, DEFAULT_XZ_PRESET,
    FINALIZED_YES, HEADER_FLAG_STREAM_CHECKSUM, HEADER_SIZE,
};

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

/// An asset payload prepared for on-disk storage: the (optionally xz-compressed) bytes, the
/// directory-entry flags describing them, and the CRC32C over those exact bytes.
struct EncodedAsset {
    bytes: Vec<u8>,
    asset_flags: u16,
    checksum: Vec<u8>,
}

/// Compress (if requested), checksum, and assemble the directory-entry flags for one asset payload.
///
/// This is the single encode path shared by [`BendlWriter::add_asset`] and
/// [`BendlAppender::commit`], so the create and append routes can never drift on compression, flag
/// assembly, or CRC coverage. It is pure (in-memory), so a failure leaves any backing file
/// untouched. The CRC32C is over the **on-disk** bytes — the compressed bytes when xz is applied,
/// so verification happens before decompression (see [`ASSET_FLAG_CHECKSUM`]).
fn encode_asset_payload(
    payload: Vec<u8>,
    compress: bool,
    is_json: bool,
) -> io::Result<EncodedAsset> {
    let bytes = if compress {
        let mut encoder = XzEncoder::new(Vec::new(), DEFAULT_XZ_PRESET);
        encoder.write_all(&payload)?;
        encoder.finish()?
    } else {
        payload
    };

    let mut asset_flags: u16 = ASSET_FLAG_CHECKSUM;
    if is_json {
        asset_flags |= ASSET_FLAG_JSON;
    }
    if compress {
        asset_flags |= ASSET_FLAG_XZ;
    }

    let checksum = crc32c::crc32c(&bytes).to_le_bytes().to_vec();
    Ok(EncodedAsset {
        bytes,
        asset_flags,
        checksum,
    })
}

/// Tracks the asset names and singleton asset-types already claimed in a bundle, and enforces the
/// canonical-name + uniqueness rules shared by the create and append paths.
///
/// [`Self::claim`] validates fully before mutating, so a rejected asset never leaves the registry
/// in a half-updated state — there is nothing to roll back.
#[derive(Default)]
struct AssetNameRegistry {
    names: HashSet<String>,
    singleton_types: HashSet<u16>,
}

impl AssetNameRegistry {
    /// An empty registry, for a fresh bundle.
    fn new() -> Self {
        Self::default()
    }

    /// Seed a registry from the directory entries of an existing finalized bundle (append path).
    fn from_entries(entries: &[BendlDirectoryEntry]) -> Self {
        let mut registry = Self::new();
        for entry in entries {
            registry.names.insert(entry.name.clone());
            if standardized_name_for(entry.asset_type).is_some() {
                registry.singleton_types.insert(entry.asset_type);
            }
        }
        registry
    }

    /// Validate the canonical-name and uniqueness rules for a candidate asset **without** mutating
    /// state. A known singleton type must use its standardized name and may appear only once; every
    /// asset name must be unique.
    fn check(&self, asset_type: u16, name: &str) -> Result<(), BendlWriteError> {
        if let Some(canonical) = standardized_name_for(asset_type) {
            if name != canonical {
                return Err(BendlWriteError::WrongCanonicalName {
                    asset_type,
                    expected: canonical.to_string(),
                    found: name.to_string(),
                });
            }
            if self.singleton_types.contains(&asset_type) {
                return Err(BendlWriteError::DuplicateSingletonType(asset_type));
            }
        }
        if self.names.contains(name) {
            return Err(BendlWriteError::DuplicateName(name.to_string()));
        }
        Ok(())
    }

    /// Validate via [`Self::check`] and, on success, reserve the name and (for singleton types) the
    /// asset-type so subsequent claims see it as taken.
    fn claim(&mut self, asset_type: u16, name: &str) -> Result<(), BendlWriteError> {
        self.check(asset_type, name)?;
        self.names.insert(name.to_string());
        if standardized_name_for(asset_type).is_some() {
            self.singleton_types.insert(asset_type);
        }
        Ok(())
    }
}

/// Writer for a single `.bendl` file.
pub struct BendlWriter<W: Write + Seek> {
    inner: W,
    header: BendlHeader,
    entries: Vec<BendlDirectoryEntry>,
    registry: AssetNameRegistry,
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
            registry: AssetNameRegistry::new(),
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

        // Validate before any expensive work, but do not reserve the name/type until the fallible
        // encoding and write have both succeeded. A failed compression or write should not poison
        // the in-memory registry and make a retry look like a duplicate.
        self.registry.check(asset_type, name)?;

        let compress = options
            .compress
            .unwrap_or_else(|| default_compresses_by_type(asset_type));
        let encoded = encode_asset_payload(payload.to_vec(), compress, options.is_json)?;

        // Write at current file position.
        let payload_offset = self.inner.stream_position()?;
        self.inner.write_all(&encoded.bytes)?;

        self.registry.claim(asset_type, name)?;
        self.entries.push(BendlDirectoryEntry {
            asset_type,
            asset_flags: encoded.asset_flags,
            name: name.to_string(),
            payload_offset,
            payload_len: encoded.bytes.len() as u64,
            checksum: Some(encoded.checksum),
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

        let stream_offset = self.inner.stream_position()?;
        self.header.stream_offset = stream_offset;

        Ok(BendlStreamSession {
            inner: Some(self.inner),
            parent: Some(ParentState {
                header: self.header,
                entries: self.entries,
                registry: self.registry,
            }),
            start_offset: stream_offset,
            bytes_written: 0,
            hasher: 0,
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
                let stream_offset = self.inner.stream_position()?;
                self.header.stream_offset = stream_offset;
                // CRC32C of an empty byte sequence is 0x00000000.
                self.header.stream_checksum = 0;
                self.header.flags |= HEADER_FLAG_STREAM_CHECKSUM;
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
    registry: AssetNameRegistry,
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
    hasher: u32,
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
        let mut parent = self.parent.take().expect("session has not been finished");

        // Patch the stream checksum into the in-memory header so BendlWriter::finish can write it
        // to disk in a single header patch pass.
        parent.header.stream_checksum = self.hasher;
        parent.header.flags |= HEADER_FLAG_STREAM_CHECKSUM;

        BendlWriter {
            inner,
            header: parent.header,
            entries: parent.entries,
            registry: parent.registry,
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
        if n > 0 {
            self.bytes_written += n as u64;
            self.hasher = crc32c::crc32c_append(self.hasher, &buf[..n]);
        }
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
    #[error("cannot append to a bundle whose header does not have finalized == 1")]
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

// =====================================================================
// Append path
// =====================================================================

/// Post-finalize appender that grows an existing `.bendl` file with new assets without rewriting
/// the assignment stream.
///
/// The workflow is:
///
/// 1. [`BendlAppender::open`] opens a finalized bundle and loads its directory into memory.
/// 2. [`BendlAppender::add_asset`] (or [`BendlAppender::add_json_asset`]) validates and buffers
///    each new asset. Validation happens up front, so duplicate singletons or names are rejected
///    **before** any file mutation, and a rejected add_asset leaves the file unchanged.
/// 3. [`BendlAppender::commit`] compresses the buffered assets (if any), appends the new asset
///    payloads after the old EOF, writes a new directory, and patches the header. The old directory
///    is left in place as orphaned bytes until a future compact/rewrite operation; this keeps the
///    old header valid until the final header patch.
///
/// A [`BendlAppender`] that is dropped without calling `commit` leaves the underlying file
/// unchanged.
pub struct BendlAppender<W: Read + Write + Seek> {
    inner: W,
    header: BendlHeader,
    existing_entries: Vec<BendlDirectoryEntry>,
    pending: Vec<PendingAsset>,
    /// Names and singleton types claimed by the existing directory plus any pending adds. Seeded
    /// from the existing entries at open time, then extended as each pending asset is enqueued.
    registry: AssetNameRegistry,
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

/// A pending asset whose payload has been encoded in memory and is ready to be written to disk.
///
/// One element per prepared asset — this is the output of the pure, in-memory compression phase of
/// [`BendlAppender::commit`], carrying everything the subsequent file-mutation phase needs to write
/// the payload and its directory entry.
struct PreparedAppendAsset {
    asset_type: u16,
    asset_name: String,
    encoded_asset: EncodedAsset,
}

impl<W: Read + Write + Seek> BendlAppender<W> {
    /// Open a finalized bundle for append.
    ///
    /// Returns [`BendlWriteError::BundleIncomplete`] if the header's `finalized` flag is not set —
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

        let registry = AssetNameRegistry::from_entries(&existing_entries);

        Ok(BendlAppender {
            inner,
            header,
            existing_entries,
            pending: Vec::new(),
            registry,
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
        // Validate against both the loaded directory and previously-enqueued pending assets, and
        // reserve the name/type on success. Nothing is buffered if validation fails.
        self.registry.claim(asset_type, name)?;

        let compress = options
            .compress
            .unwrap_or_else(|| default_compresses_by_type(asset_type));

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

    /// Phase 1 of [`Self::commit`]: drain the pending queue and encode each payload through the
    /// shared encode path, entirely in memory.
    ///
    /// This is pure with respect to the file — it has no ordering constraint against the
    /// append-only mutation in `commit`, so a failure here returns before any byte is written and
    /// leaves the bundle untouched.
    fn prepare_pending_assets(&mut self) -> Result<Vec<PreparedAppendAsset>, BendlWriteError> {
        let mut prepared = Vec::with_capacity(self.pending.len());
        for asset in self.pending.drain(..) {
            let encoded_asset =
                encode_asset_payload(asset.raw_payload, asset.compress, asset.is_json)?;
            prepared.push(PreparedAppendAsset {
                asset_type: asset.asset_type,
                asset_name: asset.name,
                encoded_asset,
            });
        }
        Ok(prepared)
    }

    /// Commit all pending appends.
    ///
    /// This compresses any buffered payloads that need it (entirely in memory), then performs the
    /// file mutation in one append-only burst: seek to old EOF, write new payloads, write a new
    /// directory, and patch the header.
    ///
    /// If compression fails, the file is left unchanged.
    pub fn commit(mut self) -> Result<W, BendlWriteError> {
        // If nothing was enqueued, commit is a no-op — return the file untouched.
        if self.pending.is_empty() {
            return Ok(self.inner);
        }

        // Phase 1: compress any pending payloads in memory. This has no ordering constraint against
        // the file mutation below — a failure here leaves the file untouched.
        let encoded = self.prepare_pending_assets()?;

        // Phase 2: append-only file mutation. Until the final header patch, the old header still
        // points at the old directory, which remains intact. A crash before the patch leaves the
        // previous bundle readable with trailing orphaned bytes.
        let old_directory_end = self
            .header
            .directory_offset
            .checked_add(self.header.directory_len)
            .ok_or_else(|| {
                BendlWriteError::Io(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "directory_offset + directory_len overflowed while appending",
                ))
            })?;

        self.inner.seek(SeekFrom::Start(old_directory_end))?;

        // Compute new entries with real offsets as we write.
        let mut new_entries: Vec<BendlDirectoryEntry> =
            Vec::with_capacity(self.existing_entries.len() + encoded.len());
        new_entries.extend(self.existing_entries.iter().cloned());

        for prepared in encoded {
            let enc = prepared.encoded_asset;
            let payload_offset = self.inner.stream_position()?;
            self.inner.write_all(&enc.bytes)?;
            new_entries.push(BendlDirectoryEntry {
                asset_type: prepared.asset_type,
                asset_flags: enc.asset_flags,
                name: prepared.asset_name,
                payload_offset,
                payload_len: enc.bytes.len() as u64,
                checksum: Some(enc.checksum),
            });
        }

        // Write the new directory at the new EOF.
        let new_directory_offset = self.inner.stream_position()?;
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
