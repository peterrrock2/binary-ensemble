//! Read-only inspection of `.bendl` files.
//!
//! A [`BendlReader`] parses a bundle's fixed header and (if present) its trailing directory table.
//! It does not read any asset payload bytes until the caller explicitly requests them via
//! [`BendlReader::asset_bytes`] or [`BendlReader::asset_reader`]. The assignment stream region is
//! likewise exposed as a byte range the caller can plumb into a [`BenStreamReader`] without this
//! module reinterpreting any BEN/XBEN internals.
//!
//! The byte-level read adapters (bounded ranges, CRC tees, verifying wrappers) live in
//! [`super::verify`]; this module composes them behind the public API.
//!
//! ## Verification surface
//!
//! - [`BendlReader::asset_bytes`] and [`BendlReader::asset_reader`] are **verify-on-touch**: the
//!   CRC32C of the on-disk payload bytes is computed as data flows through, and a mismatch is
//!   reported at EOF.
//! - [`BendlReader::asset_bytes_unverified`], [`BendlReader::asset_reader_unverified`], and
//!   [`BendlReader::asset_payload_reader_unverified`] are the explicit recovery/debug escape
//!   hatches; they never surface a [`ChecksumError`].
//! - [`BendlReader::verify_asset_checksum`] and [`BendlReader::verify_all_asset_checksums`] are
//!   explicit raw-bytes verifiers (no decoding) that do not return decoded payload bytes.

use std::io::{self, Read, Seek, SeekFrom};

use xz2::read::XzDecoder;

use super::error::{BendlReadError, ChecksumError, ChecksumTarget};
use super::format::{
    read_directory, standardized_name_for, AssignmentFormat, BendlDirectoryEntry, BendlFormatError,
    BendlHeader, ASSET_FLAG_XZ,
};
use super::verify::{
    scan_range_crc32c, CrcTeeReader, ExactLen, ShortRangeAwareReader, ShortRangeFlag,
    ShortRangeMarker, VerifyingReader,
};
use crate::io::reader::{BenStreamReader, BenWireFormat};

pub use super::verify::BendlVerifiedStreamReader;

impl From<AssignmentFormat> for BenWireFormat {
    fn from(format: AssignmentFormat) -> Self {
        match format {
            AssignmentFormat::Ben => BenWireFormat::Ben,
            AssignmentFormat::Xben => BenWireFormat::XBen,
        }
    }
}

/// Reader for a single `.bendl` file.
pub struct BendlReader<R: Read + Seek> {
    inner: R,
    header: BendlHeader,
    directory: Vec<BendlDirectoryEntry>,
}

impl<R: Read + Seek> BendlReader<R> {
    /// Open a `.bendl` file by validating its header and loading the directory table if one exists.
    ///
    /// The underlying reader is left at an unspecified position; callers should seek explicitly
    /// before reading asset or stream bytes.
    pub fn open(mut inner: R) -> Result<Self, BendlFormatError> {
        inner.seek(SeekFrom::Start(0))?;
        let header = BendlHeader::read_from(&mut inner)?;

        let directory = if header.directory_offset != 0 && header.directory_len != 0 {
            inner.seek(SeekFrom::Start(header.directory_offset))?;
            let mut bounded = (&mut inner).take(header.directory_len);
            let directory = read_directory(&mut bounded)?;
            let remaining = bounded.limit();
            if remaining != 0 {
                return Err(BendlFormatError::TrailingDirectoryBytes { remaining });
            }
            validate_directory_entries(&directory)
                .map_err(|e| BendlFormatError::MalformedDirectory(e.to_string()))?;
            directory
        } else {
            Vec::new()
        };

        Ok(BendlReader {
            inner,
            header,
            directory,
        })
    }

    /// The parsed fixed header.
    pub fn header(&self) -> &BendlHeader {
        &self.header
    }

    /// Whether the bundle was successfully finalized.
    pub fn is_finalized(&self) -> bool {
        self.header.is_finalized()
    }

    /// The sample count recorded in the header, or `None` if not authoritative (i.e. the bundle is
    /// still incomplete).
    pub fn sample_count(&self) -> Option<i64> {
        if self.header.is_finalized() {
            Some(self.header.sample_count)
        } else {
            None
        }
    }

    /// The container format of the embedded assignment stream.
    pub fn assignment_format(&self) -> Option<AssignmentFormat> {
        self.header.assignment_format_typed()
    }

