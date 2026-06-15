//! Strict-length and CRC-verification plumbing for the `.bendl` reader.
//!
//! This module owns the byte-level read adapters that the public [`super::reader::BendlReader`] API
//! composes:
//!
//! - [`ExactLen`] turns a backing range shorter than its declared length into a structural
//!   short-range error rather than a silently-short successful read.
//! - [`CrcTeeReader`] accumulates a CRC32C over the bytes that flow through it, without ever
//!   substituting an error for raw EOF.
//! - [`VerifyingReader`] wraps a CRC-accumulating byte source and, at the source's natural EOF,
//!   either confirms the stored CRC32C or surfaces [`ChecksumError::Mismatch`] in place of the
//!   usual `Ok(0)`. The same wrapper serves uncompressed assets (source =
//!   `CrcTeeReader<ExactLen<…>>`) and xz-compressed assets (source =
//!   `XzDecoder<CrcTeeReader<ExactLen<…>>>`): the only difference is *where* the tee sits, which
//!   the [`CrcSource`] trait abstracts.
//! - [`BendlVerifiedStreamReader`] folds the same verify-at-EOF discipline into full-consumption
//!   assignment-stream APIs.

use std::fmt;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::sync::{
    atomic::{AtomicBool, AtomicU32, Ordering},
    Arc,
};

use serde_json::json;
use xz2::read::XzDecoder;

use super::error::{BendlReadError, ChecksumError, ChecksumTarget};
use crate::io::reader::{BenStreamReader, BenWireFormat};
use crate::BenVariant;

// =====================================================================
// Strict-length plumbing
// =====================================================================

/// Marker error attached to the `io::Error` returned when an [`ExactLen`] reader hits underlying
/// EOF before consuming its declared length. Used by convenience APIs to recognise a bundle-layer
/// short-range failure even when it has surfaced through a codec.
#[derive(Debug)]
pub(crate) struct ShortRangeMarker {
    pub remaining: u64,
}

impl fmt::Display for ShortRangeMarker {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "bundle range ended {} byte(s) before declared length",
            self.remaining
        )
    }
}

impl std::error::Error for ShortRangeMarker {}

/// Shared flag set by an [`ExactLen`] reader when the underlying reader runs out of bytes before
/// the declared length is reached. Clones share state so a wrapper above a codec can detect the
/// short read even if the codec swallows the inner `UnexpectedEof` in favor of its own error.
#[derive(Clone, Default)]
pub(crate) struct ShortRangeFlag(Arc<AtomicBool>);

impl ShortRangeFlag {
    pub(crate) fn new() -> Self {
        Self(Arc::new(AtomicBool::new(false)))
    }

    pub(crate) fn set(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    pub(crate) fn get(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

/// Bounded reader that enforces an exact byte length. Behaves like [`std::io::Take`] for reads
/// within the declared length, but returns
/// `Err(io::Error::new(io::ErrorKind::UnexpectedEof, ShortRangeMarker))` (and sets the shared
/// [`ShortRangeFlag`]) if the underlying reader signals EOF before the declared length is reached.
///
/// `ExactLen` is the BENDL-layer guarantee that `payload_len` and `stream_len` are exact byte
/// counts of the on-disk range; a backing file shorter than declared is a corrupt bundle, not a
/// short successful read.
pub struct ExactLen<R: Read> {
    inner: R,
    remaining: u64,
    flag: ShortRangeFlag,
}

impl<R: Read> ExactLen<R> {
    pub(crate) fn new(inner: R, declared: u64, flag: ShortRangeFlag) -> Self {
        Self {
            inner,
            remaining: declared,
            flag,
        }
    }

    /// Bound a reader to an exact byte length with a private short-range flag.
    ///
    /// For callers that want the exact-length guarantee but do not wrap a codec with a
    /// [`ShortRangeAwareReader`] above the bound, so they have no use for a shared flag. A backing
    /// range shorter than `declared` still surfaces as `UnexpectedEof`.
    pub fn bounded(inner: R, declared: u64) -> Self {
        Self::new(inner, declared, ShortRangeFlag::new())
    }
}

impl<R: Read> Read for ExactLen<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.remaining == 0 || buf.is_empty() {
            return Ok(0);
        }
        let max = (buf.len() as u64).min(self.remaining) as usize;
        let n = self.inner.read(&mut buf[..max])?;
        if n == 0 {
            // Underlying reader hit EOF before our declared length. Set the shared flag so a
            // wrapper above a codec can recognise this as a bundle-range failure, and surface as
            // UnexpectedEof carrying the marker.
            let remaining = self.remaining;
            self.flag.set();
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                ShortRangeMarker { remaining },
            ));
        }
        self.remaining -= n as u64;
        Ok(n)
    }
}

