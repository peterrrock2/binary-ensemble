//! `.bendl` single-file dataset container.
//!
//! A `.bendl` file is a seekable container that wraps the existing BEN or
//! XBEN assignment stream together with optional front-loaded assets such
//! as a graph JSON, a relabel map, or a metadata blob. The directory table
//! that describes those assets lives at the end of the file so that new
//! assets can be appended to a finalized bundle in O(new asset size +
//! directory size) without rewriting the assignment stream.
//!
//! The module is organised as:
//!
//! - [`format`] — binary header and directory entry types, constants, and
//!   encode/decode helpers. Pure functions over byte buffers; no I/O.
//! - [`manifest`] — serde structs for the optional `metadata.json` asset.

pub mod format;
pub mod manifest;
pub mod reader;
pub mod writer;

#[cfg(test)]
mod tests;

pub use reader::{BendlReader, BundleAssignmentReaderError, BundleValidationError};
pub use writer::{
    AddAssetOptions, BendlStreamHandle, BendlWriteError, BendlWriter, BundleAssignmentStreamCtx,
};