    /// All directory entries in the order they appear in the directory.
    pub fn assets(&self) -> &[BendlDirectoryEntry] {
        &self.directory
    }

    /// Look up a directory entry by canonical or custom name.
    pub fn find_asset_by_name(&self, name: &str) -> Option<&BendlDirectoryEntry> {
        self.directory.iter().find(|e| e.name == name)
    }

    /// Look up the unique directory entry with the given asset type, if any. Singleton types
    /// (`metadata.json`, `graph.json`, `node_permutation_map.json`) use this to grab their payload
    /// without caring about the standardized name.
    pub fn find_asset_by_type(&self, asset_type: u16) -> Option<&BendlDirectoryEntry> {
        self.directory.iter().find(|e| e.asset_type == asset_type)
    }

    /// Resolve the stored stream CRC32C, enforcing the stream-checksum precondition once.
    ///
    /// Returns `Err(BundleIncomplete)` for unfinalized bundles (the stored `stream_checksum` is not
    /// authoritative until the bundle is finalized) and `Err(Unavailable)` when
    /// `HEADER_FLAG_STREAM_CHECKSUM` is clear (foreign or hand-built bytes; the library writer
    /// always sets this flag). The finalization check comes first by design: reporting
    /// `Unavailable` for an unfinalized bundle would be misleading.
    fn require_stream_checksum(&self) -> Result<u32, BendlReadError> {
        if !self.header.is_finalized() {
            return Err(BendlReadError::Checksum(ChecksumError::BundleIncomplete {
                target: ChecksumTarget::Stream,
            }));
        }
        if !self.header.has_stream_checksum() {
            return Err(BendlReadError::Checksum(ChecksumError::Unavailable {
                target: ChecksumTarget::Stream,
            }));
        }
        Ok(self.header.stream_checksum)
    }

    /// Return the byte range occupied by the assignment stream.
    ///
    /// For finalized bundles this is `(stream_offset, stream_len)` as recorded in the header. For
    /// incomplete bundles the end of the stream is taken as EOF (or the directory start, if a
    /// provisional directory was written).
    pub fn assignment_stream_range(&mut self) -> io::Result<(u64, u64)> {
        if self.header.is_finalized() {
            Ok((self.header.stream_offset, self.header.stream_len))
        } else {
            let end = if self.header.directory_offset != 0 {
                self.header.directory_offset
            } else {
                self.inner.seek(SeekFrom::End(0))?
            };
            let len = end.saturating_sub(self.header.stream_offset);
            Ok((self.header.stream_offset, len))
        }
    }

