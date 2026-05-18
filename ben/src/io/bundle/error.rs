//! Read-side error types for `.bendl` bundles.
//!
//! [`BendlReadError`] is the canonical error type for high-level BENDL
//! convenience APIs (anything that returns an owned value: `asset_bytes`,
//! reader constructors that consume internally, etc.). Returned `Read`
//! / iterator / stream-wrapper values keep their native `io::Result`
//! surface; checksum failures on those paths are carried as
//! `io::ErrorKind::InvalidData` with an inner [`ChecksumError`] that
//! callers can downcast.
//!
//! Variant discipline is held at the wrap site, not by the type system:
//! `Io(io::Error)` and `Decode(io::Error)` carry the same payload type,
//! so a future refactor could accidentally wrap a decoder-runtime error
//! as `Io(_)` and the type system would not notice. The error-discipline
//! tests pin which variant fires for each representative read path.

use std::fmt;
use std::io;

use thiserror::Error;

use super::format::BendlFormatError;
use crate::io::reader::DecoderInitError;

/// Identifies which checksummed region a [`ChecksumError`] refers to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChecksumTarget {
    /// A directory-entry asset, identified by name.
    Asset(String),
    /// The assignment stream (only one per bundle).
    Stream,
}

impl fmt::Display for ChecksumTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ChecksumTarget::Asset(name) => write!(f, "asset {name:?}"),
            ChecksumTarget::Stream => write!(f, "assignment stream"),
        }
    }
}

/// Checksum-specific failures from BENDL reader APIs.
///
/// Variant precedence is scoped per checksum domain, not global:
///
/// - **Asset checksum:** `Unavailable` > `Mismatch`. The directory entry
///   is authoritative regardless of bundle finalization, so
///   `verify_asset_checksum` never returns `BundleIncomplete`.
/// - **Stream checksum:** `BundleIncomplete` > `Unavailable` > `Mismatch`.
///   The stream's stored CRC depends on `stream_len` being authoritative,
///   which only holds after finalization, so an unfinalized bundle
///   short-circuits to `BundleIncomplete` before the flag is inspected.
#[derive(Debug, Error)]
pub enum ChecksumError {
    /// The computed CRC32C did not match the stored value.
    #[error(
        "checksum mismatch for {target}: computed 0x{computed:08x}, expected 0x{expected:08x}"
    )]
    Mismatch {
        /// Which region failed verification.
        target: ChecksumTarget,
        /// CRC32C computed by the reader over the on-disk bytes.
        computed: u32,
        /// CRC32C stored in the bundle.
        expected: u32,
    },

    /// The relevant checksum-presence flag (`ASSET_FLAG_CHECKSUM` on a
    /// directory entry, or the stream-level equivalent on the header)
    /// was clear; there is no stored checksum to verify against. The
    /// library writer always sets these flags, so this fires only for
    /// foreign or hand-built bytes.
    #[error("checksum is unavailable for {target}")]
    Unavailable {
        /// Which region lacks a stored checksum.
        target: ChecksumTarget,
    },

    /// The bundle is not finalized, so the stored checksum is not
    /// authoritative. Stream-only: asset-checksum APIs never produce
    /// this variant because directory entries are authoritative
    /// regardless of bundle finalization.
    #[error("bundle is unfinalized; {target} checksum is not authoritative yet")]
    BundleIncomplete {
        /// Which region's checksum is not yet authoritative.
        target: ChecksumTarget,
    },
}

/// High-level error returned by BENDL convenience APIs that consume
/// internally before producing an owned value.
///
/// See [`crate::io::bundle::reader::BendlReader`] for the variant rules
/// per API. The variant discipline is held at the wrap site (where each
/// underlying error becomes a `BendlReadError`); the type system alone
/// cannot prevent a future refactor from mis-wrapping a codec error as
/// `Io` or a header parse failure as `DecoderInit`. The "variant
/// discipline" tests pin which variant fires for each representative
/// read path.
#[derive(Debug, Error)]
pub enum BendlReadError {
    /// Underlying I/O failure at the bundle layer (seek, range read,
    /// filesystem). Never used to carry codec or checksum failures.
    #[error("IO error: {0}")]
    Io(io::Error),

    /// A format-layer error. Reserved for higher-level APIs that wrap
    /// an `open` failure or for future lazy-validation paths; normal
    /// post-open accessors should not produce this from
    /// header/directory structure.
    #[error("bundle format error: {0}")]
    Format(BendlFormatError),

    /// Checksum verification failed, was unavailable, or could not be
    /// authoritatively performed.
    #[error("checksum error: {0}")]
    Checksum(#[from] ChecksumError),

    /// A BEN/XBEN decoder rejected the embedded stream banner.
    #[error("decoder init error: {0}")]
    DecoderInit(DecoderInitError),

    /// A codec error raised while a BEN/XBEN/xz decoder was already
    /// running (malformed compressed payload, malformed assignment
    /// stream, etc.).
    #[error("decode error: {0}")]
    Decode(io::Error),
}

impl From<io::Error> for BendlReadError {
    fn from(e: io::Error) -> Self {
        BendlReadError::Io(e)
    }
}

impl From<BendlFormatError> for BendlReadError {
    fn from(e: BendlFormatError) -> Self {
        // BendlFormatError already carries an `Io` arm; unwrap it so
        // that ordinary I/O failures at the format layer surface as
        // `BendlReadError::Io` rather than getting buried inside a
        // synthetic `Format` wrap.
        match e {
            BendlFormatError::Io(io) => BendlReadError::Io(io),
            other => BendlReadError::Format(other),
        }
    }
}

impl From<DecoderInitError> for BendlReadError {
    fn from(e: DecoderInitError) -> Self {
        BendlReadError::DecoderInit(e)
    }
}