/// Wraps a reader sitting above an [`ExactLen`]-bounded source. If the underlying reader returns
/// an error and the shared `ShortRangeFlag` is set, the error is replaced with an `UnexpectedEof`
/// carrying a [`ShortRangeMarker`] so callers see a bundle-layer short-range failure rather than a
/// codec-specific error message.
pub(crate) struct ShortRangeAwareReader<R: Read> {
    inner: R,
    short_flag: ShortRangeFlag,
}

impl<R: Read> ShortRangeAwareReader<R> {
    pub(crate) fn new(inner: R, short_flag: ShortRangeFlag) -> Self {
        Self { inner, short_flag }
    }
}

impl<R: Read> Read for ShortRangeAwareReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self.inner.read(buf) {
            Ok(n) => Ok(n),
            Err(e) => Err(override_if_short(&self.short_flag, e)),
        }
    }
}

/// Scan an exact on-disk byte range and return its CRC32C, without decoding.
///
/// Seeks to `offset`, reads exactly `len` bytes in 64 KiB chunks, and returns the accumulated
/// CRC32C. A backing range shorter than `len` surfaces as
/// `io::Error::new(io::ErrorKind::UnexpectedEof, ShortRangeMarker)` so callers can distinguish a
/// truncated bundle from any other failure. This is the shared core behind both
/// [`super::reader::BendlReader::verify_asset_checksum`] and
/// [`super::reader::BendlReader::verify_stream_checksum`].
pub(crate) fn scan_range_crc32c<R: Read + Seek>(
    inner: &mut R,
    offset: u64,
    len: u64,
) -> io::Result<u32> {
    inner.seek(SeekFrom::Start(offset))?;
    let mut remaining = len;
    let mut buf = [0u8; 64 * 1024];
    let mut hasher: u32 = 0;
    while remaining > 0 {
        let want = remaining.min(buf.len() as u64) as usize;
        let n = inner.read(&mut buf[..want])?;
        if n == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                ShortRangeMarker { remaining },
            ));
        }
        hasher = crc32c::crc32c_append(hasher, &buf[..n]);
        remaining -= n as u64;
    }
    Ok(hasher)
}

// =====================================================================
// CRC-verifying reader plumbing
// =====================================================================

/// Build the `io::Error` used to surface a CRC mismatch through a `Read` or `Iterator` boundary.
/// The single definition keeps the kind (`InvalidData`) and inner [`ChecksumError`] shape identical
/// across every verify path.
pub(crate) fn crc_mismatch_error(
    target: ChecksumTarget,
    computed: u32,
    expected: u32,
) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        ChecksumError::Mismatch {
            target,
            computed,
            expected,
        },
    )
}

/// Build the bundle-layer short-range EOF error used whenever a wrapper above a codec detects (via
/// the shared [`ShortRangeFlag`]) that the backing range ended early but the codec reported its own
/// error instead. The exact remaining count is unknown at this layer, so it is reported as zero; a
/// raw [`ExactLen`] short read that survives untouched still carries the precise count in its own
/// [`ShortRangeMarker`].
pub(crate) fn short_range_eof() -> io::Error {
    io::Error::new(
        io::ErrorKind::UnexpectedEof,
        ShortRangeMarker { remaining: 0 },
    )
}

/// If `flag` fired, replace `err` with a bundle-layer [`short_range_eof`]; otherwise pass `err`
/// through unchanged. Centralizes the "codec swallowed the short read in favor of its own error"
/// rewrite shared by every reader that sits above an [`ExactLen`].
pub(crate) fn override_if_short(flag: &ShortRangeFlag, err: io::Error) -> io::Error {
    if flag.get() {
        short_range_eof()
    } else {
        err
    }
}

/// Compare a finalized stream CRC32C against the stored value, mapping a mismatch to the standard
/// `InvalidData`/[`ChecksumError::Mismatch`] error used across every stream verify path.
fn check_stream_crc(computed: u32, expected: u32) -> io::Result<()> {
    if computed == expected {
        Ok(())
    } else {
        Err(crc_mismatch_error(
            ChecksumTarget::Stream,
            computed,
            expected,
        ))
    }
}

