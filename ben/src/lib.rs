//! Tools for working with binary ensembles of districting plans.
//!
//! This crate provides several command line tools and library functions for
//! converting ensembles of districting plans contained in a JSONL file with
//! lines of the form
//!
//! ```text
//! {"assignment": <assignment>, "sample": <sample>}
//! ```
//!
//! into binary ensembles (BEN) and extremely compressed binary ensembles
//! (XBEN). It also provides several tools for working with these files
//! including several tools for relabeling the ensembles to improve
//! compression ratios.
//!
//! The main CLI tools provided by this crate are:
//!
//! - `ben`: A tool for converting JSONL files into BEN files.
//!    and for converting between BEN and XBEN files.
//! - `reben`: A tool for relabeling BEN files to improve compression ratios.
//!

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