    /// Return a verified reader for the assignment stream that checks the stored CRC32C at raw EOF.
    ///
    /// Returns `Err(ChecksumError::BundleIncomplete)` for unfinalized bundles and
    /// `Err(ChecksumError::Unavailable)` when `HEADER_FLAG_STREAM_CHECKSUM` is clear.
    ///
    /// On success, CRC mismatch surfaces from `Read::read` as
    /// `io::Error::new(io::ErrorKind::InvalidData, ChecksumError::Mismatch)` on the call that
    /// would otherwise return `Ok(0)` at raw EOF. For a raw copy that decodes nothing, driving the
    /// returned reader to EOF is sufficient. For decoded access use
    /// [`BendlReader::open_assignment_reader`].
    pub fn assignment_stream_reader(&mut self) -> Result<Box<dyn Read + '_>, BendlReadError> {
        let expected = self.require_stream_checksum()?;
        let (offset, len) = self.assignment_stream_range()?;
        self.inner.seek(SeekFrom::Start(offset))?;
        let short_flag = ShortRangeFlag::new();
        let raw = ExactLen::new(&mut self.inner, len, short_flag.clone());
        Ok(Box::new(VerifyingReader::new(
            CrcTeeReader::new(raw),
            expected,
            ChecksumTarget::Stream,
            short_flag,
        )))
    }

    /// Return a raw bounded reader for the assignment stream **without** CRC verification.
    ///
    /// Works on both finalized and unfinalized bundles. Useful for recovery/debug flows and for
    /// callers that need the raw bytes without the overhead of a CRC check.
    pub fn assignment_stream_reader_unverified(&mut self) -> io::Result<Box<dyn Read + '_>> {
        let (offset, len) = self.assignment_stream_range()?;
        self.inner.seek(SeekFrom::Start(offset))?;
        Ok(Box::new(ExactLen::new(
            &mut self.inner,
            len,
            ShortRangeFlag::new(),
        )))
    }

    /// Construct a verified decoded assignment reader that checks the stream CRC32C after the
    /// codec reaches EOF. The returned [`BendlVerifiedStreamReader`] exposes only full-consumption
    /// APIs, because partial frame/subsample iteration cannot prove the whole stream checksum.
    ///
    /// Returns `Err(BundleIncomplete)` for unfinalized bundles and `Err(Unavailable)` when the
    /// stream checksum flag is clear.
    pub fn open_assignment_reader(
        &mut self,
    ) -> Result<BendlVerifiedStreamReader<'_, R>, BendlReadError> {
        let expected = self.require_stream_checksum()?;
        let format = self.assignment_format().ok_or({
            BendlReadError::Format(BendlFormatError::UnknownAssignmentFormat(
                self.header.assignment_format,
            ))
        })?;
        let (offset, len) = self.assignment_stream_range()?;
        self.inner.seek(SeekFrom::Start(offset))?;
        let short_flag = ShortRangeFlag::new();
        let raw = ExactLen::new(&mut self.inner, len, short_flag.clone());

        BendlVerifiedStreamReader::new(raw, short_flag, expected, |source| match format {
            AssignmentFormat::Ben => BenStreamReader::from_ben(source).map_err(Into::into),
            AssignmentFormat::Xben => BenStreamReader::from_xben(source).map_err(Into::into),
        })
    }

    /// Construct a decoded assignment reader without CRC verification.
    ///
    /// This is the explicit escape hatch for partial/random-access decode paths such as
    /// `into_frames` and `into_subsample_by_*`: those operations intentionally stop before raw EOF,
    /// so they cannot verify the whole stream checksum. Call [`Self::verify_stream_checksum`]
    /// separately when whole-stream integrity matters.
    pub fn open_assignment_reader_unverified(
        &mut self,
    ) -> Result<BenStreamReader<Box<dyn Read + Send + '_>>, BendlReadError>
    where
        R: Send,
    {
        let format = self.assignment_format().ok_or({
            BendlReadError::Format(BendlFormatError::UnknownAssignmentFormat(
                self.header.assignment_format,
            ))
        })?;
        let (offset, len) = self.assignment_stream_range()?;
        self.inner.seek(SeekFrom::Start(offset))?;
        let raw: Box<dyn Read + Send + '_> =
            Box::new(ExactLen::new(&mut self.inner, len, ShortRangeFlag::new()));
        match format {
            AssignmentFormat::Ben => BenStreamReader::from_ben(raw).map_err(Into::into),
            AssignmentFormat::Xben => BenStreamReader::from_xben(raw).map_err(Into::into),
        }
    }

    /// Verify the stored stream CRC32C by scanning the raw on-disk bytes of the assignment stream.
    ///
    /// This is the explicit full-scan verifier for callers that want to check integrity without
    /// decoding the stream. For random-access extraction (which intentionally skips untouched
    /// frames), call this separately to confirm the whole stream is intact.
    ///
    /// Returns `Err(BundleIncomplete)` for unfinalized bundles and `Err(Unavailable)` when the
    /// stream checksum flag is clear.
    pub fn verify_stream_checksum(&mut self) -> Result<(), BendlReadError> {
        let expected = self.require_stream_checksum()?;
        let (offset, len) = self.assignment_stream_range()?;
        let computed = scan_range_crc32c(&mut self.inner, offset, len)?;
        if computed != expected {
            return Err(BendlReadError::Checksum(ChecksumError::Mismatch {
                target: ChecksumTarget::Stream,
                computed,
                expected,
            }));
        }
        Ok(())
    }

    /// Seek to an asset's `payload_offset` and return a reader bounded to its declared
    /// `payload_len`, paired with the [`ShortRangeFlag`] that reader will set if the backing range
    /// is shorter than declared.
    ///
    /// This is the raw on-disk byte range shared by every asset read mode (verified/unverified,
    /// decoded/raw); the codec and CRC layering is applied by each caller on top of the returned
    /// range. It is scoped to `entry`-based reads and intentionally does not cover the
    /// assignment-stream readers, which seek to a separately computed `(offset, len)`.
    fn open_asset_payload_range(
        &mut self,
        entry: &BendlDirectoryEntry,
    ) -> io::Result<(ExactLen<&mut R>, ShortRangeFlag)> {
        self.inner.seek(SeekFrom::Start(entry.payload_offset))?;
        let short_flag = ShortRangeFlag::new();
        let raw = ExactLen::new(&mut self.inner, entry.payload_len, short_flag.clone());
        Ok((raw, short_flag))
    }

    /// Read the fully-decoded bytes of an asset by directory entry, verifying its CRC32C before
    /// returning.
    ///
    /// **Contract:** this is exactly `asset_reader(entry)? then read_to_end`, one behavioral path
    /// shared with the streaming API so the two cannot drift apart. Implications:
    ///
    /// - Uncompressed asset, payload byte flipped → the CRC tee observes the mismatch at raw EOF
    ///   and returns [`BendlReadError::Checksum`].
    /// - Uncompressed asset, stored CRC bytes flipped → same; the tee compares computed-vs-stored
    ///   at EOF.
    /// - xz-compressed asset with broken xz framing → the xz decoder fails before the raw tee
    ///   reaches EOF; surface is [`BendlReadError::Decode`]. (CRC is over compressed bytes, but the
    ///   decoder's failure precedes any CRC check.)
    /// - xz-compressed asset with intact xz but wrong stored CRC → codec reaches EOF, BENDL-owned
    ///   wrapper checks CRC, returns [`BendlReadError::Checksum`].
    /// - Entry has `ASSET_FLAG_CHECKSUM` clear (foreign/hand-built bytes; the library writer never
    ///   produces this) → [`ChecksumError::Unavailable`].
    pub fn asset_bytes(&mut self, entry: &BendlDirectoryEntry) -> Result<Vec<u8>, BendlReadError> {
        let mut out = Vec::new();
        let mut reader = self.asset_reader(entry)?;
        match reader.read_to_end(&mut out) {
            Ok(_) => Ok(out),
            Err(e) => Err(classify_read_error(e, entry)),
        }
    }

    /// Same as [`Self::asset_bytes`] but skips CRC verification.
    ///
    /// Never returns [`BendlReadError::Checksum`]. Other variants (I/O, codec) still apply;
    /// corrupted xz framing still surfaces as [`BendlReadError::Decode`].
    pub fn asset_bytes_unverified(
        &mut self,
        entry: &BendlDirectoryEntry,
    ) -> Result<Vec<u8>, BendlReadError> {
        let mut out = Vec::new();
        let mut reader = self.asset_reader_unverified(entry)?;
        match reader.read_to_end(&mut out) {
            Ok(_) => Ok(out),
            Err(e) => Err(classify_read_error(e, entry)),
        }
    }

    /// Obtain a boxed reader for the decoded contents of an asset, with CRC32C verification at EOF.
    ///
    /// The returned reader is positioned at the first decoded byte and automatically handles xz
    /// decompression when the asset is flagged as compressed. The reader borrows `self`, so only
    /// one asset or stream reader may be live at a time.
    ///
    /// Checksum mismatch surfaces from `Read::read` as
    /// `io::Error::new(io::ErrorKind::InvalidData, ChecksumError)` on the call that would otherwise
    /// return `Ok(0)` at EOF. Early-drop or partial-read callers do **not** observe verification;
    /// the reader must be driven to EOF for the CRC to be checked.
    pub fn asset_reader<'a>(
        &'a mut self,
        entry: &BendlDirectoryEntry,
    ) -> Result<Box<dyn Read + 'a>, BendlReadError> {
        let expected = match entry.checksum_u32() {
            Some(c) => c,
            None => {
                return Err(BendlReadError::Checksum(ChecksumError::Unavailable {
                    target: ChecksumTarget::Asset(entry.name.clone()),
                }));
            }
        };
        let target = ChecksumTarget::Asset(entry.name.clone());

        let (raw, short_flag) = self.open_asset_payload_range(entry)?;

        // The CRC tee always sits at the raw on-disk layer (over the compressed bytes for xz
        // assets, so verification happens before decompression). For xz assets the decoder sits
        // above the tee; the verifying wrapper finalizes the check once the source reaches EOF.
        if entry.asset_flags & ASSET_FLAG_XZ != 0 {
            Ok(Box::new(VerifyingReader::new(
                XzDecoder::new(CrcTeeReader::new(raw)),
                expected,
                target,
                short_flag,
            )))
        } else {
            Ok(Box::new(VerifyingReader::new(
                CrcTeeReader::new(raw),
                expected,
                target,
                short_flag,
            )))
        }
    }

    /// Decoded reader without CRC verification: the explicit escape hatch for recovery/debug or
    /// `--no-verify` flows.
    ///
    /// If the asset is xz-flagged the returned bytes are still decompressed; "unverified" only
    /// disables the CRC check.
    pub fn asset_reader_unverified<'a>(
        &'a mut self,
        entry: &BendlDirectoryEntry,
    ) -> Result<Box<dyn Read + 'a>, BendlReadError> {
        let (raw, short_flag) = self.open_asset_payload_range(entry)?;
        if entry.asset_flags & ASSET_FLAG_XZ != 0 {
            // Wrap the decoder so that if xz reports a runtime error while the underlying
            // ExactLen has flagged a short read, the surface is a short-range UnexpectedEof
            // rather than a codec error.
            Ok(Box::new(ShortRangeAwareReader::new(
                XzDecoder::new(raw),
                short_flag,
            )))
        } else {
            Ok(Box::new(raw))
        }
    }

    /// Raw on-disk payload reader without CRC verification, kept distinct from
    /// [`Self::asset_reader_unverified`] so that callers doing low-level recovery never
    /// accidentally emit decompressed bytes (or, conversely, never accidentally emit compressed
    /// bytes expecting raw).
    ///
    /// For an xz-flagged asset this yields the compressed payload bytes byte-for-byte; for an
    /// uncompressed asset it is the same as [`Self::asset_reader_unverified`].
    pub fn asset_payload_reader_unverified<'a>(
        &'a mut self,
        entry: &BendlDirectoryEntry,
    ) -> Result<Box<dyn Read + 'a>, BendlReadError> {
        // No codec or CRC layer sits above this range, so the short-range flag has nothing to
        // observe it; a short read surfaces directly as the ExactLen's own marker.
        let (raw, _short_flag) = self.open_asset_payload_range(entry)?;
        Ok(Box::new(raw))
    }

    /// Verify the stored CRC32C of a single asset without returning any decoded bytes.
    ///
    /// The CRC is over the raw on-disk payload bytes; no decoder is invoked, so corrupted xz
    /// framing under an intact stored CRC will still report `Ok(())` (or, conversely, an intact xz
    /// payload with a corrupted stored CRC will deterministically report
    /// [`ChecksumError::Mismatch`]).
    pub fn verify_asset_checksum(
        &mut self,
        entry: &BendlDirectoryEntry,
    ) -> Result<(), BendlReadError> {
        let expected = match entry.checksum_u32() {
            Some(c) => c,
            None => {
                return Err(BendlReadError::Checksum(ChecksumError::Unavailable {
                    target: ChecksumTarget::Asset(entry.name.clone()),
                }));
            }
        };

        let computed = scan_range_crc32c(&mut self.inner, entry.payload_offset, entry.payload_len)?;
        if computed != expected {
            return Err(BendlReadError::Checksum(ChecksumError::Mismatch {
                target: ChecksumTarget::Asset(entry.name.clone()),
                computed,
                expected,
            }));
        }
        Ok(())
    }

    /// Verify every asset's CRC in directory order. Returns the **first** mismatch encountered and
    /// stops; callers that want a full audit should iterate the directory and call
    /// [`Self::verify_asset_checksum`] per entry themselves.
    pub fn verify_all_asset_checksums(&mut self) -> Result<(), BendlReadError> {
        // Clone the entries so we don't borrow self.directory across the seek/read calls on
        // self.inner.
        let entries = self.directory.clone();
        for entry in &entries {
            self.verify_asset_checksum(entry)?;
        }
        Ok(())
    }

    /// Validate that the loaded directory is well-formed under the canonical-name and uniqueness
    /// rules.
    ///
    /// Returns [`BundleValidationError`] if any entry violates the rules. This is called
    /// automatically by [`BendlReader::open`].
    pub fn validate_directory(&self) -> Result<(), BundleValidationError> {
        validate_directory_entries(&self.directory)
    }
}