/// CRC accumulator that sits between a byte source and its consumer. It never substitutes an error
/// for raw EOF; the surrounding [`VerifyingReader`] (for uncompressed assets) or the post-decoder
/// [`VerifyingReader`] (for xz assets) decides when and whether to check the accumulated hash.
pub(crate) struct CrcTeeReader<R: Read> {
    inner: R,
    hasher: u32,
}

impl<R: Read> CrcTeeReader<R> {
    pub(crate) fn new(inner: R) -> Self {
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

/// A byte source that can report the CRC32C accumulated over the raw on-disk payload bytes it has
/// passed through so far. Implemented for both the uncompressed source (`CrcTeeReader` directly)
/// and the xz-compressed source (`XzDecoder` over a `CrcTeeReader`), so a single
/// [`VerifyingReader`] serves both.
pub(crate) trait CrcSource {
    /// CRC32C of the raw on-disk bytes consumed so far.
    fn crc(&self) -> u32;
}

impl<R: Read> CrcSource for CrcTeeReader<R> {
    fn crc(&self) -> u32 {
        self.hasher
    }
}

impl<R: Read> CrcSource for XzDecoder<CrcTeeReader<R>> {
    fn crc(&self) -> u32 {
        self.get_ref().hasher
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VerifyState {
    /// Still feeding bytes from the underlying reader.
    Reading,
    /// The source reached EOF and the CRC matched. Subsequent reads return `Ok(0)` (normal EOF).
    EofChecked,
    /// A terminal error (CRC mismatch or short range) was reported once. Subsequent reads return
    /// `Ok(0)` so the reader stays well-behaved if the caller re-polls after the error.
    Failed,
}

/// Verifying reader for asset payloads. Forwards decoded bytes from a [`CrcSource`] and, at the
/// source's natural EOF, confirms the stored CRC32C or returns [`ChecksumError::Mismatch`] in place
/// of `Ok(0)`.
///
/// For an uncompressed asset the source is `CrcTeeReader<ExactLen<…>>`, so the bytes read and the
/// bytes hashed are the same payload bytes. For an xz-compressed asset the source is
/// `XzDecoder<CrcTeeReader<ExactLen<…>>>`: the tee accumulates over the raw *compressed* bytes
/// (verification happens before decompression) while the caller reads decompressed output, and the
/// hash is finalized once the decoder reaches its own EOF.
///
/// If the underlying [`ExactLen`] flagged a short range, an error from the source is rewritten to a
/// bundle-layer `UnexpectedEof`/[`ShortRangeMarker`] so the structural truncation is the reported
/// failure rather than a codec-specific error.
pub(crate) struct VerifyingReader<S: Read + CrcSource> {
    source: S,
    expected: u32,
    target: ChecksumTarget,
    short_flag: ShortRangeFlag,
    state: VerifyState,
}

impl<S: Read + CrcSource> VerifyingReader<S> {
    pub(crate) fn new(
        source: S,
        expected: u32,
        target: ChecksumTarget,
        short_flag: ShortRangeFlag,
    ) -> Self {
        Self {
            source,
            expected,
            target,
            short_flag,
            state: VerifyState::Reading,
        }
    }
}

impl<S: Read + CrcSource> Read for VerifyingReader<S> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self.state {
            VerifyState::EofChecked | VerifyState::Failed => return Ok(0),
            VerifyState::Reading => {}
        }
        match self.source.read(buf) {
            Ok(0) => {
                let computed = self.source.crc();
                if computed == self.expected {
                    self.state = VerifyState::EofChecked;
                    Ok(0)
                } else {
                    self.state = VerifyState::Failed;
                    Err(crc_mismatch_error(
                        self.target.clone(),
                        computed,
                        self.expected,
                    ))
                }
            }
            Ok(n) => Ok(n),
            Err(e) => {
                self.state = VerifyState::Failed;
                // A short read from ExactLen already carries the real remaining count; pass it
                // through untouched. Otherwise, if the source (e.g. an xz decoder) swallowed the
                // short read in favor of its own error, the shared flag lets us still surface the
                // structural truncation.
                if e.get_ref()
                    .is_some_and(|inner| inner.is::<ShortRangeMarker>())
                {
                    Err(e)
                } else {
                    Err(override_if_short(&self.short_flag, e))
                }
            }
        }
    }
}

// =====================================================================
// Verified assignment-stream reader
// =====================================================================

/// CRC32C accumulator that shares its running hash via an `Arc<AtomicU32>`. Used as the source
/// reader for [`BendlVerifiedStreamReader`]: the `Arc` lets the outer wrapper read the final hash
/// after a consuming inner method (e.g. `count_samples`) moves ownership away from the wrapper.
///
/// Unlike [`CrcTeeReader`], this type never substitutes a checksum error for raw EOF; it is always
/// the outer [`BendlVerifiedStreamReader`] that decides when and whether to check.
pub(crate) struct SharedCrc32cAccumulatorReader<R: Read> {
    inner: R,
    state: Arc<AtomicU32>,
}

impl<R: Read> Read for SharedCrc32cAccumulatorReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        if n > 0 {
            let prev = self.state.load(Ordering::Relaxed);
            self.state
                .store(crc32c::crc32c_append(prev, &buf[..n]), Ordering::Relaxed);
        }
        Ok(n)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StreamVerifyState {
    Running,
    /// A CRC mismatch was returned once as `Some(Err(...))`. Subsequent iterator calls return
    /// `None`.
    MismatchReported,
    /// A non-CRC terminal error (codec failure, bundle-layer short range, etc.) was returned once
    /// as `Some(Err(...))`. Subsequent iterator calls return `None`. Kept distinct from
    /// `MismatchReported` so the state machine self-documents which class of failure tripped it.
    Errored,
    /// CRC matched after natural EOF. Subsequent iterator calls return `None`.
    Verified,
}

/// Source reader stack underneath a [`BendlVerifiedStreamReader`].
pub(crate) type VerifiedStreamSource<'a, R> = SharedCrc32cAccumulatorReader<ExactLen<&'a mut R>>;

/// Verified decoded assignment reader returned by
/// [`super::reader::BendlReader::open_assignment_reader`].
///
/// Wraps a [`BenStreamReader`] over a CRC-accumulating source and checks the stored stream CRC32C
/// after the codec reaches natural EOF. CRC mismatch surfaces from [`Iterator::next`] as
/// `Some(Err(io::ErrorKind::InvalidData))`, returned once after the last decoded record, then
/// `None`. Consuming methods (`count_samples`, `write_all_jsonl`, `for_each_assignment` when driven
/// to natural EOF) also fold the CRC check into their return value.
pub struct BendlVerifiedStreamReader<'a, R: Read + Seek> {
    inner: BenStreamReader<VerifiedStreamSource<'a, R>>,
    expected: u32,
    arc_hasher: Arc<AtomicU32>,
    short_flag: ShortRangeFlag,
    state: StreamVerifyState,
}

impl<'a, R: Read + Seek> BendlVerifiedStreamReader<'a, R> {
    /// Construct a verified stream reader over a bounded assignment-stream range.
    ///
    /// `raw` is the bounded `ExactLen` over the on-disk stream bytes; `short_flag` is its shared
    /// short-range flag. `init` builds the BEN/XBEN decoder over the CRC-accumulating source. On a
    /// decoder-init failure caused by a truncated range (short flag set), the structural truncation
    /// is surfaced as a bundle-layer `UnexpectedEof` rather than a banner-parse error.
    pub(crate) fn new(
        raw: ExactLen<&'a mut R>,
        short_flag: ShortRangeFlag,
        expected: u32,
        init: impl FnOnce(
            VerifiedStreamSource<'a, R>,
        )
            -> Result<BenStreamReader<VerifiedStreamSource<'a, R>>, BendlReadError>,
    ) -> Result<Self, BendlReadError> {
        let arc_hasher = Arc::new(AtomicU32::new(0));
        let source = SharedCrc32cAccumulatorReader {
            inner: raw,
            state: Arc::clone(&arc_hasher),
        };
        let inner = match init(source) {
            Ok(inner) => inner,
            Err(e) => {
                if short_flag.get() {
                    return Err(BendlReadError::Io(short_range_eof()));
                }
                return Err(e);
            }
        };
        Ok(Self {
            inner,
            expected,
            arc_hasher,
            short_flag,
            state: StreamVerifyState::Running,
        })
    }

