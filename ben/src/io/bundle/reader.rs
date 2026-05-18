//! Read-only inspection of `.bendl` files.
//!
//! A [`BendlReader`] parses a bundle's fixed header and (if present) its trailing directory table.
//! It does not read any asset payload bytes until the caller explicitly requests them via
//! [`BendlReader::asset_bytes`] or [`BendlReader::asset_reader`]. The assignment stream region is
//! likewise exposed as a byte range the caller can plumb into a [`BenStreamReader`] without this
//! module reinterpreting any BEN/XBEN internals.
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

use std::io::{self, Read, Seek, SeekFrom, Take};

use xz2::read::XzDecoder;

use super::error::{BendlReadError, ChecksumError, ChecksumTarget};
use super::format::{
    read_directory, standardized_name_for, AssignmentFormat, BendlDirectoryEntry, BendlFormatError,
    BendlHeader, ASSET_FLAG_XZ,
};
use crate::io::reader::{BenStreamReader, BenWireFormat};

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

    /// Return a `Take` reader positioned at the start of the assignment stream and limited to its
    /// declared length. The caller is expected to wrap the returned reader in a [`BenStreamReader`]
    /// (via [`BendlReader::open_assignment_reader`] or directly) as appropriate for
    /// [`BendlReader::assignment_format`].
    pub fn assignment_stream_reader(&mut self) -> io::Result<Take<&mut R>> {
        let (offset, len) = self.assignment_stream_range()?;
        self.inner.seek(SeekFrom::Start(offset))?;
        Ok((&mut self.inner).take(len))
    }

    /// Construct the appropriate assignment decoder for the bundle's declared `assignment_format`
    /// and return it as a [`BenStreamReader`] over the bundle's bounded stream region.
    ///
    /// Returns an error if the header's `assignment_format` field is unrecognized or the embedded
    /// banner is malformed.
    pub fn open_assignment_reader(
        &mut self,
    ) -> Result<BenStreamReader<Take<&mut R>>, BundleAssignmentReaderError> {
        let format = self.assignment_format().ok_or(
            BundleAssignmentReaderError::UnknownAssignmentFormat(self.header.assignment_format),
        )?;
        let stream = self.assignment_stream_reader()?;
        match format {
            AssignmentFormat::Ben => {
                BenStreamReader::from_ben(stream).map_err(BundleAssignmentReaderError::Decoder)
            }
            AssignmentFormat::Xben => {
                BenStreamReader::from_xben(stream).map_err(BundleAssignmentReaderError::Decoder)
            }
        }
    }

    /// Read the fully-decoded bytes of an asset by directory entry, verifying its CRC32C before
    /// returning.
    ///
    /// **Contract:** this is exactly `asset_reader(entry)? then read_to_end` — one behavioral path
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
    /// Never returns [`BendlReadError::Checksum`]. Other variants (I/O, codec) still apply —
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
    /// return `Ok(0)` at EOF. Early-drop or partial-read callers do **not** observe verification —
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

        self.inner.seek(SeekFrom::Start(entry.payload_offset))?;
        let raw = (&mut self.inner).take(entry.payload_len);

        if entry.asset_flags & ASSET_FLAG_XZ != 0 {
            // Compressed: CRC tee sits *inside* the XzDecoder so the tee accumulates over raw
            // compressed bytes; the BENDL-owned wrapper around the decoder finalizes the check
            // after the codec reaches its own EOF.
            let tee = CrcTeeReader::new(raw);
            let decoder = XzDecoder::new(tee);
            Ok(Box::new(DecodedVerifyingReader {
                decoder,
                expected,
                target,
                state: VerifyState::Reading,
            }))
        } else {
            Ok(Box::new(RawVerifyingReader {
                inner: raw,
                hasher: 0,
                expected,
                target,
                state: VerifyState::Reading,
            }))
        }
    }

    /// Decoded reader without CRC verification — explicit escape hatch for recovery/debug or
    /// `--no-verify` flows.
    ///
    /// If the asset is xz-flagged the returned bytes are still decompressed; "unverified" only
    /// disables the CRC check.
    pub fn asset_reader_unverified<'a>(
        &'a mut self,
        entry: &BendlDirectoryEntry,
    ) -> Result<Box<dyn Read + 'a>, BendlReadError> {
        self.inner.seek(SeekFrom::Start(entry.payload_offset))?;
        let raw = (&mut self.inner).take(entry.payload_len);
        if entry.asset_flags & ASSET_FLAG_XZ != 0 {
            Ok(Box::new(XzDecoder::new(raw)))
        } else {
            Ok(Box::new(raw))
        }
    }

    /// Raw on-disk payload reader without CRC verification — kept distinct from
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
        self.inner.seek(SeekFrom::Start(entry.payload_offset))?;
        Ok(Box::new((&mut self.inner).take(entry.payload_len)))
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

        self.inner.seek(SeekFrom::Start(entry.payload_offset))?;
        let mut remaining = entry.payload_len;
        let mut buf = [0u8; 64 * 1024];
        let mut hasher: u32 = 0;
        while remaining > 0 {
            let want = remaining.min(buf.len() as u64) as usize;
            let n = self.inner.read(&mut buf[..want])?;
            if n == 0 {
                // Short read against the declared payload length — surface as an I/O error so
                // callers can distinguish a truncated bundle from a CRC mismatch.
                return Err(BendlReadError::Io(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    format!(
                        "asset {:?} payload ended {} byte(s) before declared length",
                        entry.name, remaining
                    ),
                )));
            }
            hasher = crc32c::crc32c_append(hasher, &buf[..n]);
            remaining -= n as u64;
        }

        if hasher != expected {
            return Err(BendlReadError::Checksum(ChecksumError::Mismatch {
                target: ChecksumTarget::Asset(entry.name.clone()),
                computed: hasher,
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
    /// automatically by [`BendlReader::open`] when the `strict` constructor is used in tests; in
    /// normal reads, the writer is already expected to enforce these rules and a malformed bundle
    /// is a program bug somewhere else.
    pub fn validate_directory(&self) -> Result<(), BundleValidationError> {
        validate_directory_entries(&self.directory)
    }
}

// ---------------------------------------------------------------------------
// Verifying reader plumbing
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VerifyState {
    /// Still feeding bytes from the underlying reader.
    Reading,
    /// Underlying reader returned EOF and the CRC matched. Subsequent reads return `Ok(0)`
    /// (normal EOF).
    EofChecked,
    /// CRC mismatch was reported to the caller. Subsequent reads return `Ok(0)` so the reader stays
    /// well-behaved if the caller re-polls after the error.
    Failed,
}

/// Uncompressed-asset verifying reader: forwards bytes from the bounded payload, accumulates CRC32C
/// as they fly past, and on raw EOF either confirms the checksum or returns
/// [`ChecksumError::Mismatch`] in place of the usual `Ok(0)`.
struct RawVerifyingReader<'a, R: Read + Seek> {
    inner: Take<&'a mut R>,
    hasher: u32,
    expected: u32,
    target: ChecksumTarget,
    state: VerifyState,
}

impl<R: Read + Seek> Read for RawVerifyingReader<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self.state {
            VerifyState::EofChecked | VerifyState::Failed => return Ok(0),
            VerifyState::Reading => {}
        }
        let n = self.inner.read(buf)?;
        if n == 0 {
            if self.hasher == self.expected {
                self.state = VerifyState::EofChecked;
                return Ok(0);
            }
            let err = ChecksumError::Mismatch {
                target: self.target.clone(),
                computed: self.hasher,
                expected: self.expected,
            };
            self.state = VerifyState::Failed;
            return Err(io::Error::new(io::ErrorKind::InvalidData, err));
        }
        self.hasher = crc32c::crc32c_append(self.hasher, &buf[..n]);
        Ok(n)
    }
}