/// Map a `read_to_end`-time `io::Error` (or any `Read`-derived `io::Error`) into the right
/// [`BendlReadError`] variant.
///
/// The wrap discipline is held here, in one place: a `ChecksumError`-bearing `io::Error` becomes
/// [`BendlReadError::Checksum`]; everything else fans out into `Io` vs `Decode` according to
/// context. Codec-runtime errors from xz/BEN go to [`BendlReadError::Decode`] when the entry is
/// xz-flagged; raw payload errors stay `Io`.
fn classify_read_error(err: io::Error, entry: &BendlDirectoryEntry) -> BendlReadError {
    // Bundle-layer short-range failures map to Io regardless of asset flags. A backing file
    // shorter than payload_len is a structural bundle problem, not a codec error.
    if err.get_ref().is_some_and(|e| e.is::<ShortRangeMarker>()) {
        return BendlReadError::Io(err);
    }
    if err.get_ref().is_some_and(|e| e.is::<ChecksumError>()) {
        match err
            .into_inner()
            .map(|boxed| boxed.downcast::<ChecksumError>())
        {
            Some(Ok(boxed)) => return BendlReadError::Checksum(*boxed),
            Some(Err(other)) => {
                // Downcast failed unexpectedly; reconstruct an io::Error around the still-boxed
                // payload so we don't lose context.
                return BendlReadError::Io(io::Error::new(io::ErrorKind::InvalidData, other));
            }
            None => {
                return BendlReadError::Io(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "checksum error with no payload",
                ));
            }
        }
    }
    if entry.asset_flags & ASSET_FLAG_XZ != 0 {
        BendlReadError::Decode(err)
    } else {
        BendlReadError::Io(err)
    }
}