    /// Return the BEN variant detected from the stream banner.
    pub fn variant(&self) -> BenVariant {
        self.inner.variant()
    }

    /// Return the wire format (BEN vs XBEN) of this stream.
    pub fn wire_format(&self) -> BenWireFormat {
        self.inner.wire_format()
    }

    /// Suppress progress output from the decoder.
    pub fn silent(mut self, silent: bool) -> Self {
        self.inner = self.inner.silent(silent);
        self
    }

    /// Compare the finalized stream hash against the stored CRC, mapping a mismatch to an
    /// `InvalidData` error. Called by the consuming methods after they have driven the decoder to
    /// raw EOF.
    fn finalize_checksum(&self) -> io::Result<()> {
        check_stream_crc(self.arc_hasher.load(Ordering::Relaxed), self.expected)
    }

    /// Map an error returned by a consuming inner call into the bundle-layer short-range error when
    /// the shared flag fired, otherwise pass it through.
    fn map_terminal_error(&self, e: io::Error) -> io::Error {
        override_if_short(&self.short_flag, e)
    }

    /// Count the number of samples in the stream and verify the stream CRC32C.
    ///
    /// Drives the decoder to raw EOF as a side effect, finalizing the CRC accumulator. If the
    /// count succeeds but the CRC does not match, the CRC mismatch is returned instead of the
    /// count.
    pub fn count_samples(self) -> io::Result<usize> {
        // `count_samples` consumes `self.inner`, so capture the pieces the post-EOF check needs
        // before the move rather than borrowing `self` afterwards.
        let arc = Arc::clone(&self.arc_hasher);
        let expected = self.expected;
        let short_flag = self.short_flag.clone();
        let count = self
            .inner
            .count_samples()
            .map_err(|e| override_if_short(&short_flag, e))?;
        check_stream_crc(arc.load(Ordering::Relaxed), expected)?;
        Ok(count)
    }