/// CRC accumulator that sits *inside* an [`XzDecoder`] for compressed assets. It must never
/// substitute a checksum error for raw EOF — the codec needs to see the natural `Ok(0)` so it can
/// flush pending output. The post-decoder wrapper ([`DecodedVerifyingReader`]) inspects this
/// struct's accumulated hash after codec EOF.
struct CrcTeeReader<R: Read> {
    inner: R,
    hasher: u32,
}

impl<R: Read> CrcTeeReader<R> {
    fn new(inner: R) -> Self {
        Self { inner, hasher: 0 }
    }
}

impl<R: Read> Read for CrcTeeReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        if n > 0 {
            self.hasher = crc32c::crc32c_append(self.hasher, &buf[..n]);
        }
        Ok(n)
    }
}

/// Verifying wrapper around an `XzDecoder<CrcTeeReader<…>>`. Lets the codec observe normal raw EOF
/// before finalizing the CRC check at the decoded layer.
struct DecodedVerifyingReader<'a, R: Read + Seek> {
    decoder: XzDecoder<CrcTeeReader<Take<&'a mut R>>>,
    expected: u32,
    target: ChecksumTarget,
    state: VerifyState,
}

impl<R: Read + Seek> Read for DecodedVerifyingReader<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self.state {
            VerifyState::EofChecked | VerifyState::Failed => return Ok(0),
            VerifyState::Reading => {}
        }
        let n = self.decoder.read(buf)?;
        if n == 0 {
            let computed = self.decoder.get_ref().hasher;
            if computed == self.expected {
                self.state = VerifyState::EofChecked;
                return Ok(0);
            }
            let err = ChecksumError::Mismatch {
                target: self.target.clone(),
                computed,
                expected: self.expected,
            };
            self.state = VerifyState::Failed;
            return Err(io::Error::new(io::ErrorKind::InvalidData, err));
        }
        Ok(n)
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
    if err.get_ref().is_some_and(|e| e.is::<ChecksumError>()) {
        match err
            .into_inner()
            .map(|boxed| boxed.downcast::<ChecksumError>())
        {
            Some(Ok(boxed)) => return BendlReadError::Checksum(*boxed),
            Some(Err(other)) => {
                // Downcast failed unexpectedly — reconstruct an io::Error around the still-boxed
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

/// Errors raised by [`BendlReader::open_assignment_reader`].
#[derive(Debug, thiserror::Error)]
pub enum BundleAssignmentReaderError {
    /// The header's `assignment_format` byte did not map to a known format.
    #[error("unknown assignment_format in bundle header: {0}")]
    UnknownAssignmentFormat(u8),
    /// The embedded BEN/XBEN decoder rejected the stream banner.
    #[error(transparent)]
    Decoder(#[from] crate::io::reader::DecoderInitError),
    /// An underlying I/O error occurred while seeking to the stream.
    #[error(transparent)]
    Io(#[from] io::Error),
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
}