/// Validate that every entry's payload range lies within the backing file.
///
/// Read paths stay lenient at open (a truncated bundle remains inspectable, and every byte
/// access surfaces a strict-EOF error at touch), but paths that *trust* the declared lengths
/// (the appender, which carries entries into a rewritten directory, and in-place compaction,
/// which sizes allocations and the new layout from them) must reject out-of-range extents up
/// front, so a corrupt or malicious length surfaces as an error instead of an oversized
/// reservation or a garbage layout.
pub(crate) fn validate_entry_extents(
    directory: &[BendlDirectoryEntry],
    file_len: u64,
) -> Result<(), BundleValidationError> {
    for entry in directory {
        let in_bounds = entry
            .payload_offset
            .checked_add(entry.payload_len)
            .is_some_and(|end| end <= file_len);
        if !in_bounds {
            return Err(BundleValidationError::PayloadOutOfBounds {
                name: entry.name.clone(),
                payload_offset: entry.payload_offset,
                payload_len: entry.payload_len,
                file_len,
            });
        }
    }
    Ok(())
}

pub(crate) fn validate_directory_entries(
    directory: &[BendlDirectoryEntry],
) -> Result<(), BundleValidationError> {
    let mut seen_names = std::collections::HashSet::new();

    for entry in directory {
        if !seen_names.insert(entry.name.as_str()) {
            return Err(BundleValidationError::DuplicateName(entry.name.clone()));
        }
        if let Some(canonical) = standardized_name_for(entry.asset_type) {
            if entry.name != canonical {
                return Err(BundleValidationError::WrongCanonicalName {
                    asset_type: entry.asset_type,
                    expected: canonical.to_string(),
                    found: entry.name.clone(),
                });
            }
        }
    }
    Ok(())
}

/// Errors raised when a directory violates the canonical-name or uniqueness rules.
#[derive(Debug, thiserror::Error)]
pub enum BundleValidationError {
    /// Two entries share the same name.
    #[error("duplicate asset name: {0:?}")]
    DuplicateName(String),

    /// An entry with a known singleton type is not using its standardized name.
    #[error("asset type {asset_type} must use standardized name {expected:?}, found {found:?}")]
    WrongCanonicalName {
        /// The asset type whose standardized name was violated.
        asset_type: u16,
        /// The standardized name the writer should have used.
        expected: String,
        /// The name that was actually written.
        found: String,
    },

    /// An entry's payload range extends beyond the end of the file.
    #[error(
        "asset {name:?} declares payload bytes {payload_offset}..{payload_offset}+{payload_len} \
         beyond the file end ({file_len} bytes)"
    )]
    PayloadOutOfBounds {
        /// The asset whose payload range is out of bounds.
        name: String,
        /// The payload offset declared in the directory.
        payload_offset: u64,
        /// The payload length declared in the directory.
        payload_len: u64,
        /// The actual length of the backing file.
        file_len: u64,
    },
}