    /// Decode assignments and pass each one to a callback by reference.
    ///
    /// When the callback drives the reader to natural EOF, the stream CRC is verified and a
    /// mismatch is returned as an error. When the callback stops early (`f` returns `Ok(false)`),
    /// the CRC is not checked; only a full traversal can verify the whole stream.
    pub fn for_each_assignment<F>(&mut self, mut f: F) -> io::Result<()>
    where
        F: FnMut(&[u16], u16) -> io::Result<bool>,
    {
        loop {
            match self.next() {
                Some(Ok((ref assignment, count))) => {
                    if !f(assignment, count)? {
                        return Ok(());
                    }
                }
                Some(Err(e)) => return Err(e),
                None => return Ok(()),
            }
        }
    }

    /// Decode the remaining stream, write it as JSONL, and verify the stream CRC32C.
    ///
    /// Each decoded sample is written as a JSON object containing an `assignment` vector and a
    /// 1-based `sample` index. After all records are written, the stream CRC is checked; a
    /// mismatch is returned instead of `Ok(())`.
    pub fn write_all_jsonl(&mut self, mut writer: impl Write) -> io::Result<()> {
        let mut sample_number = 0usize;
        self.for_each_assignment(|assignment, count| {
            for _ in 0..count {
                sample_number += 1;
                let line = json!({
                    "assignment": assignment,
                    "sample": sample_number,
                })
                .to_string()
                    + "\n";
                writer.write_all(line.as_bytes())?;
            }
            Ok(true)
        })
    }
}

impl<'a, R: Read + Seek> Iterator for BendlVerifiedStreamReader<'a, R> {
    type Item = io::Result<(Vec<u16>, u16)>;

    fn next(&mut self) -> Option<Self::Item> {
        match self.state {
            StreamVerifyState::MismatchReported
            | StreamVerifyState::Errored
            | StreamVerifyState::Verified => return None,
            StreamVerifyState::Running => {}
        }
        match self.inner.next() {
            Some(Err(e)) => {
                // Non-CRC terminal error: codec failure, bundle-layer short range, or anything else
                // the inner reader returned. CRC mismatch lives in the `None` branch below.
                self.state = StreamVerifyState::Errored;
                Some(Err(self.map_terminal_error(e)))
            }
            Some(item) => Some(item),
            None => {
                // Inner reached natural EOF; finalize the CRC check.
                match self.finalize_checksum() {
                    Ok(()) => {
                        self.state = StreamVerifyState::Verified;
                        None
                    }
                    Err(e) => {
                        self.state = StreamVerifyState::MismatchReported;
                        Some(Err(e))
                    }
                }
            }
        }
    }
}
