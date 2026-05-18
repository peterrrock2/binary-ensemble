//! Tools for working with binary ensembles of districting plans.
//!
//! This crate provides several command line tools and library functions for converting ensembles of
//! districting plans contained in a JSONL file with lines of the form
//!
//! ```text
//! {"assignment": <assignment>, "sample": <sample>}
//! ```
//!
//! into binary ensembles (BEN) and extremely compressed binary ensembles (XBEN). It also provides
//! several tools for working with these files including several tools for relabeling the ensembles
//! to improve compression ratios.
//!
//! The main CLI tools provided by this crate are:
//!
//! - `ben`: A tool for converting JSONL files into BEN files. and for converting between BEN and
//!   XBEN files.
//! - `reben`: A tool for relabeling BEN files to improve compression ratios.

#[cfg(not(target_pointer_width = "64"))]
compile_error!("binary-ensemble requires a 64-bit target");

/// Command-line entrypoints shared by the thin binaries in `src/bin`.
pub mod cli;
/// Encoding, decoding, and format-to-format translation helpers.
pub mod codec;
/// Shared on-disk format metadata such as stream banners.
pub mod format;
/// Streaming readers and writers for BEN and XBEN files.
pub mod io;
/// JSON graph utilities used by relabeling workflows.
pub mod json;
/// Logging helpers used by the CLI and library.
pub mod logging;
/// Higher-level operations such as extraction and relabeling.
pub mod ops;
/// In-place progress spinners for streaming operations.
pub mod progress;
/// Miscellaneous utilities that do not fit into the other modules.
pub mod util;

#[doc(hidden)]
pub mod test_utils;

#[derive(Debug, Clone, Copy, PartialEq)]
/// The BEN/XBEN variant used when encoding or decoding a stream.
pub enum BenVariant {
    /// Store each sample independently.
    Standard,
    /// Store one frame plus a repetition count for repeated consecutive samples.
    MkvChain,
    /// Store delta-encoded frames for improved compression of correlated samples.
    TwoDelta,
}

/// The subset of [`BenVariant`] values that pass through the BEN32 intermediate wire format
/// (see `docs/glossary.md`).
///
/// `TwoDelta` streams use a separate XBEN columnar layout and are intentionally excluded; functions
/// parameterised by `XBenVariant` cannot be called for TwoDelta at compile time. Convert with
/// `From<XBenVariant> for BenVariant` or `TryFrom<BenVariant> for XBenVariant`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum XBenVariant {
    Standard,
    MkvChain,
}

impl From<XBenVariant> for BenVariant {
    fn from(v: XBenVariant) -> Self {
        match v {
            XBenVariant::Standard => BenVariant::Standard,
            XBenVariant::MkvChain => BenVariant::MkvChain,
        }
    }
}

/// Returned by `TryFrom<BenVariant> for XBenVariant` when the input is `TwoDelta`, which has no
/// BEN32 representation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TwoDeltaNotXBenError;

impl std::fmt::Display for TwoDeltaNotXBenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TwoDelta has no BEN32 representation; use the XBEN columnar layout instead")
    }
}

impl std::error::Error for TwoDeltaNotXBenError {}

impl TryFrom<BenVariant> for XBenVariant {
    type Error = TwoDeltaNotXBenError;

    fn try_from(v: BenVariant) -> Result<Self, Self::Error> {
        match v {
            BenVariant::Standard => Ok(XBenVariant::Standard),
            BenVariant::MkvChain => Ok(XBenVariant::MkvChain),
            BenVariant::TwoDelta => Err(TwoDeltaNotXBenError),
        }
    }
}
